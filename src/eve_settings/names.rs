//! Character names for the ids in the file names, from ESI's public
//! `universe/names` endpoint, cached in `~/.config/yutani/characters.ron`.
//!
//! `curl` rather than an HTTP crate, like the tunnel worker's exit-IP
//! lookup: one request, no new dependency, and a process timeout around it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::model::write_atomic;
use crate::proc::output_with_timeout;

pub type Names = BTreeMap<u64, String>;

pub const ESI_NAMES_URL: &str = "https://esi.evetech.net/latest/universe/names/";
const USER_AGENT: &str = "User-Agent: yutani (COSMIC EVE companion)";
/// curl's own limit; the process timeout is the backstop above it.
const CURL_MAX_TIME: &str = "8";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(10);
/// EVE character names are at most 37 characters; a reply that says
/// otherwise is not trusted past this into the cache or a label.
pub const MAX_NAME_LEN: usize = 64;

pub fn cache_path(config_dir: &Path) -> PathBuf {
    config_dir.join("yutani").join("characters.ron")
}

/// A missing or unreadable cache is an empty one: it is only a cache.
pub fn load_cache(path: &Path) -> Names {
    std::fs::read_to_string(path).ok().and_then(|text| ron::from_str(&text).ok()).unwrap_or_default()
}

pub fn save_cache(path: &Path, names: &Names) -> std::io::Result<()> {
    let text = ron::ser::to_string_pretty(names, ron::ser::PrettyConfig::default())
        .map_err(|e| std::io::Error::other(e.to_string()))?;
    write_atomic(path, &text)
}

pub fn request_body(ids: &[u64]) -> String {
    serde_json::to_string(ids).unwrap_or_else(|_| "[]".to_string())
}

#[derive(serde::Deserialize)]
struct Named {
    category: String,
    id: u64,
    name: String,
}

/// The characters in an ESI `universe/names` reply. Anything that is not
/// the expected array (ESI's `{"error": …}` objects included) is an error
/// that names ESI, so the note line reads as "ESI said no", not "broken".
pub fn parse_response(json: &str) -> Result<Names, String> {
    let entries: Vec<Named> = serde_json::from_str(json).map_err(|_| {
        let short: String = json.chars().take(120).collect();
        format!("ESI did not return names: {short}")
    })?;
    Ok(entries
        .into_iter()
        .filter(|n| n.category == "character")
        .map(|n| (n.id, n.name.chars().take(MAX_NAME_LEN).collect()))
        .collect())
}

pub fn curl_command(ids: &[u64]) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-m", CURL_MAX_TIME, "-X", "POST"])
        .args(["-H", "Content-Type: application/json", "-H", "Accept: application/json", "-H", USER_AGENT])
        .args(["--data", &request_body(ids)])
        .arg(ESI_NAMES_URL);
    cmd
}

/// Why one ESI request gave no names. The distinction decides whether
/// asking again per id can help: ESI rejects a whole batch when any id
/// in it is unknown (`Rejected`), but a network that could not be
/// reached once will not be reached N more times (`Transport`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// curl could not run, could not connect, or timed out.
    Transport(String),
    /// ESI answered, and the body is not a names array.
    Rejected(String),
}

impl FetchError {
    fn into_message(self) -> String {
        match self {
            FetchError::Transport(m) | FetchError::Rejected(m) => m,
        }
    }
}

fn fetch_once(ids: &[u64]) -> Result<Names, FetchError> {
    let output = output_with_timeout(&mut curl_command(ids), PROCESS_TIMEOUT)
        .map_err(|e| FetchError::Transport(format!("cannot run curl: {e}")))?
        .ok_or_else(|| FetchError::Transport("ESI lookup timed out".to_string()))?;
    if !output.status.success() {
        return Err(FetchError::Transport(format!("curl failed: {}", String::from_utf8_lossy(&output.stderr).trim())));
    }
    parse_response(&String::from_utf8_lossy(&output.stdout)).map_err(FetchError::Rejected)
}

/// One batch; if ESI rejects it (it rejects the whole batch when any id
/// is unknown — a biomassed character, say) each id alone, keeping what
/// resolves. The error, if any is left, is the batch's.
pub fn fetch(ids: &[u64]) -> Result<Names, String> {
    fetch_with(ids, fetch_once)
}

/// [`fetch`] with the request swapped out. The fallback runs only on a
/// `Rejected` batch: a transport failure would be 1 + N timeouts on the
/// blocking pool with the caption stuck on "Looking up character names…",
/// and a transport failure part-way through the fallback stops it for
/// the same reason.
pub fn fetch_with(ids: &[u64], mut once: impl FnMut(&[u64]) -> Result<Names, FetchError>) -> Result<Names, String> {
    if ids.is_empty() {
        return Ok(Names::new());
    }
    match once(ids) {
        Ok(names) => Ok(names),
        Err(FetchError::Rejected(batch_error)) if ids.len() > 1 => {
            let mut names = Names::new();
            for id in ids {
                match once(std::slice::from_ref(id)) {
                    Ok(one) => names.extend(one),
                    Err(FetchError::Rejected(_)) => {}
                    Err(FetchError::Transport(_)) => break,
                }
            }
            if names.is_empty() { Err(batch_error) } else { Ok(names) }
        }
        Err(e) => Err(e.into_message()),
    }
}

/// The names for `ids`: cache first, ESI for the rest, cache updated when
/// anything new arrived. Returns whatever is known plus the fetch error,
/// so an offline machine still shows the cached names. Blocking — run it
/// on the blocking pool.
pub fn resolve(ids: Vec<u64>, cache: PathBuf) -> (Names, Option<String>) {
    resolve_with(ids, cache, fetch)
}

/// [`resolve`] with the ESI call swapped out, so the cache handling can
/// be tested without a network.
pub fn resolve_with(ids: Vec<u64>, cache: PathBuf, fetch: impl FnOnce(&[u64]) -> Result<Names, String>) -> (Names, Option<String>) {
    let mut names = load_cache(&cache);
    let missing: Vec<u64> = ids.iter().copied().filter(|id| !names.contains_key(id)).collect();
    if missing.is_empty() {
        return (names, None);
    }
    match fetch(&missing) {
        Ok(mut fresh) => {
            // Only what was asked for: an id we never sent is not ours to
            // cache, whatever the reply says.
            fresh.retain(|id, _| missing.contains(id));
            if !fresh.is_empty() {
                names.extend(fresh);
                if let Err(e) = save_cache(&cache, &names) {
                    tracing::warn!("cannot write {}: {e}", cache.display());
                }
            }
            (names, None)
        }
        Err(e) => (names, Some(e)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_request_is_a_json_array_of_ids() {
        assert_eq!(request_body(&[90000001, 90000002]), "[90000001,90000002]");
        assert_eq!(request_body(&[]), "[]");
    }

    #[test]
    fn the_response_keeps_characters_only() {
        let json = r#"[{"category":"character","id":90000001,"name":"KestrelVance"},
                       {"category":"corporation","id":98000001,"name":"Some Corp"},
                       {"category":"character","id":90000002,"name":"Sasha-666"}]"#;
        let names = parse_response(json).unwrap();
        assert_eq!(names.len(), 2);
        assert_eq!(names[&90000001], "KestrelVance");
        assert_eq!(names[&90000002], "Sasha-666");
        assert!(parse_response("not json").unwrap_err().contains("ESI"));
        assert!(parse_response(r#"{"error":"Ensure all IDs are valid before resolving."}"#).unwrap_err().contains("ESI"));
    }

    #[test]
    fn the_curl_command_posts_json_to_esi() {
        let cmd = curl_command(&[1, 2]);
        assert_eq!(cmd.get_program(), "curl");
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.contains(&ESI_NAMES_URL.to_string()));
        assert!(args.contains(&"[1,2]".to_string()));
        assert!(args.iter().any(|a| a == "Content-Type: application/json"));
        assert!(args.iter().any(|a| a.starts_with("User-Agent: yutani")));
        assert!(args.windows(2).any(|w| w[0] == "-m" && w[1] == "8"));
        assert!(args.iter().any(|a| a == "-sS"));
    }

    #[test]
    fn the_cache_round_trips_and_tolerates_a_missing_or_broken_file() {
        let dir = std::env::temp_dir().join(format!("yutani-names-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = cache_path(&dir);
        assert_eq!(path, dir.join("yutani").join("characters.ron"));
        assert!(load_cache(&path).is_empty());
        let mut names = Names::new();
        names.insert(90000001, "KestrelVance".to_string());
        save_cache(&path, &names).unwrap();
        assert_eq!(load_cache(&path), names);
        std::fs::write(&path, "(((").unwrap();
        assert!(load_cache(&path).is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The per-id fallback exists for ESI rejecting a batch that has one
    /// unknown id in it. A transport failure (curl could not connect, the
    /// process timed out) is not that: with a black-holed network it would
    /// be 1 + N curls at 8 s each with the caption stuck on "Looking up".
    #[test]
    fn a_transport_failure_is_not_retried_per_id_but_a_rejected_batch_is() {
        let ids = [1, 2, 3];
        let mut calls = Vec::new();
        let err = fetch_with(&ids, |asked: &[u64]| {
            calls.push(asked.to_vec());
            Err(FetchError::Transport("curl failed: could not resolve host".to_string()))
        })
        .unwrap_err();
        assert_eq!(calls, vec![vec![1, 2, 3]], "one batch, no fallback");
        assert!(err.contains("could not resolve host"), "{err}");

        let mut calls = Vec::new();
        let names = fetch_with(&ids, |asked: &[u64]| {
            calls.push(asked.to_vec());
            match asked {
                [2] => Err(FetchError::Rejected("ESI did not return names: {\"error\":…}".to_string())),
                [id] => Ok([(*id, format!("Name {id}"))].into_iter().collect()),
                _ => Err(FetchError::Rejected("ESI did not return names: {\"error\":…}".to_string())),
            }
        })
        .unwrap();
        assert_eq!(calls, vec![vec![1, 2, 3], vec![1], vec![2], vec![3]], "the batch, then each id alone");
        assert_eq!(names.keys().copied().collect::<Vec<_>>(), vec![1, 3]);

        // A fallback that hits a transport failure part-way stops there:
        // the network is gone, the remaining ids would only add timeouts.
        let mut calls = Vec::new();
        let names = fetch_with(&ids, |asked: &[u64]| {
            calls.push(asked.to_vec());
            match asked {
                [1] => Ok([(1, "One".to_string())].into_iter().collect()),
                [2] => Err(FetchError::Transport("ESI lookup timed out".to_string())),
                _ => Err(FetchError::Rejected("rejected".to_string())),
            }
        })
        .unwrap();
        assert_eq!(calls, vec![vec![1, 2, 3], vec![1], vec![2]], "stopped at the timeout");
        assert_eq!(names.keys().copied().collect::<Vec<_>>(), vec![1]);
    }

    /// ESI's reply is trusted only as far as the schema: a name of any
    /// length would go into the cache and the dropdown label as-is. EVE
    /// names are at most 37 characters; anything past 64 is not a name.
    #[test]
    fn a_name_longer_than_any_eve_name_is_cut_short() {
        let long = "x".repeat(200);
        let json = format!(r#"[{{"category":"character","id":7,"name":"{long}"}}]"#);
        let names = parse_response(&json).unwrap();
        assert_eq!(names[&7].chars().count(), MAX_NAME_LEN);
        let exact = "y".repeat(MAX_NAME_LEN);
        let json = format!(r#"[{{"category":"character","id":8,"name":"{exact}"}}]"#);
        assert_eq!(parse_response(&json).unwrap()[&8], exact, "at the limit is kept whole");
    }

    /// Ids we did not ask for must not reach the cache: a reply carrying
    /// extra characters would otherwise plant names for ids the profile
    /// never had — the cache-poisoning surface.
    #[test]
    fn resolve_keeps_only_the_ids_it_asked_for() {
        let dir = std::env::temp_dir().join(format!("yutani-names-extra-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = cache_path(&dir);
        let fetcher = |ids: &[u64]| -> Result<Names, String> {
            assert_eq!(ids, [5, 6]);
            Ok([(5, "Five".to_string()), (6, "Six".to_string()), (99, "Intruder".to_string())].into_iter().collect())
        };
        let (got, err) = resolve_with(vec![5, 6], path.clone(), fetcher);
        assert!(err.is_none());
        assert_eq!(got.keys().copied().collect::<Vec<_>>(), vec![5, 6]);
        assert_eq!(load_cache(&path).keys().copied().collect::<Vec<_>>(), vec![5, 6], "not in the cache either");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Everything cached → no network at all, and the cache is untouched.
    #[test]
    fn resolve_is_offline_when_the_cache_already_has_every_id() {
        let dir = std::env::temp_dir().join(format!("yutani-names-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = cache_path(&dir);
        let mut names = Names::new();
        names.insert(5, "Five".to_string());
        save_cache(&path, &names).unwrap();
        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        let (got, err) = resolve(vec![5], path.clone());
        assert_eq!(got, names);
        assert!(err.is_none());
        assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
