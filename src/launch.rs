//! `yutani launch -- <command…>`: run a command (Steam's `%command%`) inside
//! a scope under `yutani-eve.slice`, so every socket it and its children
//! open is routed through the tunnel from the first packet.

use std::process::{Command, ExitCode};

use crate::tunnel::SLICE;

pub fn systemd_run_argv(command: &[String]) -> Vec<String> {
    let mut v: Vec<String> = ["systemd-run", "--user", "--scope", "--quiet", "--collect", &format!("--slice={SLICE}"), "--"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    v.extend(command.iter().cloned());
    v
}

/// Never stops the game from starting: if `systemd-run` is missing or
/// fails to launch, run the command directly and say so on stderr.
pub fn run(command: Vec<String>) -> ExitCode {
    if command.is_empty() {
        eprintln!("yutani launch: nothing to run (usage: yutani launch -- <command…>)");
        return ExitCode::from(2);
    }
    let argv = systemd_run_argv(&command);
    match Command::new(&argv[0]).args(&argv[1..]).status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(err) => {
            eprintln!("yutani launch: systemd-run unavailable ({err}); running without the tunnel cgroup");
            match Command::new(&command[0]).args(&command[1..]).status() {
                Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
                Err(err) => {
                    eprintln!("yutani launch: cannot run {}: {err}", command[0]);
                    ExitCode::from(127)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_the_command_in_a_scope_under_the_slice() {
        let argv = systemd_run_argv(&["/path/proton".into(), "run".into(), "exefile.exe".into()]);
        assert_eq!(&argv[..7], &["systemd-run", "--user", "--scope", "--quiet", "--collect", "--slice=yutani-eve.slice", "--"]);
        assert_eq!(&argv[7..], &["/path/proton", "run", "exefile.exe"]);
    }
}
