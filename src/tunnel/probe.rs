//! The popover's PING · TQ (Nostromo spec, Phase 2): every [`PERIOD`] a
//! probe times how long Tranquility takes to greet a new connection —
//! connect, then the first byte of the 49-byte hello the server sends
//! unprompted. Nothing is sent; the socket is dropped once that byte is in.
//!
//! Two probes run. The root worker's binds the *tunnel address*, so the
//! `from <address>` policy rule sends it down `yutani0` as EVE's traffic
//! goes. The daemon's binds nothing and takes the ordinary route — the
//! daemon is not in EVE's cgroup — so the popover can show what the tunnel
//! saves (Daniel, 2026-10-08).
//!
//! Why the greeting and not the connect: TQ sits behind Cloudflare's
//! anycast proxy, so a TCP connect completes at the nearest Cloudflare
//! edge — ≈15 ms off the tunnel from the far side of the world, against
//! ≈325–360 ms through a London exit — while the greeting comes from CCP's
//! own server (≈595 ms off the tunnel, measured 2026-10-08). Only the
//! greeting compares the two paths fairly.
//!
//! Both pause unless wanted (the daemon's lease, see
//! [`super::ping_lease_path`]): around the clock they would knock on CCP's
//! login server tens of thousands of times a day for nobody.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const TQ_HOST: &str = "tranquility.servers.eveonline.com";
pub const TQ_PORT: u16 = 26000;
/// How long a probe may take, greeting included. Above the ≈600 ms the
/// greeting takes off the tunnel with room to spare, and short enough that
/// a lost probe still lands inside the applet's `ping::STALE_AFTER`.
pub const TIMEOUT: Duration = Duration::from_millis(2500);
/// How long a TQ lookup may take.
pub const DNS_TIMEOUT: Duration = Duration::from_millis(1500);
/// One probe per path this often. Each one reaches CCP's server (not just
/// Cloudflare's edge), so half the connect probe's old 1 Hz.
pub const PERIOD: Duration = Duration::from_secs(2);
pub const DNS_TTL: Duration = Duration::from_secs(300);
/// After a failed refresh the old address is kept and the lookup retried
/// this much later, not on every tick.
pub const DNS_RETRY: Duration = Duration::from_secs(30);

/// The newest probe and how many there have been.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sample {
    pub rtt_us: Option<u32>,
    pub seq: u64,
}

/// Shared between the probe task and the status writer.
#[derive(Clone, Debug, Default)]
pub struct Latest(Arc<Mutex<Sample>>);

impl Latest {
    pub fn get(&self) -> Sample {
        *self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn record(&self, rtt_us: Option<u32>) {
        let mut s = self.0.lock().unwrap_or_else(|p| p.into_inner());
        s.rtt_us = rtt_us;
        s.seq += 1;
    }
}

/// One timed greeting from `target`: connect (from `source` when given,
/// else wherever the routing table says), then wait for the first byte.
/// `None` on any failure (unbindable source, refusal, a close before any
/// byte, timeout).
pub async fn greeting_rtt(source: Option<Ipv4Addr>, target: SocketAddr, timeout: Duration) -> Option<u32> {
    use tokio::io::AsyncReadExt as _;
    let socket = tokio::net::TcpSocket::new_v4().ok()?;
    if let Some(source) = source {
        socket.bind(SocketAddr::from((source, 0))).ok()?;
    }
    let start = Instant::now();
    let greeted = async {
        let mut stream = socket.connect(target).await.ok()?;
        let mut byte = [0u8; 1];
        (stream.read(&mut byte).await.ok()? == 1).then_some(())
    };
    tokio::time::timeout(timeout, greeted).await.ok()??;
    Some(u32::try_from(start.elapsed().as_micros()).unwrap_or(u32::MAX))
}

async fn resolve() -> Option<SocketAddr> {
    let lookup = tokio::net::lookup_host((TQ_HOST, TQ_PORT));
    tokio::time::timeout(DNS_TIMEOUT, lookup).await.ok()?.ok()?.find(SocketAddr::is_ipv4)
}

/// Whether this tick should look TQ up. With no address at all, every
/// tick; with one, once it is `DNS_TTL` old when the last attempt
/// succeeded, or `DNS_RETRY` after the last attempt when that one failed.
/// `last_attempt` is `(when, succeeded)`.
fn should_resolve(last_attempt: Option<(Instant, bool)>, have_addr: bool, now: Instant) -> bool {
    match last_attempt {
        _ if !have_addr => true,
        None => true,
        Some((at, ok)) => now.saturating_duration_since(at) >= if ok { DNS_TTL } else { DNS_RETRY },
    }
}

/// Probe every [`PERIOD`] until `stop` is set, while `wanted()` says so.
/// A paused tick does nothing at all — no lookup, no connect, no `record`
/// — so the sequence stands still and the applet sees it stall. A failed
/// resolution keeps the previous address (see [`should_resolve`]).
async fn run(source: Option<Ipv4Addr>, latest: Latest, stop: Arc<AtomicBool>, wanted: impl Fn() -> bool) {
    probe_loop(source, latest, stop, wanted, PERIOD, resolve).await
}

async fn probe_loop<F, Fut>(
    source: Option<Ipv4Addr>,
    latest: Latest,
    stop: Arc<AtomicBool>,
    wanted: impl Fn() -> bool,
    period: Duration,
    resolve: F,
) where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<SocketAddr>>,
{
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut target: Option<SocketAddr> = None;
    let mut last_attempt: Option<(Instant, bool)> = None;
    loop {
        tick.tick().await;
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if !wanted() {
            continue;
        }
        if should_resolve(last_attempt, target.is_some(), Instant::now()) {
            let got = resolve().await;
            last_attempt = Some((Instant::now(), got.is_some()));
            target = got.or(target);
        }
        let rtt = match target {
            Some(addr) => greeting_rtt(source, addr, TIMEOUT).await,
            None => None,
        };
        latest.record(rtt);
    }
}

/// A running probe; dropping it tells its thread to stop (within a tick).
pub struct Probe(Arc<AtomicBool>);

impl Drop for Probe {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

/// Start the probe on its own OS thread with its own runtime, so the
/// worker's blocking subprocess calls (`wg show`, the exit-IP `curl`)
/// cannot hold up the connect it is timing. The runtime is built here,
/// before the thread, so `None` (with a warning) covers both a runtime
/// and a thread that cannot be made: the tunnel carries on without a ping.
/// The tunnel worker's probe, from the tunnel address, while `wanted` is
/// set.
pub fn spawn(source: Ipv4Addr, latest: Latest, wanted: Arc<AtomicBool>) -> Option<Probe> {
    start("tq-probe", Some(source), latest, move || wanted.load(Ordering::Relaxed))
}

/// The daemon's probe over the ordinary route, while the ping lease in
/// `runtime_dir` is fresh — the same lease that wakes the worker's.
pub fn spawn_direct(runtime_dir: std::path::PathBuf, latest: Latest) -> Option<Probe> {
    let lease = super::ping_lease_in(&runtime_dir);
    start("tq-probe-direct", None, latest, move || super::lease_fresh(&lease, std::time::SystemTime::now()))
}

fn start(name: &str, source: Option<Ipv4Addr>, latest: Latest, wanted: impl Fn() -> bool + Send + 'static) -> Option<Probe> {
    let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::warn!("ping probe: runtime: {e}");
            return None;
        }
    };
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let started = std::thread::Builder::new()
        .name(name.into())
        .spawn(move || rt.block_on(run(source, latest, flag, wanted)));
    match started {
        Ok(_) => Some(Probe(stop)),
        Err(e) => {
            tracing::warn!("ping probe: thread: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    /// A stand-in for TQ: greets every connection with one byte, on its
    /// own thread for the life of the test process.
    fn greeter() -> SocketAddr {
        use std::io::Write as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for mut conn in listener.incoming().flatten() {
                let _ = conn.write_all(b"-");
            }
        });
        addr
    }

    #[test]
    fn a_greeting_is_a_round_trip_from_any_source_or_none() {
        let target = greeter();
        block_on(async {
            for source in [Some(Ipv4Addr::LOCALHOST), None] {
                let rtt = greeting_rtt(source, target, TIMEOUT).await;
                assert!(rtt.is_some_and(|us| us < 1_000_000), "{source:?}: {rtt:?}");
            }
        });
    }

    /// A server that accepts but never speaks is a loss, at the timeout —
    /// the connect alone (Cloudflare's edge) is not a round trip to TQ.
    #[test]
    fn a_silent_server_is_a_loss() {
        block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target = listener.local_addr().unwrap();
            let rtt = greeting_rtt(None, target, Duration::from_millis(200)).await;
            assert_eq!(rtt, None);
        });
    }

    #[test]
    fn a_closed_port_is_a_loss() {
        block_on(async {
            // Bound but never listening, and held for the whole test: the
            // port refuses at once and no other test can take it meanwhile
            // (binding and dropping raced a parallel listener, 2026-10-07).
            let held = tokio::net::TcpSocket::new_v4().unwrap();
            held.bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
            let target = held.local_addr().unwrap();
            assert_eq!(greeting_rtt(Some(Ipv4Addr::LOCALHOST), target, TIMEOUT).await, None);
            drop(held);
        });
    }

    /// A source address this host does not own cannot be bound: a loss, not
    /// a panic (the tunnel address vanishes when the link goes down).
    #[test]
    fn an_unusable_source_is_a_loss() {
        block_on(async {
            let target = SocketAddr::from((Ipv4Addr::LOCALHOST, 9));
            assert_eq!(greeting_rtt(Some(Ipv4Addr::new(192, 0, 2, 1)), target, TIMEOUT).await, None);
        });
    }

    #[test]
    fn every_record_advances_the_sequence_and_keeps_the_latest() {
        let l = Latest::default();
        assert_eq!(l.get(), Sample { rtt_us: None, seq: 0 });
        l.record(Some(361_000));
        l.record(None);
        assert_eq!(l.get(), Sample { rtt_us: None, seq: 2 });
        l.record(Some(359_000));
        assert_eq!(l.get(), Sample { rtt_us: Some(359_000), seq: 3 });
    }

    #[test]
    fn a_dropped_probe_stops_its_loop() {
        let target = greeter();
        let latest = Latest::default();
        let stop = Arc::new(AtomicBool::new(false));
        let probe = Probe(stop.clone());
        let (l, s) = (latest.clone(), stop);
        let wanted = Arc::new(AtomicBool::new(true));
        let th = std::thread::spawn(move || {
            block_on(probe_loop(Some(Ipv4Addr::LOCALHOST), l, s, move || wanted.load(Ordering::Relaxed), Duration::from_millis(50), || async move { Some(target) }))
        });
        std::thread::sleep(Duration::from_millis(400));
        assert!(latest.get().seq >= 2, "{:?}", latest.get());
        drop(probe);
        th.join().unwrap(); // returns only because the flag was seen
        let seq = latest.get().seq;
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(latest.get().seq, seq);
    }

    /// Unwanted, a tick does nothing: no lookup, no record, `seq` stands
    /// still; wanted again, it carries on.
    #[test]
    fn an_unwanted_probe_neither_resolves_nor_records() {
        let target = greeter();
        let latest = Latest::default();
        let stop = Arc::new(AtomicBool::new(false));
        let wanted = Arc::new(AtomicBool::new(false));
        let lookups = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let (l, s, w, n) = (latest.clone(), stop.clone(), wanted.clone(), lookups.clone());
        let th = std::thread::spawn(move || {
            block_on(probe_loop(Some(Ipv4Addr::LOCALHOST), l, s, move || w.load(Ordering::Relaxed), Duration::from_millis(50), move || {
                n.fetch_add(1, Ordering::Relaxed);
                async move { Some(target) }
            }))
        });
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(latest.get().seq, 0);
        assert_eq!(lookups.load(Ordering::Relaxed), 0);
        wanted.store(true, Ordering::Relaxed);
        std::thread::sleep(Duration::from_millis(300));
        assert!(latest.get().seq >= 2, "{:?}", latest.get());
        assert_eq!(lookups.load(Ordering::Relaxed), 1, "a good address is reused");
        stop.store(true, Ordering::Relaxed);
        th.join().unwrap();
    }

    #[test]
    fn lookups_follow_the_ttl_after_success_and_the_retry_after_failure() {
        let t0 = Instant::now();
        let s = Duration::from_secs;
        // No address: every tick, whatever happened last.
        assert!(should_resolve(None, false, t0));
        assert!(should_resolve(Some((t0, false)), false, t0 + s(1)));
        assert!(should_resolve(Some((t0, true)), false, t0 + s(1)));
        // Last attempt succeeded: reuse until DNS_TTL.
        assert!(!should_resolve(Some((t0, true)), true, t0 + s(1)));
        assert!(!should_resolve(Some((t0, true)), true, t0 + DNS_TTL - s(1)));
        assert!(should_resolve(Some((t0, true)), true, t0 + DNS_TTL));
        // Last attempt failed but an old address is kept: retry after DNS_RETRY, not every tick.
        assert!(!should_resolve(Some((t0, false)), true, t0 + s(1)));
        assert!(!should_resolve(Some((t0, false)), true, t0 + DNS_RETRY - s(1)));
        assert!(should_resolve(Some((t0, false)), true, t0 + DNS_RETRY));
        // A clock that reads earlier than the attempt never forces a lookup.
        assert!(!should_resolve(Some((t0 + s(5), true)), true, t0));
    }

    /// `cargo test --lib tunnel::probe -- --ignored --nocapture`: prints
    /// the live greeting time from TQ directly and, with the tunnel up,
    /// through 10.2.0.2.
    #[test]
    #[ignore]
    fn live_probe_direct_and_through_the_tunnel() {
        block_on(async {
            let addr = resolve().await.expect("resolve TQ");
            let direct = greeting_rtt(None, addr, TIMEOUT).await;
            let tunnel = greeting_rtt(Some(Ipv4Addr::new(10, 2, 0, 2)), addr, TIMEOUT).await;
            eprintln!("TQ {addr}: direct {direct:?} µs, via 10.2.0.2 {tunnel:?} µs");
            assert!(direct.is_some());
        });
    }
}
