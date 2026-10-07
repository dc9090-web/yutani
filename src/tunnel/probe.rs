//! The popover's PING · TQ (Nostromo spec, Phase 2): once a second the
//! root worker times a TCP connect to Tranquility *from the tunnel
//! address*, so the probe takes the same path EVE's traffic does — the
//! `from <address>` policy rule sends it down `yutani0`, as it does the
//! exit-address lookup. Nothing is sent; the socket is dropped as soon as
//! the handshake completes. Through a London exit from the far side of the world this is
//! ≈325–360 ms, where the home link reads ≈16 ms because it stops at the
//! local Cloudflare edge — the tunnel figure is the one that matters.
//! It pauses unless wanted (the daemon's lease, see
//! [`super::ping_lease_path`]): at 1 Hz around the clock it would knock on
//! CCP's login edge from a shared VPN exit 86k times a day for nobody.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const TQ_HOST: &str = "tranquility.servers.eveonline.com";
pub const TQ_PORT: u16 = 26000;
pub const TIMEOUT: Duration = Duration::from_millis(1500);
pub const DNS_TTL: Duration = Duration::from_secs(300);
/// After a failed refresh the old address is kept and the lookup retried
/// this much later, not on every tick.
pub const DNS_RETRY: Duration = Duration::from_secs(30);

/// The newest probe and how many there have been.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// One timed connect from `source` to `target`; `None` on any failure
/// (unbindable source, refusal, timeout).
pub async fn connect_rtt(source: Ipv4Addr, target: SocketAddr, timeout: Duration) -> Option<u32> {
    let socket = tokio::net::TcpSocket::new_v4().ok()?;
    socket.bind(SocketAddr::from((source, 0))).ok()?;
    let start = Instant::now();
    let stream = tokio::time::timeout(timeout, socket.connect(target)).await.ok()?.ok()?;
    let rtt = start.elapsed();
    drop(stream);
    Some(u32::try_from(rtt.as_micros()).unwrap_or(u32::MAX))
}

async fn resolve() -> Option<SocketAddr> {
    let lookup = tokio::net::lookup_host((TQ_HOST, TQ_PORT));
    tokio::time::timeout(TIMEOUT, lookup).await.ok()?.ok()?.find(SocketAddr::is_ipv4)
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

/// Probe once a second until `stop` is set, while `wanted` is. A paused
/// tick does nothing at all — no lookup, no connect, no `record` — so the
/// sequence stands still and the applet sees it stall. A failed
/// resolution keeps the previous address (see [`should_resolve`]).
pub async fn run(source: Ipv4Addr, latest: Latest, stop: Arc<AtomicBool>, wanted: Arc<AtomicBool>) {
    probe_loop(source, latest, stop, wanted, Duration::from_secs(1), resolve).await
}

async fn probe_loop<F, Fut>(
    source: Ipv4Addr,
    latest: Latest,
    stop: Arc<AtomicBool>,
    wanted: Arc<AtomicBool>,
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
        if !wanted.load(Ordering::Relaxed) {
            continue;
        }
        if should_resolve(last_attempt, target.is_some(), Instant::now()) {
            let got = resolve().await;
            last_attempt = Some((Instant::now(), got.is_some()));
            target = got.or(target);
        }
        let rtt = match target {
            Some(addr) => connect_rtt(source, addr, TIMEOUT).await,
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
/// It probes only while `wanted` is set.
pub fn spawn(source: Ipv4Addr, latest: Latest, wanted: Arc<AtomicBool>) -> Option<Probe> {
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
        .name("tq-probe".into())
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

    #[test]
    fn a_listening_port_answers_with_a_round_trip() {
        block_on(async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target = listener.local_addr().unwrap();
            let rtt = connect_rtt(Ipv4Addr::LOCALHOST, target, TIMEOUT).await;
            assert!(rtt.is_some_and(|us| us < 1_000_000), "{rtt:?}");
        });
    }

    #[test]
    fn a_closed_port_is_a_loss() {
        block_on(async {
            // Bind and drop: the port is now closed and refuses at once.
            let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
            let target = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
            assert_eq!(connect_rtt(Ipv4Addr::LOCALHOST, target, TIMEOUT).await, None);
        });
    }

    /// A source address this host does not own cannot be bound: a loss, not
    /// a panic (the tunnel address vanishes when the link goes down).
    #[test]
    fn an_unusable_source_is_a_loss() {
        block_on(async {
            let target = SocketAddr::from((Ipv4Addr::LOCALHOST, 9));
            assert_eq!(connect_rtt(Ipv4Addr::new(192, 0, 2, 1), target, TIMEOUT).await, None);
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
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target = listener.local_addr().unwrap();
        let latest = Latest::default();
        let stop = Arc::new(AtomicBool::new(false));
        let probe = Probe(stop.clone());
        let (l, s) = (latest.clone(), stop);
        let wanted = Arc::new(AtomicBool::new(true));
        let th = std::thread::spawn(move || {
            block_on(probe_loop(Ipv4Addr::LOCALHOST, l, s, wanted, Duration::from_millis(50), || async move { Some(target) }))
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
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let target = listener.local_addr().unwrap();
        let latest = Latest::default();
        let stop = Arc::new(AtomicBool::new(false));
        let wanted = Arc::new(AtomicBool::new(false));
        let lookups = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let (l, s, w, n) = (latest.clone(), stop.clone(), wanted.clone(), lookups.clone());
        let th = std::thread::spawn(move || {
            block_on(probe_loop(Ipv4Addr::LOCALHOST, l, s, w, Duration::from_millis(50), move || {
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

    /// `cargo test --lib tunnel::probe -- --ignored` with the tunnel up:
    /// prints the live RTT to TQ through 10.2.0.2.
    #[test]
    #[ignore]
    fn live_probe_through_the_tunnel() {
        block_on(async {
            let addr = resolve().await.expect("resolve TQ");
            let rtt = connect_rtt(Ipv4Addr::new(10, 2, 0, 2), addr, TIMEOUT).await;
            eprintln!("TQ {addr} via 10.2.0.2: {rtt:?} µs");
            assert!(rtt.is_some());
        });
    }
}
