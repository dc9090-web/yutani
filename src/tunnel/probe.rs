//! The popover's PING · TQ (Nostromo spec, Phase 2): once a second the
//! root worker times a TCP connect to Tranquility *from the tunnel
//! address*, so the probe takes the same path EVE's traffic does — the
//! `from <address>` policy rule sends it down `yutani0`, as it does the
//! exit-address lookup. Nothing is sent; the socket is dropped as soon as
//! the handshake completes. Through a London exit from the far side of the world this is
//! ≈360 ms, where the home link reads ≈16 ms because it stops at the local
//! Cloudflare edge — the tunnel figure is the one that matters.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const TQ_HOST: &str = "tranquility.servers.eveonline.com";
pub const TQ_PORT: u16 = 26000;
pub const TIMEOUT: Duration = Duration::from_millis(1500);
pub const DNS_TTL: Duration = Duration::from_secs(300);

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
    tokio::net::lookup_host((TQ_HOST, TQ_PORT)).await.ok()?.find(SocketAddr::is_ipv4)
}

/// Probe once a second until `stop` is set. A failed resolution is a loss
/// and is retried on the next tick; a good one is reused for `DNS_TTL`.
pub async fn run(source: Ipv4Addr, latest: Latest, stop: Arc<AtomicBool>) {
    probe_loop(source, latest, stop, Duration::from_secs(1), resolve).await
}

async fn probe_loop<F, Fut>(source: Ipv4Addr, latest: Latest, stop: Arc<AtomicBool>, period: Duration, resolve: F)
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Option<SocketAddr>>,
{
    let mut tick = tokio::time::interval(period);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut target: Option<(SocketAddr, Instant)> = None;
    loop {
        tick.tick().await;
        if stop.load(Ordering::Relaxed) {
            return;
        }
        if target.is_none_or(|(_, at)| at.elapsed() >= DNS_TTL) {
            target = resolve().await.map(|a| (a, Instant::now())).or(target);
        }
        let rtt = match target {
            Some((addr, _)) => connect_rtt(source, addr, TIMEOUT).await,
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
/// cannot hold up the connect it is timing. `None` if the thread or
/// runtime cannot be made: the tunnel carries on without a ping.
pub fn spawn(source: Ipv4Addr, latest: Latest) -> Option<Probe> {
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let started = std::thread::Builder::new().name("tq-probe".into()).spawn(move || {
        match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt.block_on(run(source, latest, flag)),
            Err(e) => tracing::warn!("ping probe: runtime: {e}"),
        }
    });
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
        let th = std::thread::spawn(move || {
            block_on(probe_loop(Ipv4Addr::LOCALHOST, l, s, Duration::from_millis(50), || async move { Some(target) }))
        });
        std::thread::sleep(Duration::from_millis(400));
        assert!(latest.get().seq >= 2, "{:?}", latest.get());
        drop(probe);
        th.join().unwrap(); // returns only because the flag was seen
        let seq = latest.get().seq;
        std::thread::sleep(Duration::from_millis(200));
        assert_eq!(latest.get().seq, seq);
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
