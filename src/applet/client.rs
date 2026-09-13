//! The applet's side of the IPC socket: connect, write one request line,
//! read one reply line, close. A connect that finds nothing listening is
//! the daemon-offline signal, not an error to show.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use crate::ipc::{MAX_REPLY, Request, Response, socket_path};
use crate::tunnel::status::Status;

/// Budget for one full round trip (connect + write + reply). A daemon that
/// accepts the connection but never answers would otherwise hang the
/// applet's poll task forever.
pub const IPC_TIMEOUT: Duration = Duration::from_secs(3);

/// Budget for `tunnel connect|disconnect`. The daemon answers those only
/// once its `systemctl start|stop yutani-tunnel` has returned, and the
/// unit's own `TimeoutStopSec` is 10 s (a wedged worker is SIGKILLed then),
/// so the `status` budget would give up — and show a timeout under the row
/// — on an action that is in fact succeeding.
pub const TUNNEL_TIMEOUT: Duration = Duration::from_secs(15);

/// The round-trip budget for `request`: [`TUNNEL_TIMEOUT`] for the two
/// tunnel actions, [`IPC_TIMEOUT`] for everything else.
pub fn budget(request: &Request) -> Duration {
    match request {
        Request::TunnelConnect | Request::TunnelDisconnect => TUNNEL_TIMEOUT,
        _ => IPC_TIMEOUT,
    }
}

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
    send_to_with(path, request, IPC_TIMEOUT).await
}

/// `send_to`, but with an injectable round-trip budget so tests don't have
/// to wait out the real `IPC_TIMEOUT`. On expiry this returns `Failed`, not
/// `Offline`: the daemon accepted the connection, so it exists — it's just
/// stuck.
pub async fn send_to_with(path: &Path, request: &Request, timeout: Duration) -> Result<Option<String>, IpcError> {
    match tokio::time::timeout(timeout, send_to_inner(path, request)).await {
        Ok(result) => result,
        Err(_) => Err(IpcError::Failed(format!("timeout after {}ms", timeout.as_millis()))),
    }
}

async fn send_to_inner(path: &Path, request: &Request) -> Result<Option<String>, IpcError> {
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
    let read_result =
        BufReader::new(read.take(MAX_REPLY as u64)).read_line(&mut line).await;
    // Nothing came back at all: the daemon accepted the connection and then
    // went away without writing a byte — a shutdown race, which is the
    // offline state, not a malformed reply. (`Response::parse("")` would
    // call it `malformed reply ""` and put that in front of the user.) It
    // arrives either as a clean EOF or, when the daemon died without
    // reading our request, as a connection reset; both mean the same thing,
    // so the test is "we read nothing", not "which errno".
    if line.is_empty() {
        return Err(IpcError::Offline);
    }
    read_result.map_err(|err| IpcError::Failed(format!("reply: {err}")))?;
    if !line.ends_with('\n') && line.len() >= MAX_REPLY {
        return Err(IpcError::Failed("reply too long".into()));
    }
    match Response::parse(&line) {
        Response::Ok => Ok(None),
        Response::OkData(data) => Ok(Some(data)),
        Response::Err(msg) => Err(IpcError::Failed(msg)),
    }
}

/// `send_to` against `$XDG_RUNTIME_DIR/yutani.sock`, with the request's
/// own [`budget`].
pub async fn send(request: Request) -> Result<Option<String>, IpcError> {
    send_to_with(&socket_path().map_err(|err| IpcError::Failed(err.to_string()))?, &request, budget(&request)).await
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
    status_from(socket_path().map_err(|err| IpcError::Failed(err.to_string()))?).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::status::{ClientStatus, Status, TunnelStatus};

    /// A current-thread runtime; `#[tokio::test]` would need the `macros`
    /// feature (and a new lock entry), which this crate does not have.
    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap()
            .block_on(f)
    }

    /// A throwaway directory holding one test's socket, removed when the
    /// guard is dropped — however the test ends. Derefs to the socket path,
    /// so it reads like the plain `PathBuf` it replaced.
    struct Sock {
        dir: std::path::PathBuf,
        path: std::path::PathBuf,
    }

    impl Drop for Sock {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    impl std::ops::Deref for Sock {
        type Target = Path;

        fn deref(&self) -> &Path {
            &self.path
        }
    }

    fn socket(tag: &str) -> Sock {
        let dir = std::env::temp_dir().join(format!("yutani-client-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("yutani.sock");
        Sock { dir, path }
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
                up_for_s: None,
                failed: false,
                exit_address: None,
                rx_bytes: 413_100_000,
                tx_bytes: 2_790_000_000,
            },
        }
    }

    #[test]
    fn a_missing_socket_means_the_daemon_is_offline() {
        let sock = socket("absent");
        let _ = std::fs::remove_file(&*sock);
        let err = block_on(status_from(sock.to_path_buf())).unwrap_err();
        assert_eq!(err, IpcError::Offline);
    }

    #[test]
    fn status_round_trips_through_the_line_protocol() {
        let sock = socket("ok");
        let json = serde_json::to_string(&sample_status()).unwrap();
        let reply: &'static str = Box::leak(format!("ok {json}\n").into_boxed_str());
        let got = block_on(async {
            let server = one_shot(&sock, reply).await;
            let status = status_from(sock.to_path_buf()).await;
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
        let sock = socket("err");
        let got = block_on(async {
            let server = one_shot(&sock, "err no client 3 (2 known)\n").await;
            let r = send_to(&sock, &crate::ipc::Request::Focus(3)).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "focus 3\n");
        assert_eq!(got.1.unwrap_err(), IpcError::Failed("no client 3 (2 known)".into()));

        let sock = socket("bare");
        let got = block_on(async {
            let server = one_shot(&sock, "ok\n").await;
            let r = send_to(&sock, &crate::ipc::Request::Hide).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "hide\n");
        assert_eq!(got.1.unwrap(), None);
    }

    #[test]
    fn a_reply_that_is_not_json_is_a_failure_not_a_panic() {
        let sock = socket("garbage");
        let got = block_on(async {
            let server = one_shot(&sock, "ok not json at all\n").await;
            let r = status_from(sock.to_path_buf()).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "status\n");
        assert!(matches!(got.1.unwrap_err(), IpcError::Failed(m) if m.starts_with("status:")));
    }

    /// A daemon that accepts the connection and then never answers must not
    /// hang the caller forever: the round trip has a budget, and blowing it
    /// is a `Failed`, not a silent wait.
    #[test]
    fn a_daemon_that_accepts_but_never_replies_times_out() {
        let sock = socket("hang");
        let got = block_on(async {
            let listener = tokio::net::UnixListener::bind(&*sock).unwrap();
            let server = tokio::spawn(async move {
                let (_stream, _) = listener.accept().await.unwrap();
                // Accept, then sit forever without reading or replying.
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
            let r = send_to_with(&sock, &Request::Status, Duration::from_millis(200)).await;
            server.abort();
            r
        });
        // Milliseconds, not seconds: a sub-second budget must not report
        // itself as "0s".
        assert!(
            matches!(&got, Err(IpcError::Failed(m)) if m == "timeout after 200ms"),
            "got {got:?}"
        );
    }

    /// The reply cap is much larger than the request cap: a `status` line
    /// listing many EVE clients can easily exceed 1 KiB and must not be
    /// truncated into a parse failure.
    #[test]
    fn a_multi_kilobyte_status_reply_round_trips() {
        let sock = socket("big-status");
        let clients: Vec<ClientStatus> = (0..100)
            .map(|i| ClientStatus { name: format!("Client-{i:03}-with-a-longish-callsign"), active: i % 3 == 0 })
            .collect();
        let mut status = sample_status();
        status.clients = clients;
        let json = serde_json::to_string(&status).unwrap();
        assert!(json.len() > 5 * 1024, "fixture is only {} bytes, want >5KiB", json.len());
        let reply: &'static str = Box::leak(format!("ok {json}\n").into_boxed_str());
        let got = block_on(async {
            let server = one_shot(&sock, reply).await;
            let status = status_from(sock.to_path_buf()).await;
            (server.await.unwrap(), status)
        });
        assert_eq!(got.0, "status\n");
        let status = got.1.unwrap();
        assert_eq!(status.clients.len(), 100);
        assert_eq!(status.clients[42].name, "Client-042-with-a-longish-callsign");
    }

    /// M6: a daemon shutting down accepts the connection and then closes it
    /// without writing a byte. That is not a malformed reply to show the
    /// user — it is the daemon going away, which is exactly the offline
    /// state, and the applet already knows how to present that.
    #[test]
    fn a_socket_that_closes_without_answering_is_the_offline_state() {
        // `read_request` picks which of the two shapes the daemon dies in:
        // false → it never read our line, so the close is a connection
        // reset; true → it read the line and then went, a clean EOF. Both
        // are "nothing came back", and both must read as offline.
        let hang_up = |tag: &'static str, read_request: bool| {
            let sock = socket(tag);
            let got = block_on(async {
                let listener = tokio::net::UnixListener::bind(&*sock).unwrap();
                let server = tokio::spawn(async move {
                    let (stream, _) = listener.accept().await.unwrap();
                    if read_request {
                        let mut line = String::new();
                        let _ = BufReader::new(stream).read_line(&mut line).await;
                    }
                    // and gone, without writing a byte
                });
                let plain = send_to(&sock, &Request::Status).await;
                let _ = server.await;
                plain
            });
            got.unwrap_err()
        };
        assert_eq!(hang_up("reset", false), IpcError::Offline, "a reset with nothing read");
        assert_eq!(hang_up("eof", true), IpcError::Offline, "a clean EOF with nothing read");

        // …and `status` says the same thing, rather than a JSON error.
        let sock = socket("shutdown-status");
        let got = block_on(async {
            let listener = tokio::net::UnixListener::bind(&*sock).unwrap();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                drop(stream);
            });
            let r = status_from(sock.to_path_buf()).await;
            let _ = server.await;
            r
        });
        assert_eq!(got.unwrap_err(), IpcError::Offline);
    }

    /// A reply with bytes in it but no newline is a different thing: the
    /// daemon spoke and got cut off mid-sentence. That stays a `Failed`, so
    /// it is reported rather than quietly presented as "not running".
    #[test]
    fn a_truncated_but_non_empty_reply_is_still_a_failure() {
        let sock = socket("truncated");
        let got = block_on(async {
            let server = one_shot(&sock, "ok {\"clients\":").await;
            let r = status_from(sock.to_path_buf()).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "status\n");
        assert!(matches!(got.1.unwrap_err(), IpcError::Failed(m) if m.starts_with("status:")));
    }

    /// A reply that blows straight through the cap without ever finding a
    /// newline is rejected explicitly, not silently truncated into garbage.
    #[test]
    fn a_reply_past_the_cap_with_no_newline_is_reply_too_long() {
        let sock = socket("toolong");
        let huge = "x".repeat(70 * 1024);
        let reply: &'static str = Box::leak(huge.into_boxed_str());
        let got = block_on(async {
            let server = one_shot(&sock, reply).await;
            let r = send_to(&sock, &Request::Status).await;
            (server.await.unwrap(), r)
        });
        assert_eq!(got.0, "status\n");
        assert!(matches!(got.1.unwrap_err(), IpcError::Failed(m) if m.contains("too long")));
    }

    /// I2: the daemon answers `tunnel connect|disconnect` only after its
    /// `systemctl start|stop` has completed, and the unit's own
    /// `TimeoutStopSec` is 10 s — so the 3 s `status` budget would give up
    /// on an action that is in fact succeeding. `status` keeps its 3 s.
    #[test]
    fn tunnel_requests_get_a_longer_budget_than_status() {
        assert_eq!(budget(&Request::TunnelConnect), TUNNEL_TIMEOUT);
        assert_eq!(budget(&Request::TunnelDisconnect), TUNNEL_TIMEOUT);
        assert_eq!(budget(&Request::Status), IPC_TIMEOUT);
        assert_eq!(budget(&Request::Quit), IPC_TIMEOUT);
        assert!(TUNNEL_TIMEOUT >= Duration::from_secs(12), "TimeoutStopSec=10 plus a margin");
    }
}
