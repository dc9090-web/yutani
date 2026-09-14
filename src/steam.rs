//! Steam's launch options for EVE Online (app 8500): reading them from
//! each account's `localconfig.vdf`, judging whether the `yutani launch`
//! wrapper they name still exists, and carrying the verdict to the panel
//! applet and the settings window. Yutani only ever *reads* Steam's files.
//! See docs/superpowers/specs/2026-09-14-steam-launch-check-design.md.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// EVE Online's Steam app id.
pub const EVE_APP_ID: &str = "8500";

/// EVE's `LaunchOptions` from one account's `localconfig.vdf`.
///
/// The file is Valve's KeyValues text, nested
/// `UserLocalConfigStore → Software → Valve → Steam → apps → "8500" →
/// "LaunchOptions"`. A line walker is enough: an EVE block is any line
/// that is exactly `"8500"` and is followed by `{` (the `apptickets`
/// section also has an `"8500"` key, but as a one-line pair, so it is
/// skipped). There can be more than one such block — Steam also keeps a
/// second `"8500"` block under a top-level `UserLocalConfigStore → apps`
/// section, holding only the overlay setting — so the walk takes the
/// first block that has a direct-child `LaunchOptions`, not the first
/// block, and remembers any block it walks past without one.
///
/// `None`: no EVE block at all. `Some("")`: every EVE block seen had no
/// `LaunchOptions` key — Steam stores no key for an empty field, so that
/// is what "never set" looks like.
pub fn launch_options(localconfig: &str) -> Option<String> {
    let key = format!("\"{EVE_APP_ID}\"");
    let mut lines = localconfig.lines().map(str::trim).peekable();
    let mut empty_block_seen = false;
    while let Some(line) = lines.next() {
        if line != key {
            continue;
        }
        while lines.peek().is_some_and(|l| l.is_empty()) {
            lines.next();
        }
        if lines.peek() != Some(&"{") {
            continue;
        }
        lines.next();
        let mut depth = 1usize;
        let mut closed = false;
        for line in lines.by_ref() {
            match line {
                "{" => depth += 1,
                "}" => {
                    depth -= 1;
                    if depth == 0 {
                        empty_block_seen = true;
                        closed = true;
                        break;
                    }
                }
                _ => {
                    if depth == 1
                        && let Some(value) = quoted_pair(line, "LaunchOptions")
                    {
                        return Some(value);
                    }
                }
            }
        }
        if !closed {
            // The block never closed: a truncated file. Nothing to judge.
            return None;
        }
    }
    if empty_block_seen { Some(String::new()) } else { None }
}

/// `"Key"   "value"` → the value, when the key is `key`.
fn quoted_pair(line: &str, key: &str) -> Option<String> {
    let (k, rest) = quoted(line)?;
    if !k.eq_ignore_ascii_case(key) {
        return None;
    }
    let (v, _) = quoted(rest.trim_start())?;
    Some(v)
}

/// The quoted string at the start of `s`, with VDF's `\"`, `\\`, `\n`
/// and `\t` undone, and whatever follows the closing quote.
fn quoted(s: &str) -> Option<(String, &str)> {
    let body = s.strip_prefix('"')?;
    let mut chars = body.char_indices();
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => {
                let (_, e) = chars.next()?;
                out.push(match e {
                    'n' => '\n',
                    't' => '\t',
                    other => other,
                });
            }
            '"' => return Some((out, &body[i + 1..])),
            c => out.push(c),
        }
    }
    None
}

/// The `yutani` the launch line runs the game through: the token right
/// before `launch --`, when it is `yutani` or a path ending in `/yutani`.
/// Tokens are split on whitespace, and a token wrapped in double quotes
/// is unquoted before the check; a path containing spaces is still
/// unsupported, and the README never suggests one.
pub fn wrapper_path(launch_options: &str) -> Option<&str> {
    let tokens: Vec<&str> = launch_options.split_whitespace().collect();
    tokens.windows(3).find_map(|w| {
        let p = unquote(w[0]);
        // A leftover `"` means the quotes didn't balance: `unquote`
        // handed the token back unchanged, and a bare suffix check
        // would ignore the stray quote wherever it landed.
        (w[1] == "launch" && w[2] == "--" && !p.contains('"') && (p == "yutani" || p.ends_with("/yutani"))).then_some(p)
    })
}

/// Strip a matched pair of double quotes from `t`; `t` unchanged when it
/// isn't quoted at all, or when the quotes don't match (an unbalanced
/// quote is left alone rather than silently mangled).
fn unquote(t: &str) -> &str {
    t.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(t)
}

/// What the launch line means for the next press of Play.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "problem", rename_all = "snake_case")]
pub enum Verdict {
    /// `yutani launch --` is there and the binary it names resolves.
    Ok,
    /// The line names a yutani that does not resolve: an absolute path
    /// that does not exist, or a bare `yutani` not on PATH. Play fails in
    /// `/bin/sh` before Proton runs, with nothing on screen.
    Broken { path: String },
    /// No `yutani launch --` at all: EVE starts outside the tunnel.
    NoWrapper,
}

impl Verdict {
    /// The sentence the popover and the settings banner share; `None`
    /// when there is nothing to say.
    pub fn message(&self) -> Option<String> {
        match self {
            Verdict::Ok => None,
            Verdict::Broken { path } => Some(format!("Steam launches EVE through {path}, which is missing.")),
            Verdict::NoWrapper => Some("Steam launches EVE without yutani, so it runs outside the tunnel.".to_string()),
        }
    }
}

/// Judge one launch line. `resolves` says whether a wrapper token names a
/// real binary; it is injected so every verdict is testable without a
/// filesystem (the production one is [`resolves`]).
pub fn judge(launch_options: &str, resolves: &dyn Fn(&str) -> bool) -> Verdict {
    match wrapper_path(launch_options) {
        None => Verdict::NoWrapper,
        Some(p) if resolves(p) => Verdict::Ok,
        Some(p) => Verdict::Broken { path: p.to_string() },
    }
}

/// One account's verdict and the file it came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub verdict: Verdict,
    /// The `localconfig.vdf` the launch line was read from.
    pub file: PathBuf,
}

/// Where Steam keeps `userdata/`: native (the XDG path and the legacy
/// `~/.steam` symlink tree) and the Flatpak. Mirrors
/// `eve_settings::steam_vdf_candidates`, one level up.
pub fn steam_roots(home: &Path) -> [PathBuf; 3] {
    [
        home.join(".local/share/Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ]
}

/// Every account's verdict, in root then account-id order. A root whose
/// `userdata` canonicalises to one already seen (the legacy symlink) is
/// skipped, so no account is reported twice; an unreadable file or an
/// account with no EVE block is silently skipped — a missing Steam is not
/// Yutani's problem to report.
pub fn scan(home: &Path, resolves: &dyn Fn(&str) -> bool) -> Vec<Finding> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for root in steam_roots(home) {
        let Ok(userdata) = root.join("userdata").canonicalize() else { continue };
        if seen.contains(&userdata) {
            continue;
        }
        seen.push(userdata.clone());
        let Ok(entries) = std::fs::read_dir(&userdata) else { continue };
        let mut accounts: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            })
            .collect();
        accounts.sort();
        for account in accounts {
            let file = account.join("config").join("localconfig.vdf");
            let Ok(text) = std::fs::read_to_string(&file) else { continue };
            let Some(line) = launch_options(&text) else { continue };
            out.push(Finding { verdict: judge(&line, resolves), file });
        }
    }
    out
}

/// The production resolver: a token with a `/` is a path that must be a
/// file; a bare name is searched on this process's `PATH`.
pub fn resolves(name: &str) -> bool {
    if name.contains('/') {
        return Path::new(name).is_file();
    }
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|dir| dir.join(name).is_file()))
        .unwrap_or(false)
}

/// What the daemon reports: only the accounts whose launch line is not
/// [`Verdict::Ok`], from the real home and the real `PATH`.
pub fn problems() -> Vec<Finding> {
    let Some(home) = dirs::home_dir() else { return Vec::new() };
    scan(&home, &resolves).into_iter().filter(|f| f.verdict != Verdict::Ok).collect()
}

/// The first thing worth saying about `findings`, for the popover and the
/// settings banner.
pub fn first_message(findings: &[Finding]) -> Option<String> {
    findings.iter().find_map(|f| f.verdict.message())
}

use cosmic::iced::{self, Subscription};
use std::time::Duration;

/// How often the daemon re-reads Steam's files. Steam rewrites
/// `localconfig.vdf` when Properties closes and when it exits, so a fix
/// made in Steam's UI shows up within this.
pub const PERIOD: Duration = Duration::from_secs(30);

/// The daemon's watch: [`problems`] at startup and every [`PERIOD`],
/// emitted only when the list changed. The scan reads a few small files
/// on the blocking pool; the subscription's future shares the one-worker
/// UI runtime and must not block it.
pub fn subscription() -> Subscription<Vec<Finding>> {
    Subscription::run(run)
}

fn run() -> impl iced::futures::Stream<Item = Vec<Finding>> {
    iced::futures::stream::unfold(None::<Vec<Finding>>, |mut last| async move {
        loop {
            if last.is_some() {
                futures_timer::Delay::new(PERIOD).await;
            }
            let now = tokio::task::spawn_blocking(problems).await.unwrap_or_default();
            if last.as_ref() != Some(&now) {
                return Some((now.clone(), Some(now)));
            }
            last = Some(now);
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cut from a real `localconfig.vdf`: the `apptickets` decoy (a
    /// one-line `"8500"` pair), then the real block under `apps`, then a
    /// neighbour.
    const REAL: &str = "\"UserLocalConfigStore\"\n{\n\t\"apptickets\"\n\t{\n\t\t\"8500\"\t\t\"1\"\n\t}\n\t\"Software\"\n\t{\n\t\t\"Valve\"\n\t\t{\n\t\t\t\"Steam\"\n\t\t\t{\n\t\t\t\t\"apps\"\n\t\t\t\t{\n\t\t\t\t\t\"8500\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LastPlayed\"\t\t\"1789355329\"\n\t\t\t\t\t\t\"cloud\"\n\t\t\t\t\t\t{\n\t\t\t\t\t\t\t\"last_sync_state\"\t\t\"synchronized\"\n\t\t\t\t\t\t}\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%\"\n\t\t\t\t\t}\n\t\t\t\t\t\"8870\"\n\t\t\t\t\t{\n\t\t\t\t\t\t\"LaunchOptions\"\t\t\"-novid\"\n\t\t\t\t\t}\n\t\t\t\t}\n\t\t\t}\n\t\t}\n\t}\n}\n";

    #[test]
    fn the_eve_launch_line_is_read_past_the_apptickets_decoy() {
        assert_eq!(
            launch_options(REAL).as_deref(),
            Some("PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%")
        );
        assert_eq!(
            launch_options(&REAL.replace("\"LaunchOptions\"", "\"launchoptions\"")).as_deref(),
            Some("PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%")
        );
    }

    #[test]
    fn a_neighbouring_app_is_never_mistaken_for_eve() {
        let only_neighbour = REAL.replace("\"8500\"\n", "\"8501\"\n");
        assert_eq!(launch_options(&only_neighbour), None);
    }

    #[test]
    fn a_second_eve_block_before_the_real_one_is_walked_past() {
        // A real localconfig.vdf has a second "8500" block under a
        // top-level UserLocalConfigStore -> apps section: the overlay
        // setting, not EVE's launch options.
        let decoy = "\"apps\"\n{\n\t\"8500\"\n\t{\n\t\t\"OverlayAppEnable\"\t\t\"0\"\n\t}\n}\n";
        let with_decoy = format!("{decoy}{REAL}");
        assert_eq!(
            launch_options(&with_decoy).as_deref(),
            Some("PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%")
        );
    }

    #[test]
    fn a_truncated_block_after_a_decoy_is_still_nothing_to_judge() {
        let decoy = "\"apps\"\n{\n\t\"8500\"\n\t{\n\t\t\"OverlayAppEnable\"\t\t\"0\"\n\t}\n}\n";
        assert_eq!(launch_options(&format!("{decoy}\"8500\"\n{{\n\t\"LastPlayed\"\t\t\"1\"\n")), None);
    }

    #[test]
    fn a_second_eve_block_after_the_real_one_changes_nothing() {
        let decoy = "\"apps\"\n{\n\t\"8500\"\n\t{\n\t\t\"OverlayAppEnable\"\t\t\"0\"\n\t}\n}\n";
        assert_eq!(
            launch_options(&format!("{REAL}{decoy}")).as_deref(),
            Some("PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%")
        );
    }

    #[test]
    fn a_lone_overlay_block_is_an_empty_line() {
        let decoy = "\"apps\"\n{\n\t\"8500\"\n\t{\n\t\t\"OverlayAppEnable\"\t\t\"0\"\n\t}\n}\n";
        assert_eq!(launch_options(decoy).as_deref(), Some(""));
    }

    #[test]
    fn an_eve_block_without_launch_options_is_an_empty_line() {
        let stripped = REAL.replace(
            "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%\"\n",
            "",
        );
        assert_eq!(launch_options(&stripped).as_deref(), Some(""));
    }

    #[test]
    fn a_nested_launch_options_key_does_not_count() {
        // Only a direct child of the EVE block is EVE's launch line.
        let nested = REAL.replace("\"last_sync_state\"\t\t\"synchronized\"", "\"LaunchOptions\"\t\t\"nested\"").replace(
            "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%\"\n",
            "",
        );
        assert_eq!(launch_options(&nested).as_deref(), Some(""));
    }

    #[test]
    fn vdf_escapes_are_undone() {
        let escaped = REAL.replace("/usr/local/bin/yutani launch", "\\\"/opt/y\\\\utani\\\" launch");
        // `\"` → `"`, `\\` → `\`.
        assert_eq!(
            launch_options(&escaped).as_deref(),
            Some("PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 \"/opt/y\\utani\" launch -- %command%")
        );
    }

    #[test]
    fn an_escaped_quote_is_kept_and_the_value_ends_at_the_closing_quote() {
        let trailing = REAL.replace(
            "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 /usr/local/bin/yutani launch -- %command%\"\n",
            "\t\t\t\t\t\t\"LaunchOptions\"\t\t\"a \\\"b\\\" c\"\t\"trailing\"\n",
        );
        assert_eq!(launch_options(&trailing).as_deref(), Some("a \"b\" c"));
    }

    #[test]
    fn garbage_is_none() {
        assert_eq!(launch_options(""), None);
        assert_eq!(launch_options("\"8500\"\n{\n\"LaunchOptions\""), None);
        assert_eq!(launch_options("\"8500\"\n\"LaunchOptions\"\t\"x\"\n"), None);
    }

    #[test]
    fn the_wrapper_is_the_token_before_launch() {
        assert_eq!(wrapper_path("A=1 yutani launch -- %command%"), Some("yutani"));
        assert_eq!(wrapper_path("A=1 /usr/bin/yutani launch -- %command%"), Some("/usr/bin/yutani"));
        assert_eq!(wrapper_path("/home/d/Yutani/target/release/yutani launch -- %command%"), Some("/home/d/Yutani/target/release/yutani"));
        assert_eq!(wrapper_path("A=1 \"/opt/yutani\" launch -- %command%"), Some("/opt/yutani"));
        assert_eq!(wrapper_path("A=1 %command%"), None);
        assert_eq!(wrapper_path(""), None);
        // `launch` alone, or a different wrapper, is not ours.
        assert_eq!(wrapper_path("gamemoderun launch -- %command%"), None);
        assert_eq!(wrapper_path("yutani launch %command%"), None);
        // A path with a space is unsupported and must not yield a garbled token.
        assert_eq!(wrapper_path("A=1 \"/opt/my games/yutani\" launch -- %command%"), None);
        // Unbalanced quote.
        assert_eq!(wrapper_path("A=1 \"/opt/yutani launch -- %command%"), None);
    }

    #[test]
    fn every_verdict() {
        let exists = |p: &str| p == "/usr/bin/yutani" || p == "yutani";
        assert_eq!(judge("A=1 yutani launch -- %command%", &exists), Verdict::Ok);
        assert_eq!(judge("A=1 /usr/bin/yutani launch -- %command%", &exists), Verdict::Ok);
        assert_eq!(
            judge("A=1 /usr/local/bin/yutani launch -- %command%", &exists),
            Verdict::Broken { path: "/usr/local/bin/yutani".into() }
        );
        assert_eq!(judge("A=1 %command%", &exists), Verdict::NoWrapper);
        assert_eq!(judge("", &exists), Verdict::NoWrapper);
    }

    #[test]
    fn the_messages_name_the_path_and_ok_says_nothing() {
        assert_eq!(Verdict::Ok.message(), None);
        assert_eq!(
            Verdict::Broken { path: "/usr/local/bin/yutani".into() }.message().as_deref(),
            Some("Steam launches EVE through /usr/local/bin/yutani, which is missing.")
        );
        assert_eq!(
            Verdict::NoWrapper.message().as_deref(),
            Some("Steam launches EVE without yutani, so it runs outside the tunnel.")
        );
    }

    #[test]
    fn a_verdict_serialises_with_a_problem_tag() {
        let json = serde_json::to_string(&Verdict::Broken { path: "/x/yutani".into() }).unwrap();
        assert_eq!(json, r#"{"problem":"broken","path":"/x/yutani"}"#);
        assert_eq!(serde_json::to_string(&Verdict::NoWrapper).unwrap(), r#"{"problem":"no_wrapper"}"#);
        assert_eq!(serde_json::from_str::<Verdict>(r#"{"problem":"ok"}"#).unwrap(), Verdict::Ok);
    }

    fn sandbox(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-steam-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_account(root: &std::path::Path, id: &str, launch: Option<&str>) -> std::path::PathBuf {
        let dir = root.join("userdata").join(id).join("config");
        std::fs::create_dir_all(&dir).unwrap();
        let text = match launch {
            Some(line) => REAL.replace("/usr/local/bin/yutani launch -- %command%", line).replace(
                "PROTON_ENABLE_WAYLAND=1 WINE_NO_WM_DECORATION=1 ",
                "",
            ),
            None => REAL.replace("\"8500\"\n", "\"8501\"\n"),
        };
        let file = dir.join("localconfig.vdf");
        std::fs::write(&file, text).unwrap();
        file
    }

    #[test]
    fn the_roots_cover_native_legacy_and_flatpak_steam() {
        let home = std::path::Path::new("/home/x");
        assert_eq!(
            steam_roots(home),
            [
                std::path::PathBuf::from("/home/x/.local/share/Steam"),
                std::path::PathBuf::from("/home/x/.steam/steam"),
                std::path::PathBuf::from("/home/x/.var/app/com.valvesoftware.Steam/.local/share/Steam"),
            ]
        );
    }

    #[test]
    fn scan_reports_one_finding_per_account_with_an_eve_block() {
        let home = sandbox("scan");
        let native = home.join(".local/share/Steam");
        let fine = write_account(&native, "1001", Some("/usr/bin/yutani launch -- %command%"));
        let broken = write_account(&native, "1002", Some("/usr/local/bin/yutani launch -- %command%"));
        let _no_eve = write_account(&native, "1003", None);
        // A non-numeric entry and an account with no config are skipped.
        std::fs::create_dir_all(native.join("userdata/anonymous")).unwrap();
        std::fs::create_dir_all(native.join("userdata/1004")).unwrap();
        // The legacy tree is a symlink to the native one: not a second copy.
        std::fs::create_dir_all(home.join(".steam")).unwrap();
        std::os::unix::fs::symlink(&native, home.join(".steam/steam")).unwrap();

        let exists = |p: &str| p == "/usr/bin/yutani";
        let found = scan(&home, &exists);
        // `scan` walks the canonical root, so compare canonical paths: the
        // temp dir itself may sit behind a symlink.
        let seen: Vec<(Verdict, std::path::PathBuf)> =
            found.iter().map(|f| (f.verdict.clone(), f.file.canonicalize().unwrap())).collect();
        assert_eq!(
            seen,
            vec![
                (Verdict::Ok, fine.canonicalize().unwrap()),
                (Verdict::Broken { path: "/usr/local/bin/yutani".into() }, broken.canonicalize().unwrap()),
            ]
        );
        assert_eq!(
            first_message(&found).as_deref(),
            Some("Steam launches EVE through /usr/local/bin/yutani, which is missing.")
        );
        assert_eq!(first_message(&found[..1]), None);
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn no_steam_at_all_is_nothing_to_report() {
        let home = sandbox("nosteam");
        assert!(scan(&home, &|_| true).is_empty());
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn resolves_checks_paths_on_disk_and_bare_names_on_path() {
        assert!(resolves("/bin/sh"));
        assert!(!resolves("/nonexistent/yutani"));
        assert!(resolves("sh"));
        assert!(!resolves("yutani-surely-not-installed-under-this-name"));
    }

    #[test]
    fn a_finding_round_trips_as_json() {
        let f = Finding { verdict: Verdict::NoWrapper, file: "/h/localconfig.vdf".into() };
        let json = serde_json::to_string(&f).unwrap();
        assert_eq!(json, r#"{"verdict":{"problem":"no_wrapper"},"file":"/h/localconfig.vdf"}"#);
        assert_eq!(serde_json::from_str::<Finding>(&json).unwrap(), f);
    }
}
