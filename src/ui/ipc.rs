//! IPC server: `$XDG_RUNTIME_DIR/yutani.sock` as an iced subscription. Each
//! connection carries one request; the app answers through `Responder`.

use cosmic::iced::futures::channel::mpsc;
use cosmic::iced::futures::{SinkExt, StreamExt};
use cosmic::iced::{self, Subscription};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tokio::sync::oneshot;

use crate::ipc::{MAX_LINE, Request, Response, socket_path};

/// One-shot reply channel. `Clone` (so it can live in a `Msg`) but only the
/// first `respond` is delivered.
#[derive(Clone, Debug)]
pub struct Responder(Arc<Mutex<Option<oneshot::Sender<Response>>>>);

impl Responder {
    pub fn respond(&self, response: Response) {
        if let Some(tx) = self.0.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = tx.send(response);
        }
    }
}

#[cfg(test)]
impl Responder {
    /// A reply handle together with the receiver a connection task would
    /// be waiting on, so a test can see whether (and what) `handle_request`
    /// answered from the update thread.
    pub fn detached() -> (Responder, oneshot::Receiver<Response>) {
        let (tx, rx) = oneshot::channel();
        (Responder(Arc::new(Mutex::new(Some(tx)))), rx)
    }
}

#[derive(Clone, Debug)]
pub struct IpcEvent {
    pub request: Request,
    pub reply: Responder,
}

pub fn subscription() -> Subscription<IpcEvent> {
    Subscription::run(run)
}

/// Delete the socket file (on quit). Safe to call when it doesn't exist.
pub fn remove_socket() {
    let _ = socket_path().and_then(std::fs::remove_file);
}

/// Bind the listener, replacing a stale socket left by a crashed instance
/// but never one that still answers.
fn bind() -> std::io::Result<UnixListener> {
    let path = socket_path()?;
    if path.exists() {
        if StdUnixStream::connect(&path).is_ok() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AddrInUse,
                "another yutani instance is listening",
            ));
        }
        std::fs::remove_file(&path)?;
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    tracing::info!("ipc listening on {}", path.display());
    Ok(listener)
}

/// How long to wait after the `failures`-th consecutive `accept` error
/// (1-based) before trying again: 100 ms, doubling to a 1 s ceiling. An
/// error that persists — EMFILE/ENFILE once the process is at its fd limit
/// (dmabuf planes, GL, the reader threads' pipes), ENOMEM — comes back
/// immediately on every call, and a loop that just `continue`s spins one
/// tokio worker at 100 % and writes a warning per iteration to the journal.
pub fn accept_backoff(failures: u32) -> Duration {
    let ms = 100u64.saturating_mul(1u64 << failures.saturating_sub(1).min(4));
    Duration::from_millis(ms.min(1000))
}

fn run() -> impl iced::futures::Stream<Item = IpcEvent> {
    let (tx, rx) = mpsc::channel::<IpcEvent>(16);
    // The accept loop never yields items itself; it feeds `tx`. Selecting it
    // with `rx` keeps it alive for as long as the subscription runs.
    let accept_loop = iced::futures::stream::once(async move {
        let listener = match bind() {
            Ok(l) => l,
            Err(err) => {
                tracing::warn!("ipc unavailable: {err}; `yutani focus` and shortcuts will not work");
                return;
            }
        };
        let mut failures = 0u32;
        loop {
            let (stream, _) = match listener.accept().await {
                Ok(s) => {
                    failures = 0;
                    s
                }
                Err(err) => {
                    failures = failures.saturating_add(1);
                    tracing::warn!("ipc accept failed: {err}");
                    tokio::time::sleep(accept_backoff(failures)).await;
                    continue;
                }
            };
            let mut tx = tx.clone();
            tokio::spawn(async move {
                let (read, mut write) = stream.into_split();
                let mut line = String::new();
                let mut reader = BufReader::new(read).take(MAX_LINE as u64);
                let response = match reader.read_line(&mut line).await {
                    Ok(0) => Response::Err("empty request".into()),
                    Ok(_) if !line.ends_with('\n') && line.len() >= MAX_LINE => {
                        Response::Err("request too long".into())
                    }
                    Ok(_) => match Request::parse(&line) {
                        Err(msg) => Response::Err(msg),
                        Ok(request) => {
                            let (reply_tx, reply_rx) = oneshot::channel();
                            let reply = Responder(Arc::new(Mutex::new(Some(reply_tx))));
                            if tx.send(IpcEvent { request, reply }).await.is_err() {
                                Response::Err("app is shutting down".into())
                            } else {
                                reply_rx
                                    .await
                                    .unwrap_or_else(|_| Response::Err("no reply from app".into()))
                            }
                        }
                    },
                    Err(err) => Response::Err(format!("read failed: {err}")),
                };
                let _ = write.write_all(response.to_line().as_bytes()).await;
                let _ = write.shutdown().await;
            });
        }
    })
    .filter_map(|()| async { None::<IpcEvent> });
    iced::futures::stream::select(accept_loop, rx)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [I5] A persistent accept error must not spin the worker: every
    /// failure waits, the wait grows, and it never exceeds a second.
    #[test]
    fn accept_errors_back_off_from_100ms_to_a_second() {
        assert_eq!(accept_backoff(1), Duration::from_millis(100));
        assert_eq!(accept_backoff(2), Duration::from_millis(200));
        assert_eq!(accept_backoff(3), Duration::from_millis(400));
        assert_eq!(accept_backoff(4), Duration::from_millis(800));
        assert_eq!(accept_backoff(5), Duration::from_secs(1));
        assert_eq!(accept_backoff(u32::MAX), Duration::from_secs(1));
        assert!(accept_backoff(0) >= Duration::from_millis(100), "even a nonsense count waits");
    }
}
