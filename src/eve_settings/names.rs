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
    Ok(entries.into_iter().filter(|n| n.category == "character").map(|n| (n.id, n.name)).collect())
}

pub fn curl_command(ids: &[u64]) -> Command {
    let mut cmd = Command::new("curl");
    cmd.args(["-sS", "-m", CURL_MAX_TIME, "-X", "POST"])
        .args(["-H", "Content-Type: application/json", "-H", "Accept: application/json", "-H", USER_AGENT])
        .args(["--data", &request_body(ids)])
        .arg(ESI_NAMES_URL);
    cmd
}

fn fetch_once(ids: &[u64]) -> Result<Names, String> {
    let output = output_with_timeout(&mut curl_command(ids), PROCESS_TIMEOUT)
        .map_err(|e| format!("cannot run curl: {e}"))?
        .ok_or_else(|| "ESI lookup timed out".to_string())?;
    if !output.status.success() {
        return Err(format!("curl failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }
    parse_response(&String::from_utf8_lossy(&output.stdout))
}

/// One batch; if that fails (ESI rejects the whole batch when any id is
/// unknown — a biomassed character, say) each id alone, keeping what
/// resolves. The error, if any is left, is the batch's.
pub fn fetch(ids: &[u64]) -> Result<Names, String> {
    if ids.is_empty() {
        return Ok(Names::new());
    }
    match fetch_once(ids) {
        Ok(names) => Ok(names),
        Err(batch_error) if ids.len() > 1 => {
            let mut names = Names::new();
            for id in ids {
                if let Ok(one) = fetch_once(std::slice::from_ref(id)) {
                    names.extend(one);
                }
            }
            if names.is_empty() { Err(batch_error) } else { Ok(names) }
        }
        Err(e) => Err(e),
    }
}

/// The names for `ids`: cache first, ESI for the rest, cache updated when
/// anything new arrived. Returns whatever is known plus the fetch error,
/// so an offline machine still shows the cached names. Blocking — run it
/// on the blocking pool.
pub fn resolve(ids: Vec<u64>, cache: PathBuf) -> (Names, Option<String>) {
    let mut names = load_cache(&cache);
    let missing: Vec<u64> = ids.iter().copied().filter(|id| !names.contains_key(id)).collect();
    if missing.is_empty() {
        return (names, None);
    }
    match fetch(&missing) {
        Ok(fresh) => {
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
