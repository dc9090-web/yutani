# Nostromo popover — Phase 2 (ping) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The popover's PING · TQ row shows the real round trip to Tranquility through the tunnel. Its quality word is judged against the session's own normal.

**Architecture:**
- **Probe.** The root tunnel worker spawns a probe task. Once a second it times a TCP connect to `tranquility.servers.eveonline.com:26000` from the tunnel address, and publishes the latest result plus a sequence number in its status file.
- **Daemon.** It passes them through `TunnelStatus`.
- **Applet.** It feeds them into the existing `PingWindow`, whose quality rule becomes relative to the window's median.

**Tech Stack:** Rust 2024, tokio (`net`, `time` features — already enabled), serde.

**Spec:** `docs/superpowers/specs/2026-10-07-yutani-nostromo-design.md` § "Phase 2 — ping to Tranquility" (revised 2026-10-07).

## Global Constraints

- **Never run `cargo fmt`** (the repo is not rustfmt-clean).
- **Never `git add -A` / `git add .`.** Stage files by path; `design/` is untracked on purpose.
- **Checks after every task:** `cargo test --lib` and `cargo test --bin yutani-applet` pass, and `cargo clippy --all-targets` adds no new warnings in the files touched.
- **Commit messages** end with a blank line, then exactly `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **New wire fields are `serde(default)`:** an applet and a daemon of different versions must still parse each other.
- **Ping wire fields:**
  - `ping_us: Option<u32>`: whole microseconds, `None` = the probe was lost.
  - `ping_seq: u64`: 0 = no probe yet.
- **Probe:**
  - target `tranquility.servers.eveonline.com:26000`;
  - source = the tunnel's address with port 0;
  - timeout 1.5 s;
  - one probe per second;
  - DNS cached 5 min;
  - no data is ever sent.
- **Quality**, with `baseline` = the median of the window's successes and `latest` = the newest success:
  - **POOR** if loss > 5 %, or `latest > 1.6 × baseline + 20`, or no success at all;
  - else **DEGRADED** if loss > 0, or `latest > 1.25 × baseline + 10`, or `jitter > 0.15 × baseline + 5`;
  - else **NOMINAL**.
  - **IDLE** when the tunnel is down.

---

### Task 1: Ping fields on the wire

**Files:**
- Modify: `src/tunnel/status.rs` (`TunnelFile`, `TunnelStatus`, `assemble`, tests)
- Modify: struct literals that list every field: `src/tunnel/worker.rs:558` (set `ping_us: None, ping_seq: 0` for now — Task 2 fills them), `src/applet/icon.rs:132`, `src/applet/client.rs:197`, `src/applet/console.rs:376` (add `ping_us: None, ping_seq: 0`).

**Interfaces — Produces:**
- `TunnelFile { …, ping_us: Option<u32>, ping_seq: u64 }`
- `TunnelStatus { …, ping_us: Option<u32>, ping_seq: u64 }`

- [ ] **Step 1: Write the failing tests.** Add to `status.rs` tests:

```rust
    /// The worker's latest probe reaches the applet only while the link is
    /// up: a stale file must not keep a ping alive after a disconnect.
    #[test]
    fn the_ping_is_passed_on_only_while_connected() {
        let f = TunnelFile { ping_us: Some(361_500), ping_seq: 42, ..file() };
        let up = assemble(Some(&f), true, None, true, false, "London", 1_789_180_010);
        assert_eq!((up.ping_us, up.ping_seq), (Some(361_500), 42));
        let down = assemble(Some(&f), false, None, true, false, "London", 1_789_180_010);
        assert_eq!((down.ping_us, down.ping_seq), (None, 0));
    }

    /// Files and replies from before Phase 2 have no ping fields.
    #[test]
    fn ping_fields_default_when_absent() {
        let mut v = serde_json::to_value(file()).unwrap();
        v.as_object_mut().unwrap().remove("ping_us");
        v.as_object_mut().unwrap().remove("ping_seq");
        let f: TunnelFile = serde_json::from_value(v).unwrap();
        assert_eq!((f.ping_us, f.ping_seq), (None, 0));
        let mut s = serde_json::to_value(TunnelStatus::default()).unwrap();
        s.as_object_mut().unwrap().remove("ping_us");
        s.as_object_mut().unwrap().remove("ping_seq");
        let s: TunnelStatus = serde_json::from_value(s).unwrap();
        assert_eq!((s.ping_us, s.ping_seq), (None, 0));
    }
```

- [ ] **Step 2: Run them.** `cargo test --lib tunnel::status` → FAIL (no such fields).

- [ ] **Step 3: Implement.**
  - Add the two fields to both structs, after `exit_address`, each with a doc comment and `#[serde(default)]`:

```rust
    /// The latest round trip to Tranquility through the tunnel, in whole
    /// microseconds (`probe.rs`); `None` when that probe was lost.
    /// `serde(default)`: files and replies from before the probe.
    #[serde(default)]
    pub ping_us: Option<u32>,
    /// Incremented once per probe; 0 before the first. The applet takes a
    /// sample only when this advances.
    #[serde(default)]
    pub ping_seq: u64,
```

  - In `assemble`:

```rust
        ping_us: if connected { file.and_then(|f| f.ping_us) } else { None },
        ping_seq: if connected { file.map_or(0, |f| f.ping_seq) } else { 0 },
```

  - Add `ping_us: None, ping_seq: 0` to the struct literals listed under Files, so everything compiles.

- [ ] **Step 4: Run the tests.** `cargo test --lib` and `cargo test --bin yutani-applet` → PASS.
- [ ] **Step 5: Commit.** Message: `feat(tunnel): ping fields on the tunnel status wire`, with the trailer.

---

### Task 2: The probe in the tunnel worker

**Files:**
- Create: `src/tunnel/probe.rs`
- Modify: `src/tunnel/mod.rs` (`pub mod probe;`), `src/tunnel/worker.rs` (`serve` spawns the probe; `write_status` publishes it)

**Interfaces:**
- Consumes: Task 1's fields.
- Produces:

```rust
pub const TQ_HOST: &str = "tranquility.servers.eveonline.com";
pub const TQ_PORT: u16 = 26000;
pub const TIMEOUT: Duration = Duration::from_millis(1500);
pub const DNS_TTL: Duration = Duration::from_secs(300);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sample { pub rtt_us: Option<u32>, pub seq: u64 }
#[derive(Clone, Default)] pub struct Latest(Arc<Mutex<Sample>>);   // .get() -> Sample, .record(Option<u32>)
pub async fn connect_rtt(source: Ipv4Addr, target: SocketAddr, timeout: Duration) -> Option<u32>
pub async fn run(source: Ipv4Addr, latest: Latest)   // never returns; aborted with the task
```

- [ ] **Step 1: Write the failing tests** (`probe.rs`):

```rust
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
}
```

- [ ] **Step 2: Run them.** `cargo test --lib tunnel::probe` → FAIL.

- [ ] **Step 3: Implement `probe.rs`.**

```rust
//! The popover's PING · TQ (Nostromo spec, Phase 2): once a second the
//! root worker times a TCP connect to Tranquility *from the tunnel
//! address*, so the probe takes the same path EVE's traffic does — the
//! `from <address>` policy rule sends it down `yutani0`, as it does the
//! exit-address lookup. Nothing is sent; the socket is dropped as soon as
//! the handshake completes. Through a London exit from the far side of the world this is
//! ≈360 ms, where the home link reads ≈16 ms because it stops at the local
//! Cloudflare edge — the tunnel figure is the one that matters.

use std::net::{Ipv4Addr, SocketAddr};
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

/// Probe once a second for as long as the task lives. The worker aborts it
/// when the tunnel goes down. A failed resolution is a loss and is retried
/// on the next tick; a good one is reused for `DNS_TTL`.
pub async fn run(source: Ipv4Addr, latest: Latest) {
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut target: Option<(SocketAddr, Instant)> = None;
    loop {
        tick.tick().await;
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
```

  > `.or(target)` keeps the last good address when a re-resolution fails after the TTL. It is still retried on every tick, because `at` is old.

  Then wire it into `worker.rs`:
  - In `serve`, after `let mut exit = ExitIp::new();`:

```rust
    // PING · TQ: the probe runs beside this loop and is aborted with it.
    let latest = super::probe::Latest::default();
    let probe = tokio::spawn(super::probe::run(loaded.conf.address, latest.clone()));
    let _stop_probe = Teardown::new(move || probe.abort());
```

    Check `Teardown::new`'s signature (`impl FnOnce() + 'a`). If the borrow checker objects because `probe` is moved, use `scopeguard`-style `struct AbortOnDrop(tokio::task::JoinHandle<()>)` with `impl Drop` calling `self.0.abort()`.
  - `write_status` gains a `ping: super::probe::Sample` parameter and sets `ping_us: ping.rtt_us, ping_seq: ping.seq` in the `TunnelFile` literal. The tick passes `latest.get()`.
  - Add `pub mod probe;` to `src/tunnel/mod.rs`.

- [ ] **Step 4: Run the tests.** `cargo test --lib tunnel` → PASS (including the existing worker tests). Then `cargo build --release`.
- [ ] **Step 5: Live dry check (no root needed).** Use a one-off test run against the real target from the tunnel address. It is `#[ignore]`d, so CI never does network I/O:

```rust
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
```

  Run it. Expected: on the order of 360 000 µs.
- [ ] **Step 6: Commit.** Message: `feat(tunnel): probe Tranquility through the tunnel once a second`, with the trailer.

---

### Task 3: The applet takes the samples and judges them against the session's normal

**Files:**
- Modify: `src/applet/ping.rs` (relative quality; replace `Quality::of`)
- Modify: `src/bin/yutani-applet/app.rs` (feed the window; clear it when down or offline)

**Interfaces:**
- Consumes: `TunnelStatus::{ping_us, ping_seq}`.
- Produces: `Quality::judge(latest: f32, baseline: f32, jitter: f32, loss_pct: f32) -> Quality`.

- [ ] **Step 1: Write the failing tests.** In `ping.rs`, replace `quality_boundaries_follow_the_latest_success` with:

```rust
    /// Quality is judged against the session's own median, so 360 ms from
    /// the far side of the world is NOMINAL when that is normal (Nostromo spec, Phase 2).
    #[test]
    fn quality_is_relative_to_the_sessions_normal() {
        let q = Quality::judge;
        assert_eq!(q(360.0, 360.0, 3.0, 0.0), Quality::Nominal);
        assert_eq!(q(5.0, 4.0, 1.0, 0.0), Quality::Nominal, "the +10 ms floor keeps a tiny ping calm");
        assert_eq!(q(460.0, 360.0, 3.0, 0.0), Quality::Nominal, "1.25 × 360 + 10 = 460: equal is not above");
        assert_eq!(q(461.0, 360.0, 3.0, 0.0), Quality::Degraded);
        assert_eq!(q(360.0, 360.0, 60.0, 0.0), Quality::Degraded, "jitter above 0.15 × 360 + 5 = 59");
        assert_eq!(q(360.0, 360.0, 3.0, 2.9), Quality::Degraded, "any loss");
        assert_eq!(q(597.0, 360.0, 3.0, 0.0), Quality::Poor, "> 1.6 × 360 + 20 = 596");
        assert_eq!(q(360.0, 360.0, 3.0, 5.9), Quality::Poor, "loss above 5 %");
    }

    #[test]
    fn a_steady_long_ping_reads_nominal_and_a_lost_window_reads_poor() {
        let steady = window(&[Some(360.0), Some(361.0), Some(359.5), Some(360.4)]).summary(true);
        assert_eq!((steady.value.as_str(), steady.quality), ("360", Quality::Nominal));
        assert_eq!(window(&[None, None]).summary(true).quality, Quality::Poor, "all lost");
        assert_eq!(Quality::Nominal.word(), "NOMINAL");
    }
```

  In `app.rs` tests:

```rust
    /// Each new probe becomes one sample; a repeated sequence adds nothing;
    /// a dropped tunnel clears the window.
    #[test]
    fn ping_samples_follow_the_probe_sequence() {
        let mut applet = applet();
        let mut s = connected();
        s.tunnel.ping_us = Some(360_000);
        s.tunnel.ping_seq = 1;
        let _ = applet.update(Msg::Status(Ok(s.clone())));
        let _ = applet.update(Msg::Status(Ok(s.clone())));
        assert_eq!(applet.ping.summary(true).value, "360");
        s.tunnel.ping_us = None;
        s.tunnel.ping_seq = 2;
        let _ = applet.update(Msg::Status(Ok(s.clone())));
        assert!(applet.ping.summary(true).jitter_loss.ends_with("LOSS 50.0%"), "{}", applet.ping.summary(true).jitter_loss);
        s.tunnel.connected = false;
        let _ = applet.update(Msg::Status(Ok(s)));
        assert_eq!(applet.ping.summary(true).quality, yutani::applet::ping::Quality::Idle, "cleared: no samples");
    }
```

- [ ] **Step 2: Run them** → FAIL.

- [ ] **Step 3: Implement.**
  - In `ping.rs`, replace `fn of` with:

```rust
    /// The quality word for the newest success `latest`, judged against the
    /// window's own median `baseline`, with the window's jitter and loss
    /// (Nostromo spec, Phase 2). The `+ n` floors keep a few-ms ping from
    /// flapping on noise.
    pub fn judge(latest: f32, baseline: f32, jitter: f32, loss_pct: f32) -> Self {
        if loss_pct > 5.0 || latest > 1.6 * baseline + 20.0 {
            Quality::Poor
        } else if loss_pct > 0.0 || latest > 1.25 * baseline + 10.0 || jitter > 0.15 * baseline + 5.0 {
            Quality::Degraded
        } else {
            Quality::Nominal
        }
    }
```

  - In `summary`, compute the median of `ok`: sort a copy; for an even count take the mean of the two middle values. Then:

```rust
            quality: latest.map_or(Quality::Poor, |l| Quality::judge(l, median, jitter, loss)),
```

  - Update the module doc comment: samples are fed by the daemon's probe (Phase 2), and quality is relative.
  - In `app.rs`, `Msg::Status(Ok(..))`, after `let live = status.tunnel.connected;`:

```rust
                if live {
                    let t = &status.tunnel;
                    if t.ping_seq > 0 {
                        self.ping.push_seq(t.ping_seq, t.ping_us.map(|us| us as f32 / 1000.0));
                    }
                } else {
                    self.ping.clear();
                }
```

  - And in `Msg::Status(Err(IpcError::Offline))`: `self.ping.clear();`.

- [ ] **Step 4: Run the tests** → PASS. Check clippy.
- [ ] **Step 5: Commit.** Message: `feat(applet): ping samples from the probe, quality relative to the session`, with the trailer.

---

### Task 4: Install and look (controller)

- [ ] Build and install the package:

```bash
cd packaging && makepkg -f --noconfirm
pkexec pacman -U --noconfirm <ABSOLUTE path to the new .pkg.tar.zst>
```

  Commit the PKGBUILD pkgver bump afterwards.
- [ ] **Ask Daniel first:** the probe runs in the root tunnel worker, so the tunnel unit must restart. That drops the tunnel for a few seconds; do it with no EVE client running. `systemctl --no-ask-password restart yutani-tunnel.service`. The polkit rule allows this.
- [ ] Restart the daemon (`systemctl --user restart yutani`) and the panel (`pkill -x cosmic-panel`).
- [ ] Check `cat /run/yutani/tunnel.json`: `ping_us` is ≈360000 and `ping_seq` increases each second. `yutani status` shows the same.
- [ ] Have Daniel open the popover, then take a screenshot. Expected: PING · TQ reads ≈360 MS and NOMINAL, the sparkline is filled, and MIN/AVG/MAX and JIT/LOSS show numbers.
