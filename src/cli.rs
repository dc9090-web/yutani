//! `yutani <command>`: send one request over the IPC socket and exit.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::Duration;

use crate::ipc::{Request, Response, socket_path};

const TIMEOUT: Duration = Duration::from_secs(3);

/// True when an app instance is listening on the socket.
pub fn is_running() -> bool {
    socket_path().and_then(|path| UnixStream::connect(path)).is_ok()
}

/// Send one request and return the reply. `Err` is a message already
/// formatted for stderr.
fn exchange(request: &Request) -> Result<Response, String> {
    let path = socket_path().map_err(|err| format!("yutani: {err}"))?;
    let stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(err) if matches!(err.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            return Err("yutani is not running".into());
        }
        Err(err) => return Err(format!("yutani: cannot connect to {}: {err}", path.display())),
    };
    stream
        .set_read_timeout(Some(TIMEOUT))
        .and(stream.set_write_timeout(Some(TIMEOUT)))
        .map_err(|err| format!("yutani: socket setup failed: {err}"))?;
    let mut writer = &stream;
    writer.write_all(request.to_line().as_bytes()).map_err(|err| format!("yutani: send failed: {err}"))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line).map_err(|err| format!("yutani: no reply: {err}"))?;
    Ok(Response::parse(&line))
}

/// Send `request`; print the reply; map it to an exit code.
pub fn send(request: &Request) -> ExitCode {
    match exchange(request) {
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
        Ok(Response::Ok) => ExitCode::SUCCESS,
        Ok(Response::OkData(data)) => {
            println!("{data}");
            ExitCode::SUCCESS
        }
        Ok(Response::Err(msg)) => {
            eprintln!("yutani: {msg}");
            ExitCode::from(1)
        }
    }
}

/// `yutani layouts`: the saved layout names, one per line (and nothing at
/// all when none are saved).
pub fn layouts() -> ExitCode {
    match exchange(&Request::Layouts) {
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::from(1)
        }
        Ok(Response::OkData(json)) => match serde_json::from_str::<Vec<String>>(&json) {
            Ok(names) => {
                for name in names {
                    println!("{name}");
                }
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("yutani: cannot read the layout list: {err}");
                ExitCode::from(1)
            }
        },
        Ok(Response::Ok) => ExitCode::SUCCESS,
        Ok(Response::Err(msg)) => {
            eprintln!("yutani: {msg}");
            ExitCode::from(1)
        }
    }
}
