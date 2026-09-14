//! IPC wire protocol between `yutani` (the app, server) and `yutani <cmd>`
//! (the CLI, client). Newline-delimited text, one request per connection.

use std::io;
use std::path::{Path, PathBuf};

/// Longest request line the server will read (bytes, including `\n`).
pub const MAX_LINE: usize = 1024;

/// Longest reply line the client will read (bytes, including `\n`). Replies
/// carry JSON (e.g. `status`, which lists every EVE client) and can easily
/// exceed a request-sized buffer, so this is much larger than `MAX_LINE`,
/// which bounds only the request line the server reads.
pub const MAX_REPLY: usize = 64 * 1024;

/// `$XDG_RUNTIME_DIR/yutani.sock`, or `/tmp/yutani-<uid>/yutani.sock`
/// without the variable (a bare TTY session). `/tmp` is world-writable, so
/// the fallback directory is only used once [`private_dir`] has created it
/// 0700 or verified that what is already there is ours and private; `Err`
/// means it is not, and the socket must not be used.
pub fn socket_path() -> io::Result<PathBuf> {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(dir) if !dir.is_empty() => Ok(PathBuf::from(dir).join("yutani.sock")),
        _ => {
            let uid = uid();
            let dir = PathBuf::from(format!("/tmp/yutani-{uid}"));
            private_dir(&dir, uid)?;
            Ok(dir.join("yutani.sock"))
        }
    }
}

/// Create `dir` as a private directory (mode 0700), or accept an existing
/// one only if it is a real directory (not a symlink) owned by `uid` with
/// exactly that mode. In a world-writable parent another local user could
/// have put something at that path first: a directory they own would let
/// them pre-create the socket (the server's `bind` fails) or, while nothing
/// is listening, answer the CLI and applet themselves with forged replies.
pub fn private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        // `mode` is subject to the umask; make sure of it.
        Ok(()) => std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?,
        Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
        Err(err) => return Err(err),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    let shown = dir.display();
    if !meta.is_dir() {
        return Err(io::Error::other(format!("{shown} exists but is not a directory")));
    }
    if meta.uid() != uid {
        return Err(io::Error::other(format!("{shown} is owned by uid {}, not {uid}", meta.uid())));
    }
    if meta.mode() & 0o777 != 0o700 {
        return Err(io::Error::other(format!("{shown} is not private (mode {:o})", meta.mode() & 0o777)));
    }
    Ok(())
}

pub fn uid() -> u32 {
    // SAFETY: getuid has no preconditions and cannot fail.
    unsafe { libc_getuid() }
}
unsafe extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// 1-based index in layout order.
    Focus(usize),
    Next,
    Prev,
    Show,
    Hide,
    Toggle,
    Layout(String),
    Layouts,
    Settings,
    /// Open the settings window on a named page (`settings layouts`).
    SettingsPage(String),
    Quit,
    Status,
    TunnelConnect,
    TunnelDisconnect,
}

impl Request {
    /// `Request::Layout`, refusing a name that could not survive the wire:
    /// the server reads one line, so a name with a line break in it would
    /// silently become a request for whatever precedes the break.
    pub fn layout(name: String) -> Result<Request, String> {
        if name.contains(['\n', '\r']) {
            return Err("layout names cannot contain a line break".into());
        }
        Ok(Request::Layout(name))
    }

    /// Parse one request line (trailing newline optional). Errors are the
    /// text the server sends back after `err `.
    pub fn parse(line: &str) -> Result<Request, String> {
        let line = line.trim_end_matches(['\n', '\r']);
        let mut words = line.splitn(2, ' ');
        let cmd = words.next().unwrap_or("");
        let arg = words.next().map(str::trim).filter(|a| !a.is_empty());
        match (cmd, arg) {
            ("focus", Some(n)) => match n.parse::<usize>() {
                Ok(n) if n >= 1 => Ok(Request::Focus(n)),
                _ => Err(format!("focus needs a client number from 1, got {n:?}")),
            },
            ("focus", None) => Err("focus needs a client number".into()),
            ("next", None) => Ok(Request::Next),
            ("prev", None) => Ok(Request::Prev),
            ("show", None) => Ok(Request::Show),
            ("hide", None) => Ok(Request::Hide),
            ("toggle", None) => Ok(Request::Toggle),
            ("layout", Some(name)) => Ok(Request::Layout(name.to_string())),
            ("layout", None) => Err("layout needs a name".into()),
            ("layouts", None) => Ok(Request::Layouts),
            ("settings", None) => Ok(Request::Settings),
            ("settings", Some(page)) => Ok(Request::SettingsPage(page.to_string())),
            ("quit", None) => Ok(Request::Quit),
            ("status", None) => Ok(Request::Status),
            ("tunnel", Some("connect")) => Ok(Request::TunnelConnect),
            ("tunnel", Some("disconnect")) => Ok(Request::TunnelDisconnect),
            ("tunnel", _) => Err("tunnel needs connect or disconnect".into()),
            ("", _) => Err("empty request".into()),
            (cmd, Some(_))
                if matches!(cmd, "next" | "prev" | "show" | "hide" | "toggle" | "layouts" | "quit" | "status") =>
            {
                Err(format!("{cmd} takes no argument"))
            }
            (cmd, _) => Err(format!("unknown command {cmd:?}")),
        }
    }

    pub fn to_line(&self) -> String {
        match self {
            Request::Focus(n) => format!("focus {n}\n"),
            Request::Next => "next\n".into(),
            Request::Prev => "prev\n".into(),
            Request::Show => "show\n".into(),
            Request::Hide => "hide\n".into(),
            Request::Toggle => "toggle\n".into(),
            Request::Layout(name) => format!("layout {name}\n"),
            Request::Layouts => "layouts\n".into(),
            Request::Settings => "settings\n".into(),
            Request::SettingsPage(page) => format!("settings {page}\n"),
            Request::Quit => "quit\n".into(),
            Request::Status => "status\n".into(),
            Request::TunnelConnect => "tunnel connect\n".into(),
            Request::TunnelDisconnect => "tunnel disconnect\n".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Response {
    Ok,
    OkData(String),
    Err(String),
}

impl Response {
    pub fn parse(line: &str) -> Response {
        let line = line.trim_end_matches(['\n', '\r']);
        match line.strip_prefix("err") {
            Some(rest) if line == "err" || rest.starts_with(' ') => Response::Err(rest.trim_start().to_string()),
            _ if line == "ok" => Response::Ok,
            _ => match line.strip_prefix("ok ") {
                Some(rest) => Response::OkData(rest.to_string()),
                None => Response::Err(format!("malformed reply {line:?}")),
            },
        }
    }

    pub fn to_line(&self) -> String {
        match self {
            Response::Ok => "ok\n".into(),
            Response::OkData(data) => format!("ok {}\n", data.replace('\n', " ")),
            Response::Err(msg) => format!("err {}\n", msg.replace('\n', " ")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_every_command() {
        assert_eq!(Request::parse("focus 3\n"), Ok(Request::Focus(3)));
        assert_eq!(Request::parse("next"), Ok(Request::Next));
        assert_eq!(Request::parse("prev\r\n"), Ok(Request::Prev));
        assert_eq!(Request::parse("show"), Ok(Request::Show));
        assert_eq!(Request::parse("hide"), Ok(Request::Hide));
        assert_eq!(Request::parse("toggle"), Ok(Request::Toggle));
        assert_eq!(Request::parse("layout pvp fleet"), Ok(Request::Layout("pvp fleet".into())));
        assert_eq!(Request::parse("settings"), Ok(Request::Settings));
        assert_eq!(Request::parse("settings layouts"), Ok(Request::SettingsPage("layouts".into())));
        assert_eq!(Request::parse("quit"), Ok(Request::Quit));
        assert_eq!(Request::parse("status"), Ok(Request::Status));
        assert_eq!(Request::parse("tunnel connect"), Ok(Request::TunnelConnect));
        assert_eq!(Request::parse("tunnel disconnect"), Ok(Request::TunnelDisconnect));
    }

    #[test]
    fn rejects_bad_requests_with_a_reason() {
        assert!(Request::parse("focus 0").unwrap_err().contains("from 1"));
        assert!(Request::parse("focus x").unwrap_err().contains("client number"));
        assert!(Request::parse("focus").unwrap_err().contains("client number"));
        assert_eq!(Request::parse("layout").unwrap_err(), "layout needs a name");
        assert_eq!(Request::parse("next now").unwrap_err(), "next takes no argument");
        assert_eq!(Request::parse("dance").unwrap_err(), "unknown command \"dance\"");
        assert_eq!(Request::parse("").unwrap_err(), "empty request");
        assert_eq!(Request::parse("tunnel").unwrap_err(), "tunnel needs connect or disconnect");
        assert_eq!(Request::parse("tunnel up").unwrap_err(), "tunnel needs connect or disconnect");
    }

    #[test]
    fn layouts_is_its_own_command_not_a_layout_named_s() {
        assert_eq!(Request::parse("layouts"), Ok(Request::Layouts));
        assert_eq!(Request::parse("layouts now").unwrap_err(), "layouts takes no argument");
        assert_eq!(Request::parse("layout current"), Ok(Request::Layout("current".into())));
        assert_eq!(Request::Layouts.to_line(), "layouts\n");
    }

    #[test]
    fn request_lines_round_trip() {
        for r in [
            Request::Focus(9),
            Request::Next,
            Request::Prev,
            Request::Show,
            Request::Hide,
            Request::Toggle,
            Request::Layout("a b".into()),
            Request::Layouts,
            Request::Settings,
            Request::SettingsPage("layouts".into()),
            Request::Quit,
            Request::Status,
            Request::TunnelConnect,
            Request::TunnelDisconnect,
        ] {
            let line = r.to_line();
            assert!(line.ends_with('\n'));
            assert_eq!(Request::parse(&line), Ok(r));
        }
    }

    #[test]
    fn responses_round_trip_and_tolerate_garbage() {
        assert_eq!(Response::parse("ok\n"), Response::Ok);
        assert_eq!(Response::parse("err no client 4"), Response::Err("no client 4".into()));
        assert_eq!(Response::parse("err"), Response::Err(String::new()));
        assert_eq!(Response::Err("a\nb".into()).to_line(), "err a b\n");
        assert!(matches!(Response::parse("banana"), Response::Err(m) if m.contains("malformed")));
        assert!(matches!(Response::parse("error x"), Response::Err(m) if m.contains("malformed")));
        assert_eq!(Response::parse("ok {\"a\":1}\n"), Response::OkData("{\"a\":1}".into()));
        assert_eq!(Response::OkData("x".into()).to_line(), "ok x\n");
    }

    #[test]
    fn socket_path_follows_runtime_dir() {
        // Only the shape is asserted; the env var is process-global, so it
        // is not mutated here.
        let p = socket_path().unwrap();
        assert!(p.to_string_lossy().ends_with(".sock"));
        assert!(p.is_absolute());
    }

    #[test]
    fn a_layout_name_with_a_line_break_is_refused_before_it_reaches_the_wire() {
        assert!(Request::layout("pvp\nfleet".into()).is_err(), "would be sent as `layout pvp`");
        assert!(Request::layout("pvp\rfleet".into()).is_err());
        assert_eq!(Request::layout("pvp fleet".into()), Ok(Request::Layout("pvp fleet".into())));
    }

    /// A fresh path under the test temp dir, removed when dropped.
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(tag: &str) -> Self {
            let p = std::env::temp_dir().join(format!("yutani-ipc-test-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&p);
            let _ = std::fs::remove_file(&p);
            Scratch(p)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn mode_of(p: &Path) -> u32 {
        use std::os::unix::fs::MetadataExt;
        std::fs::symlink_metadata(p).unwrap().mode() & 0o777
    }

    #[test]
    fn a_missing_fallback_dir_is_created_private_and_then_accepted_again() {
        let s = Scratch::new("create");
        private_dir(&s.0, uid()).unwrap();
        assert!(s.0.is_dir());
        assert_eq!(mode_of(&s.0), 0o700, "must be private whatever the umask");
        private_dir(&s.0, uid()).expect("our own private dir must be accepted on the next start");
    }

    #[test]
    fn a_fallback_dir_that_others_can_write_is_refused() {
        use std::os::unix::fs::PermissionsExt;
        let s = Scratch::new("mode");
        std::fs::create_dir(&s.0).unwrap();
        std::fs::set_permissions(&s.0, std::fs::Permissions::from_mode(0o755)).unwrap();
        let err = private_dir(&s.0, uid()).unwrap_err();
        assert!(err.to_string().contains("not private"), "got {err}");
    }

    #[test]
    fn a_fallback_dir_owned_by_someone_else_is_refused() {
        let s = Scratch::new("owner");
        std::fs::create_dir(&s.0).unwrap();
        // The dir is ours; asking for a different uid is the same check
        // from the other side (a real foreign dir cannot be made in a test).
        let err = private_dir(&s.0, uid().wrapping_add(1)).unwrap_err();
        assert!(err.to_string().contains("owned by"), "got {err}");
    }

    #[test]
    fn a_symlink_or_file_in_place_of_the_fallback_dir_is_refused() {
        let real = Scratch::new("real");
        private_dir(&real.0, uid()).unwrap();
        let link = Scratch::new("link");
        std::os::unix::fs::symlink(&real.0, &link.0).unwrap();
        let err = private_dir(&link.0, uid()).unwrap_err();
        assert!(err.to_string().contains("not a directory"), "a symlink to a private dir is still not ours: {err}");
        let file = Scratch::new("file");
        std::fs::write(&file.0, b"").unwrap();
        assert!(private_dir(&file.0, uid()).is_err());
    }
}
