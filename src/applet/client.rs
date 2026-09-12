//! The applet's side of the IPC socket: connect, write one request line,
//! read one reply line, close. A connect that finds nothing listening is
//! the daemon-offline signal, not an error to show.

use std::path::{Path, PathBuf};

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::ipc::{MAX_LINE, Request, Response, socket_path};
use crate::tunnel::status::Status;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IpcError {
    /// Nothing is listening on the socket: the daemon is not running.
    Offline,
    /// The daemon answered `err <msg>`, or the exchange itself failed.
    Failed(String),
}

impl std::fmt::Display for IpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IpcError::Offline => f.write_str("yutani is not running"),
            IpcError::Failed(msg) => f.write_str(msg),
        }
    }
}

/// One request, one reply. `Ok(None)` is a bare `ok`; `Ok(Some(data))` is
/// `ok <data>` (plan A's `Response::OkData`).
pub async fn send_to(path: &Path, request: &Request) -> Result<Option<String>, IpcError> {
    let stream = match UnixStream::connect(path).await {
        Ok(stream) => stream,
        Err(err)
            if matches!(err.kind(), std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused) =>
        {
            return Err(IpcError::Offline);
        }
        Err(err) => return Err(IpcError::Failed(format!("connect: {err}"))),
    };
    let (read, mut write) = stream.into_split();
    write
        .write_all(request.to_line().as_bytes())
        .await
        .map_err(|err| IpcError::Failed(format!("send: {err}")))?;
    let mut line = String::new();
    BufReader::new(read.take(MAX_LINE as u64))
        .read_line(&mut line)
        .await
        .map_err(|err| IpcError::Failed(format!("reply: {err}")))?;
    match Response::parse(&line) {
        Response::Ok => Ok(None),
        Response::OkData(data) => Ok(Some(data)),
        Response::Err(msg) => Err(IpcError::Failed(msg)),
    }
}

/// `send_to` against `$XDG_RUNTIME_DIR/yutani.sock`.
pub async fn send(request: Request) -> Result<Option<String>, IpcError> {
    send_to(&socket_path(), &request).await
}

/// The `status` request, parsed. Takes an owned path so the future is
/// `'static` and can be handed to `Task::future`.
pub async fn status_from(path: PathBuf) -> Result<Status, IpcError> {
    let data = send_to(&path, &Request::Status)
        .await?
        .ok_or_else(|| IpcError::Failed("status: daemon replied ok with no data".to_string()))?;
    serde_json::from_str(&data).map_err(|err| IpcError::Failed(format!("status: {err}")))
}

pub async fn status() -> Result<Status, IpcError> {
    status_from(socket_path()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::status::{ClientStatus, Status, TunnelStatus};

    /// A current-thread runtime; `#[tokio::test]` would need the `macros`
    /// feature (and a new lock entry), which this crate does not have.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_io().build().unwrap().block_on(f)
    }

    fn socket(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("yutani-client-{}-{tag}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        dir.join("yutani.sock")
    }

    /// Accept exactly one connection, read the request line, answer `reply`.
    /// Returns the request line the client sent.
    async fn one_shot(path: &std::path::Path, reply: &'static str) -> tokio::task::JoinHandle<String> {
        let _ = std::fs::remove_file(path);
        let listener = tokio::net::UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
            let (stream, _) = listener.accept().await.unwrap();
            let (read, mut write) = stream.into_split();
            let mut line = String::new();
            BufReader::new(read).read_line(&mut line).await.unwrap();
            write.write_all(reply.as_bytes()).await.unwrap();
            write.shutdown().await.unwrap();
            line
        })
    }

    fn sample_status() -> Status {
        Status {
            clients: vec![ClientStatus { name: "KestrelVance".into(), active: true }],
            hidden: false,
            tunnel: TunnelStatus {
                installed: true,
                connected: true,
                iface: "yutani0".into(),
                location: "London".into(),
                address: Some("10.2.0.2".into()),
                endpoint: Some("198.51.100.10:51820".into()),
                handshake_age_s: Some(21),
                rx_bytes: 413_100_000,
                tx_bytes: 2_790_000_000,
            },
        }
    }

    #[test]
    fn a_missing_socket_means_the_daemon_is_offline() {
        let path = socket("absent");
        let _ = std::fs::remove_file(&path);
        let err = block_on(status_from(path)).unwrap_err();
        assert_eq!(err, IpcError::Offline);
    }

    #[test]
    fn status_round_trips_through_the_line_protocol() {
        let path = socket("ok");
        let json = serde_json::to_string(&sample_status()).unwrap();
        let reply: &'static str = Box::leak(format!("ok {json}\n").into_boxed_str());
        let got = block_on(async {
            let server = one_shot(&path, reply).await;
            let status = status_from(path.clone()).await;
            (server.await.unwrap(), status)
        });
        assert_eq!(got.0, "status\n");
        let status = got.1.unwrap();
        assert_eq!(status.clients[0].name, "KestrelVance");
        assert!(status.clients[0].active);
        assert_eq!(status.tunnel.handshake_age_s, Some(21));
        assert_eq!(status.tunnel.tx_bytes, 2_790_000_000);
    }

    #[test]
    fn an_err_reply_becomes_a_failure_and_a_plain_ok_carries_no_data() {
        let path = socket("err");
        let got = block_on(async {
            let server = one_shot(&path, "err no client 3 (2 known)\n").await;
            let r = send_to(&path, &crate::ipc::Request::Focus(3)).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "focus 3\n");
        assert_eq!(got.1.unwrap_err(), IpcError::Failed("no client 3 (2 known)".into()));

        let path = socket("bare");
        let got = block_on(async {
            let server = one_shot(&path, "ok\n").await;
            let r = send_to(&path, &crate::ipc::Request::Hide).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "hide\n");
        assert_eq!(got.1.unwrap(), None);
    }

    #[test]
    fn a_reply_that_is_not_json_is_a_failure_not_a_panic() {
        let path = socket("garbage");
        let got = block_on(async {
            let server = one_shot(&path, "ok not json at all\n").await;
            let r = status_from(path.clone()).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "status\n");
        assert!(matches!(got.1.unwrap_err(), IpcError::Failed(m) if m.starts_with("status:")));
    }
}
