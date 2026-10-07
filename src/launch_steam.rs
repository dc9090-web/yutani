//! Start EVE through Steam for Launch EVE: `steam steam://rungameid/8500`,
//! detached (null stdio, own process group) and reaped on a thread, so a
//! Steam that exits is never a zombie under the daemon and a signal to the
//! daemon's group never reaches it.

use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

pub const URL: &str = "steam://rungameid/8500";

pub fn command(program: &str) -> Command {
    let mut c = Command::new(program);
    c.arg(URL).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).process_group(0);
    c
}

/// Spawn `program URL` and reap it in the background; `Err` if it cannot
/// be started at all.
pub fn spawn(program: &str) -> std::io::Result<()> {
    let mut child = command(program).spawn()?;
    std::thread::Builder::new().name("reap-steam".into()).spawn(move || {
        let _ = child.wait();
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_is_steam_with_the_eve_url() {
        let c = command("steam");
        assert_eq!(c.get_program(), "steam");
        assert_eq!(c.get_args().collect::<Vec<_>>(), [URL]);
    }

    #[test]
    fn a_missing_program_is_an_error_not_a_panic() {
        assert!(spawn("/no/such/steam-yutani-test").is_err());
        assert!(spawn("/bin/true").is_ok());
    }
}
