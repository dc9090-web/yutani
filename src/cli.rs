//! `yutani <command>`: send one request over the IPC socket and exit.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::time::Duration;

use crate::ipc::{Request, Response, socket_path};

const TIMEOUT: Duration = Duration::from_secs(3);

/// True when an app instance is listening on the socket.
pub fn is_running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

/// Send `request`; print the reply; map it to an exit code.
pub fn send(request: &Request) -> ExitCode {
    let path = socket_path();
    let stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        Err(err) if matches!(err.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) => {
            eprintln!("yutani is not running");
            return ExitCode::from(1);
        }
        Err(err) => {
            eprintln!("yutani: cannot connect to {}: {err}", path.display());
            return ExitCode::from(1);
        }
    };
    if let Err(err) = stream.set_read_timeout(Some(TIMEOUT)).and(stream.set_write_timeout(Some(TIMEOUT))) {
        eprintln!("yutani: socket setup failed: {err}");
        return ExitCode::from(1);
    }
    let mut writer = &stream;
    if let Err(err) = writer.write_all(request.to_line().as_bytes()) {
        eprintln!("yutani: send failed: {err}");
        return ExitCode::from(1);
    }
    let mut line = String::new();
    if let Err(err) = BufReader::new(&stream).read_line(&mut line) {
        eprintln!("yutani: no reply: {err}");
        return ExitCode::from(1);
    }
    match Response::parse(&line) {
        Response::Ok => ExitCode::SUCCESS,
        Response::Err(msg) => {
            eprintln!("yutani: {msg}");
            ExitCode::from(1)
        }
    }
}
