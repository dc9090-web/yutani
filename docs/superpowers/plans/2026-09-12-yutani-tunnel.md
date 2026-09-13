# EVE-only WireGuard Tunnel Implementation Plan (plan A)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** EVE's traffic (client + launcher, including DNS) goes through a WireGuard tunnel to London and nothing else does; while the tunnel is meant to be up but broken EVE has no network; `yutani tunnel install|connect|disconnect|status`, `yutani launch -- %command%`, auto-adoption of running EVE processes, and an IPC `status` request for the upcoming applet.

**Architecture:** A root worker (`yutani tunnel run`, ExecStart of `yutani-tunnel.service`) creates `yutani0`, a policy-routing table keyed on fwmark `0x59`, and an nftables table that marks / DNATs-DNS / kill-switches packets from sockets in the `yutani-eve.slice` cgroup, then publishes `/run/yutani/tunnel.json` once a second. A one-time `pkexec` install writes the conf (root-only), the unit and a polkit rule so the user can start/stop that one unit without a password. The daemon adopts EVE processes into the slice (`StartTransientUnit` on the user bus) and answers `status` / `tunnel connect|disconnect` over IPC; `yutani launch` wraps Steam's `%command%` in a scope under the slice.

**Tech Stack:** Rust 2024, `tokio` (add `rt`, `signal`, `time`), `serde_json` (new direct dep, already in lock), iproute2 / wireguard-tools / nftables / systemd binaries at runtime.

**Spec:** `docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md`

## Global Constraints

- Constants (verbatim from the spec): interface `yutani0`; fwmark `0x59`; routing table `51820`; slice `yutani-eve.slice`; cgroup path `user.slice/user-<uid>.slice/user@<uid>.service/yutani.slice/yutani-eve.slice` (nft `socket cgroupv2 level 5`; systemd nests `yutani-eve.slice` under `yutani.slice` because of the dash, hence five components); conf `/etc/yutani/tunnel.conf` (0600 root); status `/run/yutani/tunnel.json` (0644, atomic rename); unit `/etc/systemd/system/yutani-tunnel.service`; polkit rule `/etc/polkit-1/rules.d/50-yutani-tunnel.rules`.
- No shell scripts: the worker execs `ip`, `wg`, `nft`, `sysctl` with generated argv; any failure during *up* runs the full *down* and exits non-zero with the failing command and its stderr in the log.
- The private key appears only in `/etc/yutani/tunnel.conf` and the transient 0600 `wg setconf` file; never in logs, `tunnel.json`, IPC replies or dry-run output (dry-run prints `PrivateKey = <redacted>`).
- New direct deps: `serde_json = "1"`; `tokio` features become `["net", "io-util", "sync", "rt", "signal", "time", "macros"]` (`macros` for the worker's `tokio::select!`). No new `[[package]]` entries in `Cargo.lock`.
- IPC protocol additions: requests `status`, `tunnel connect`, `tunnel disconnect`; reply form `ok <json>` for `status` (`Response::OkData(String)`).
- `yutani launch` must never prevent the game from starting: if `systemd-run` is missing or fails, run the command directly and print a warning to stderr.
- Every commit: `cargo build -q` warning-free for new code (pre-existing: `Config::save/save_to`), `cargo test -q` green.
- Commit trailer on every commit:
  ```
  Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH
  ```
- Privileged steps (`install`, `connect`, the worker) cannot be executed by an agent (password prompt); Task 6 is Daniel's hands-on acceptance. Every other task must be verifiable without root: unit tests plus `--dry-run` output.

---

## File Structure

| File | Responsibility |
|---|---|
| `src/tunnel/mod.rs` (new) | `pub mod conf; pub mod rules; pub mod status; pub mod worker; pub mod install; pub mod control;` + shared constants. |
| `src/tunnel/conf.rs` (new) | wg-quick conf parse/validate; wg-native rendering; label derivation. Pure. |
| `src/tunnel/rules.rs` (new) | nft ruleset text; `ip`/`sysctl`/`wg` argv lists for up/down; cgroup path. Pure. |
| `src/tunnel/status.rs` (new) | `TunnelFile` (what the worker writes), `wg show … dump` parsing, `TunnelStatus`/`Status` (IPC JSON), sysfs readers. |
| `src/tunnel/worker.rs` (new) | `yutani tunnel run`: execute up, loop, down on SIGTERM. |
| `src/tunnel/install.rs` (new) | unit/polkit texts; `install-root`/`uninstall-root`; `pkexec` wrappers; `--dry-run`. |
| `src/tunnel/control.rs` (new) | `connect`/`disconnect` via `systemctl`; `installed()`; `current_status()` assembly. |
| `src/launch.rs` (new) | `yutani launch -- cmd…` via `systemd-run --scope`. |
| `src/adopt.rs` (new) | `/proc` scan classification + `busctl StartTransientUnit` argv; daemon subscription. |
| `src/ipc.rs` | `Request::{Status, TunnelConnect, TunnelDisconnect}`, `Response::OkData`. |
| `src/cli.rs` | print `OkData` payload. |
| `src/ui/mod.rs` | handle the three new requests; adoption subscription. |
| `src/model/config.rs` | `TunnelConfig { location, auto_adopt, adopt_processes }`. |
| `src/main.rs` | `tunnel …` and `launch` subcommands. |
| `docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md` | status notes after acceptance. |

Test-count baseline: 75 after plan 4.

---

### Task 1: wg-quick conf parsing (`src/tunnel/conf.rs`) and the `tunnel` config section

**Files:**
- Create: `src/tunnel/mod.rs`, `src/tunnel/conf.rs`
- Modify: `src/model/config.rs`, `src/main.rs` (`mod tunnel;`), `Cargo.toml`

**Interfaces (produced):**
```rust
// src/tunnel/mod.rs
pub const IFACE: &str = "yutani0";
pub const FWMARK: u32 = 0x59;
pub const TABLE: u32 = 51820;
pub const SLICE: &str = "yutani-eve.slice";
pub const CONF_PATH: &str = "/etc/yutani/tunnel.conf";
pub const STATUS_PATH: &str = "/run/yutani/tunnel.json";
pub const UNIT_NAME: &str = "yutani-tunnel.service";
pub const UNIT_PATH: &str = "/etc/systemd/system/yutani-tunnel.service";
pub const POLKIT_PATH: &str = "/etc/polkit-1/rules.d/50-yutani-tunnel.rules";
// src/tunnel/conf.rs
pub struct WgConf { pub private_key: String, pub address: std::net::Ipv4Addr, pub prefix_len: u8, pub dns: Option<std::net::Ipv4Addr>, pub mtu: Option<u32>, pub peer_public_key: String, pub preshared_key: Option<String>, pub allowed_ips: Vec<String>, pub endpoint: String, pub keepalive: Option<u32>, pub label: String }
impl WgConf { pub fn parse(text: &str, fallback_label: &str) -> Result<WgConf, String>; pub fn wg_native(&self) -> String; pub fn redacted(&self) -> String; }
// src/model/config.rs
pub struct TunnelConfig { pub location: String, pub auto_adopt: bool, pub adopt_processes: Vec<String> }  // Config gains `pub tunnel: TunnelConfig`
```

- [ ] **Step 1: Dependencies and module skeleton**

`Cargo.toml`: change the tokio line to `tokio = { version = "1", features = ["net", "io-util", "sync", "rt", "signal", "time"] }` and add `serde_json = "1"` after it. `cargo build -q` must add no `[[package]]` entries (`git diff --stat Cargo.lock` → only the `yutani` dependency list).

Create `src/tunnel/mod.rs`:

```rust
//! EVE-only WireGuard tunnel: root worker, install, control, status.
//! See docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md.

pub mod conf;

pub const IFACE: &str = "yutani0";
pub const FWMARK: u32 = 0x59;
pub const TABLE: u32 = 51820;
pub const SLICE: &str = "yutani-eve.slice";
pub const CONF_PATH: &str = "/etc/yutani/tunnel.conf";
pub const STATUS_PATH: &str = "/run/yutani/tunnel.json";
pub const UNIT_NAME: &str = "yutani-tunnel.service";
pub const UNIT_PATH: &str = "/etc/systemd/system/yutani-tunnel.service";
pub const POLKIT_PATH: &str = "/etc/polkit-1/rules.d/50-yutani-tunnel.rules";
```

Add `mod tunnel;` to `src/main.rs` (after `mod shortcuts;`).

- [ ] **Step 2: Config section — failing tests** (append to `mod tests` in `src/model/config.rs`)

```rust
    #[test]
    fn tunnel_defaults_and_parse() {
        let c = Config::default();
        assert_eq!(c.tunnel.location, "London");
        assert!(c.tunnel.auto_adopt);
        assert_eq!(c.tunnel.adopt_processes, vec!["exefile.exe", "eve-online.exe", "evelauncher.exe"]);
        let c: Config = ron::from_str("(tunnel: (location: \"Amsterdam\", auto_adopt: false))").unwrap();
        assert_eq!(c.tunnel.location, "Amsterdam");
        assert!(!c.tunnel.auto_adopt);
        assert_eq!(c.tunnel.adopt_processes.len(), 3);
    }
```

- [ ] **Step 3: Config section — implement**

In `src/model/config.rs`, after `ShortcutsConfig`:

```rust
/// EVE-only WireGuard tunnel (spec 2026-09-12-yutani-tunnel-design.md).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TunnelConfig {
    /// Label shown for the exit ("London").
    pub location: String,
    /// Move running EVE processes into the tunnel cgroup automatically.
    pub auto_adopt: bool,
    /// Executable names (case-insensitive) that count as EVE.
    pub adopt_processes: Vec<String>,
}

impl Default for TunnelConfig {
    fn default() -> Self {
        Self {
            location: "London".into(),
            auto_adopt: true,
            adopt_processes: vec!["exefile.exe".into(), "eve-online.exe".into(), "evelauncher.exe".into()],
        }
    }
}
```

Add `pub tunnel: TunnelConfig,` as the last `Config` field (doc `/// EVE-only WireGuard tunnel.`) and `tunnel: TunnelConfig::default(),` in `Default`. Run `cargo test -q config:: 2>&1 | tail -2` → green.

- [ ] **Step 4: Conf parser — failing tests** (in `src/tunnel/conf.rs`, write the tests module first)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const PROTON: &str = "[Interface]\n# Key for EVE\n# Bouncing = 12\nPrivateKey = cHJpdmF0ZWtleXByaXZhdGVrZXlwcml2YXRla2V5cHI=\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n\n[Peer]\n# UK#455\nPublicKey = cGVlcnB1YmxpY2tleXBlZXJwdWJsaWNrZXlwZWVycHU=\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 198.51.100.10:51820\n\nPersistentKeepalive = 25\n";

    #[test]
    fn parses_the_proton_layout() {
        let c = WgConf::parse(PROTON, "EVE-UK-455").unwrap();
        assert_eq!(c.private_key, "cHJpdmF0ZWtleXByaXZhdGVrZXlwcml2YXRla2V5cHI=");
        assert_eq!(c.address, "10.2.0.2".parse::<std::net::Ipv4Addr>().unwrap());
        assert_eq!(c.prefix_len, 32);
        assert_eq!(c.dns, Some("10.2.0.1".parse().unwrap()));
        assert_eq!(c.mtu, None);
        assert_eq!(c.peer_public_key, "cGVlcnB1YmxpY2tleXBlZXJwdWJsaWNrZXlwZWVycHU=");
        assert_eq!(c.allowed_ips, vec!["0.0.0.0/0", "::/0"]);
        assert_eq!(c.endpoint, "198.51.100.10:51820");
        assert_eq!(c.keepalive, Some(25));
        // Label: the first comment in [Peer] wins over the fallback.
        assert_eq!(c.label, "UK#455");
    }

    #[test]
    fn label_falls_back_to_the_given_name() {
        let text = PROTON.replace("# UK#455\n", "");
        assert_eq!(WgConf::parse(&text, "EVE-UK-455").unwrap().label, "EVE-UK-455");
    }

    #[test]
    fn address_takes_the_first_ipv4_and_mtu_is_honoured() {
        let text = PROTON.replace("Address = 10.2.0.2/32", "Address = fd00::2/128, 10.9.8.7/24\nMTU = 1380");
        let c = WgConf::parse(&text, "x").unwrap();
        assert_eq!(c.address.to_string(), "10.9.8.7");
        assert_eq!(c.prefix_len, 24);
        assert_eq!(c.mtu, Some(1380));
    }

    #[test]
    fn rejects_incomplete_confs_with_a_reason() {
        let no_key = PROTON.replace("PrivateKey", "PrivateKeyX");
        assert!(WgConf::parse(&no_key, "x").unwrap_err().contains("PrivateKey"));
        let no_v4 = PROTON.replace("Address = 10.2.0.2/32", "Address = fd00::2/128");
        assert!(WgConf::parse(&no_v4, "x").unwrap_err().contains("IPv4"));
        let two_peers = format!("{PROTON}\n[Peer]\nPublicKey = A=\nEndpoint = 1.2.3.4:1\nAllowedIPs = 0.0.0.0/0\n");
        assert!(WgConf::parse(&two_peers, "x").unwrap_err().contains("one [Peer]"));
        let no_endpoint = PROTON.replace("Endpoint = 198.51.100.10:51820\n", "");
        assert!(WgConf::parse(&no_endpoint, "x").unwrap_err().contains("Endpoint"));
        assert!(WgConf::parse("", "x").is_err());
        assert!(WgConf::parse("[Interface]\nPrivateKey = a\nAddress = 10.0.0.1/32\n[Peer]\nPublicKey = b\nEndpoint = h:1\nAllowedIPs = 0.0.0.0/0\nBogus = 1\n", "x").unwrap_err().contains("Bogus"));
    }

    #[test]
    fn wg_native_keeps_only_what_wg_setconf_accepts() {
        let c = WgConf::parse(PROTON, "x").unwrap();
        assert_eq!(
            c.wg_native(),
            "[Interface]\nPrivateKey = cHJpdmF0ZWtleXByaXZhdGVrZXlwcml2YXRla2V5cHI=\n\n[Peer]\nPublicKey = cGVlcnB1YmxpY2tleXBlZXJwdWJsaWNrZXlwZWVycHU=\nAllowedIPs = 0.0.0.0/0, ::/0\nEndpoint = 198.51.100.10:51820\nPersistentKeepalive = 25\n"
        );
        let with_psk = PROTON.replace("PublicKey =", "PresharedKey = cHNr\nPublicKey =");
        assert!(WgConf::parse(&with_psk, "x").unwrap().wg_native().contains("PresharedKey = cHNr\n"));
    }

    #[test]
    fn redacted_never_contains_secrets() {
        let with_psk = PROTON.replace("PublicKey =", "PresharedKey = cHNr\nPublicKey =");
        let r = WgConf::parse(&with_psk, "x").unwrap().redacted();
        assert!(!r.contains("cHJpdmF0ZWtleXByaXZhdGVrZXlwcml2YXRla2V5cHI="));
        assert!(!r.contains("cHNr"));
        assert!(r.contains("PrivateKey = <redacted>"));
        assert!(r.contains("PresharedKey = <redacted>"));
        assert!(r.contains("Endpoint = 198.51.100.10:51820"));
    }
}
```

- [ ] **Step 5: Conf parser — implement** (above the tests in `src/tunnel/conf.rs`)

```rust
//! wg-quick style configuration: parse, validate, and render the subset
//! `wg setconf` accepts. `Address`, `DNS` and `MTU` are ours to apply.

use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WgConf {
    pub private_key: String,
    pub address: Ipv4Addr,
    pub prefix_len: u8,
    pub dns: Option<Ipv4Addr>,
    pub mtu: Option<u32>,
    pub peer_public_key: String,
    pub preshared_key: Option<String>,
    pub allowed_ips: Vec<String>,
    pub endpoint: String,
    pub keepalive: Option<u32>,
    /// Human label for the exit: first `# comment` in `[Peer]`, else `fallback_label`.
    pub label: String,
}

const INTERFACE_KEYS: &[&str] = &["PrivateKey", "Address", "DNS", "MTU", "ListenPort", "Table", "PreUp", "PostUp", "PreDown", "PostDown", "SaveConfig", "FwMark"];
const PEER_KEYS: &[&str] = &["PublicKey", "PresharedKey", "AllowedIPs", "Endpoint", "PersistentKeepalive"];

#[derive(Default)]
struct Section {
    name: String,
    entries: Vec<(String, String)>,
    first_comment: Option<String>,
}

fn sections(text: &str) -> Result<Vec<Section>, String> {
    let mut out: Vec<Section> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            if let Some(s) = out.last_mut()
                && s.first_comment.is_none()
                && !comment.contains('=')
            {
                s.first_comment = Some(comment.trim().to_string());
            }
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            out.push(Section { name: line[1..line.len() - 1].trim().to_string(), ..Default::default() });
            continue;
        }
        let Some((k, v)) = line.split_once('=') else {
            return Err(format!("line {}: expected `Key = value`, got {line:?}", i + 1));
        };
        let Some(s) = out.last_mut() else {
            return Err(format!("line {}: `{}` before any [section]", i + 1, k.trim()));
        };
        s.entries.push((k.trim().to_string(), v.trim().to_string()));
    }
    Ok(out)
}

fn get<'a>(s: &'a Section, key: &str) -> Option<&'a str> {
    s.entries.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.as_str())
}

impl WgConf {
    pub fn parse(text: &str, fallback_label: &str) -> Result<WgConf, String> {
        let secs = sections(text)?;
        let ifaces: Vec<&Section> = secs.iter().filter(|s| s.name.eq_ignore_ascii_case("Interface")).collect();
        let peers: Vec<&Section> = secs.iter().filter(|s| s.name.eq_ignore_ascii_case("Peer")).collect();
        let [iface] = ifaces[..] else { return Err(format!("expected exactly one [Interface], found {}", ifaces.len())) };
        let [peer] = peers[..] else { return Err(format!("expected exactly one [Peer], found {}", peers.len())) };
        for (k, _) in &iface.entries {
            if !INTERFACE_KEYS.iter().any(|known| known.eq_ignore_ascii_case(k)) {
                return Err(format!("unknown [Interface] key {k}"));
            }
        }
        for (k, _) in &peer.entries {
            if !PEER_KEYS.iter().any(|known| known.eq_ignore_ascii_case(k)) {
                return Err(format!("unknown [Peer] key {k}"));
            }
        }
        let private_key = get(iface, "PrivateKey").ok_or("[Interface] needs PrivateKey")?.to_string();
        let (address, prefix_len) = get(iface, "Address")
            .ok_or("[Interface] needs Address")?
            .split(',')
            .map(str::trim)
            .find_map(|a| {
                let (ip, len) = a.split_once('/').unwrap_or((a, "32"));
                Some((ip.parse::<Ipv4Addr>().ok()?, len.parse::<u8>().ok()?))
            })
            .ok_or("[Interface] Address has no IPv4 entry")?;
        let dns = match get(iface, "DNS") {
            Some(v) => v.split(',').map(str::trim).find_map(|d| d.parse::<Ipv4Addr>().ok()),
            None => None,
        };
        let mtu = get(iface, "MTU").map(|m| m.parse::<u32>().map_err(|_| format!("bad MTU {m:?}"))).transpose()?;
        let peer_public_key = get(peer, "PublicKey").ok_or("[Peer] needs PublicKey")?.to_string();
        let preshared_key = get(peer, "PresharedKey").map(str::to_string);
        let allowed_ips: Vec<String> =
            get(peer, "AllowedIPs").ok_or("[Peer] needs AllowedIPs")?.split(',').map(|s| s.trim().to_string()).collect();
        let endpoint = get(peer, "Endpoint").ok_or("[Peer] needs Endpoint")?.to_string();
        let keepalive = get(peer, "PersistentKeepalive")
            .map(|k| k.parse::<u32>().map_err(|_| format!("bad PersistentKeepalive {k:?}")))
            .transpose()?;
        let label = peer.first_comment.clone().filter(|c| !c.is_empty()).unwrap_or_else(|| fallback_label.to_string());
        Ok(WgConf { private_key, address, prefix_len, dns, mtu, peer_public_key, preshared_key, allowed_ips, endpoint, keepalive, label })
    }

    /// Exactly what `wg setconf` accepts.
    pub fn wg_native(&self) -> String {
        let mut s = format!("[Interface]\nPrivateKey = {}\n\n[Peer]\nPublicKey = {}\n", self.private_key, self.peer_public_key);
        if let Some(psk) = &self.preshared_key {
            s.push_str(&format!("PresharedKey = {psk}\n"));
        }
        s.push_str(&format!("AllowedIPs = {}\nEndpoint = {}\n", self.allowed_ips.join(", "), self.endpoint));
        if let Some(k) = self.keepalive {
            s.push_str(&format!("PersistentKeepalive = {k}\n"));
        }
        s
    }

    /// For logs and `--dry-run`: the conf with secrets replaced.
    pub fn redacted(&self) -> String {
        let mut c = self.clone();
        c.private_key = "<redacted>".into();
        if c.preshared_key.is_some() {
            c.preshared_key = Some("<redacted>".into());
        }
        let mut s = c.wg_native();
        s.insert_str(s.find("\n\n[Peer]").unwrap_or(s.len()), &format!("\nAddress = {}/{}{}{}", c.address, c.prefix_len,
            c.dns.map(|d| format!("\nDNS = {d}")).unwrap_or_default(),
            c.mtu.map(|m| format!("\nMTU = {m}")).unwrap_or_default()));
        s
    }
}
```

- [ ] **Step 6: Tests, commit**

Run: `cargo test -q tunnel::conf 2>&1 | tail -2` → `6 passed`; `cargo test -q 2>&1 | grep 'test result'` → `82 passed`. Build warnings: `WgConf` items unused outside tests until Task 3 — acceptable transitional state; list them in the report.

```bash
git add Cargo.toml Cargo.lock src/tunnel/mod.rs src/tunnel/conf.rs src/model/config.rs src/main.rs
git commit -m "feat(tunnel): wg-quick conf parser/validator and tunnel config section

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 2: Rules and status types (`src/tunnel/rules.rs`, `src/tunnel/status.rs`) — pure

**Files:**
- Create: `src/tunnel/rules.rs`, `src/tunnel/status.rs`
- Modify: `src/tunnel/mod.rs` (`pub mod rules; pub mod status;`)

**Interfaces (produced):**
```rust
// rules.rs
pub fn cgroup_path(uid: u32) -> String;                              // "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice"
pub fn nft_ruleset(uid: u32, dns: Option<Ipv4Addr>) -> String;
pub fn up_commands(conf: &WgConf, wg_conf_path: &str) -> Vec<Vec<String>>;   // argv lists, in order
pub fn down_commands() -> Vec<Vec<String>>;
// status.rs
#[derive(Serialize, Deserialize)] pub struct TunnelFile { pub up: bool, pub iface: String, pub address: String, pub endpoint: String, pub latest_handshake_unix: u64, pub rx_bytes: u64, pub tx_bytes: u64, pub since_unix: u64 }
pub fn parse_wg_dump(dump: &str) -> Option<(String /*endpoint*/, u64 /*handshake*/, u64 /*rx*/, u64 /*tx*/)>;
#[derive(Serialize, Deserialize)] pub struct TunnelStatus { pub installed: bool, pub connected: bool, pub iface: String, pub location: String, pub address: Option<String>, pub endpoint: Option<String>, pub handshake_age_s: Option<u64>, pub rx_bytes: u64, pub tx_bytes: u64 }
#[derive(Serialize, Deserialize)] pub struct ClientStatus { pub name: String, pub active: bool }
#[derive(Serialize, Deserialize)] pub struct Status { pub clients: Vec<ClientStatus>, pub hidden: bool, pub tunnel: TunnelStatus }
pub fn assemble(file: Option<&TunnelFile>, iface_present: bool, sysfs: Option<(u64, u64)>, installed: bool, location: &str, now_unix: u64) -> TunnelStatus;
```

- [ ] **Step 1: Failing tests for rules** (`src/tunnel/rules.rs`, tests module)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::conf::WgConf;

    fn conf() -> WgConf {
        WgConf::parse(
            "[Interface]\nPrivateKey = k=\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n",
            "x",
        )
        .unwrap()
    }

    #[test]
    fn cgroup_path_uses_the_uid_and_slice() {
        assert_eq!(cgroup_path(1000), "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice");
    }

    #[test]
    fn nft_ruleset_marks_dnats_dns_and_kill_switches() {
        let r = nft_ruleset(1000, Some("10.2.0.1".parse().unwrap()));
        assert!(r.starts_with("table inet yutani {\n"));
        assert!(r.contains("type route hook output priority mangle; policy accept;"));
        assert!(r.contains(r#"socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" meta mark set 0x59"#));
        assert!(r.contains("type nat hook output priority dstnat; policy accept;"));
        assert!(r.contains(r#"socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" meta l4proto { tcp, udp } th dport 53 dnat ip to 10.2.0.1"#));
        assert!(r.contains("type filter hook output priority filter; policy accept;"));
        assert!(r.contains(r#"socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" oifname "lo" accept"#));
        assert!(r.contains(r#"socket cgroupv2 level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice" oifname != "yutani0" counter drop"#));
    }

    #[test]
    fn nft_ruleset_without_dns_has_no_nat_chain() {
        let r = nft_ruleset(1000, None);
        assert!(!r.contains("dnat"));
        assert!(r.contains("meta mark set 0x59"));
    }

    #[test]
    fn up_commands_in_spec_order() {
        let cmds = up_commands(&conf(), "/run/yutani/wg.conf");
        let joined: Vec<String> = cmds.iter().map(|c| c.join(" ")).collect();
        assert_eq!(
            joined,
            vec![
                "ip link add yutani0 type wireguard",
                "wg setconf yutani0 /run/yutani/wg.conf",
                "ip address add 10.2.0.2/32 dev yutani0",
                "ip link set yutani0 mtu 1420 up",
                "sysctl -q -w net.ipv4.conf.yutani0.rp_filter=2",
                "ip route add default dev yutani0 table 51820",
                "ip rule add fwmark 0x59 lookup 51820 priority 1000",
                "ip rule add from 10.2.0.2 lookup 51820 priority 1001",
            ]
        );
    }

    #[test]
    fn up_commands_honour_conf_mtu() {
        let mut c = conf();
        c.mtu = Some(1380);
        assert!(up_commands(&c, "/x").iter().any(|c| c.join(" ") == "ip link set yutani0 mtu 1380 up"));
    }

    #[test]
    fn down_commands_reverse_everything_and_are_idempotent_shaped() {
        let joined: Vec<String> = down_commands().iter().map(|c| c.join(" ")).collect();
        assert_eq!(
            joined,
            vec![
                "nft delete table inet yutani",
                "ip rule del fwmark 0x59 lookup 51820 priority 1000",
                "ip rule del priority 1001",
                "ip route flush table 51820",
                "ip link del yutani0",
            ]
        );
    }
}
```

- [ ] **Step 2: Implement rules**

```rust
//! Pure generators for everything the root worker executes: the nftables
//! ruleset and the `ip`/`wg`/`sysctl` argv lists. Keeping these pure means
//! the exact commands are unit-tested and printable by `--dry-run`.

use std::net::Ipv4Addr;

use super::conf::WgConf;
use super::{FWMARK, IFACE, SLICE, TABLE};

pub fn cgroup_path(uid: u32) -> String {
    format!("user.slice/user-{uid}.slice/user@{uid}.service/{SLICE}")
}

fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|s| s.to_string()).collect()
}

/// `socket cgroupv2 level 5 "<path>"` — level = number of path components.
fn cgroup_match(uid: u32) -> String {
    format!(r#"socket cgroupv2 level 5 "{}""#, cgroup_path(uid))
}

pub fn nft_ruleset(uid: u32, dns: Option<Ipv4Addr>) -> String {
    let m = cgroup_match(uid);
    let mut s = String::from("table inet yutani {\n");
    s.push_str("    chain setmark {\n        type route hook output priority mangle; policy accept;\n");
    s.push_str(&format!("        {m} meta mark set {FWMARK:#x}\n    }}\n"));
    if let Some(dns) = dns {
        s.push_str("    chain dns {\n        type nat hook output priority dstnat; policy accept;\n");
        s.push_str(&format!("        {m} meta l4proto {{ tcp, udp }} th dport 53 dnat ip to {dns}\n    }}\n"));
    }
    s.push_str("    chain killswitch {\n        type filter hook output priority filter; policy accept;\n");
    s.push_str(&format!("        {m} oifname \"lo\" accept\n"));
    s.push_str(&format!("        {m} oifname != \"{IFACE}\" counter drop\n    }}\n"));
    s.push_str("}\n");
    s
}

/// Everything before loading the nft ruleset, in order. `wg_conf_path` is
/// the transient 0600 file holding `WgConf::wg_native()`.
pub fn up_commands(conf: &WgConf, wg_conf_path: &str) -> Vec<Vec<String>> {
    let mtu = conf.mtu.unwrap_or(1420).to_string();
    let addr = format!("{}/{}", conf.address, conf.prefix_len);
    let from = conf.address.to_string();
    let table = TABLE.to_string();
    let mark = format!("{FWMARK:#x}");
    vec![
        argv(&["ip", "link", "add", IFACE, "type", "wireguard"]),
        argv(&["wg", "setconf", IFACE, wg_conf_path]),
        argv(&["ip", "address", "add", &addr, "dev", IFACE]),
        argv(&["ip", "link", "set", IFACE, "mtu", &mtu, "up"]),
        argv(&["sysctl", "-q", "-w", &format!("net.ipv4.conf.{IFACE}.rp_filter=2")]),
        argv(&["ip", "route", "add", "default", "dev", IFACE, "table", &table]),
        argv(&["ip", "rule", "add", "fwmark", &mark, "lookup", &table, "priority", "1000"]),
        argv(&["ip", "rule", "add", "from", &from, "lookup", &table, "priority", "1001"]),
    ]
}

/// Teardown. Every command is attempted regardless of earlier failures.
pub fn down_commands() -> Vec<Vec<String>> {
    let table = TABLE.to_string();
    let mark = format!("{FWMARK:#x}");
    vec![
        argv(&["nft", "delete", "table", "inet", "yutani"]),
        argv(&["ip", "rule", "del", "fwmark", &mark, "lookup", &table, "priority", "1000"]),
        argv(&["ip", "rule", "del", "priority", "1001"]),
        argv(&["ip", "route", "flush", "table", &table]),
        argv(&["ip", "link", "del", IFACE]),
    ]
}
```

- [ ] **Step 3: Failing tests for status** (`src/tunnel/status.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const DUMP: &str = "cHJpdg==\tcHVi\t51820\toff\ncGVlcnB1YmxpY2tleXBlZXJwdWJsaWNrZXlwZWVycHU=\t(none)\t198.51.100.10:51820\t0.0.0.0/0,::/0\t1789180000\t413100000\t2790000000\t25\n";

    #[test]
    fn parses_the_peer_line_of_wg_dump() {
        assert_eq!(parse_wg_dump(DUMP), Some(("198.51.100.10:51820".into(), 1789180000, 413100000, 2790000000)));
        assert_eq!(parse_wg_dump("cHJpdg==\tcHVi\t51820\toff\n"), None);
        assert_eq!(parse_wg_dump(""), None);
    }

    fn file() -> TunnelFile {
        TunnelFile { up: true, iface: "yutani0".into(), address: "10.2.0.2".into(), endpoint: "198.51.100.10:51820".into(), latest_handshake_unix: 1000, rx_bytes: 5, tx_bytes: 7, since_unix: 900 }
    }

    #[test]
    fn assemble_connected_prefers_sysfs_counters() {
        let s = assemble(Some(&file()), true, Some((50, 70)), true, "London", 1021);
        assert!(s.installed && s.connected);
        assert_eq!(s.handshake_age_s, Some(21));
        assert_eq!((s.rx_bytes, s.tx_bytes), (50, 70));
        assert_eq!(s.address.as_deref(), Some("10.2.0.2"));
        assert_eq!(s.endpoint.as_deref(), Some("198.51.100.10:51820"));
        assert_eq!(s.location, "London");
        assert_eq!(s.iface, "yutani0");
    }

    #[test]
    fn assemble_without_iface_is_disconnected_even_if_file_says_up() {
        let s = assemble(Some(&file()), false, None, true, "London", 1021);
        assert!(!s.connected);
        assert_eq!(s.handshake_age_s, None);
        assert_eq!((s.rx_bytes, s.tx_bytes), (5, 7)); // last known totals stay
    }

    #[test]
    fn assemble_no_handshake_yet_and_not_installed() {
        let mut f = file();
        f.latest_handshake_unix = 0;
        let s = assemble(Some(&f), true, None, true, "London", 1021);
        assert!(s.connected);
        assert_eq!(s.handshake_age_s, None);
        let s = assemble(None, false, None, false, "London", 1);
        assert!(!s.installed && !s.connected && s.address.is_none());
    }

    #[test]
    fn status_json_round_trips() {
        let st = Status {
            clients: vec![ClientStatus { name: "KestrelVance".into(), active: true }],
            hidden: false,
            tunnel: assemble(Some(&file()), true, Some((1, 2)), true, "London", 1021),
        };
        let json = serde_json::to_string(&st).unwrap();
        assert!(!json.contains('\n'));
        let back: Status = serde_json::from_str(&json).unwrap();
        assert_eq!(back.clients[0].name, "KestrelVance");
        assert_eq!(back.tunnel.handshake_age_s, Some(21));
    }
}
```

- [ ] **Step 4: Implement status**

```rust
//! Tunnel status: what the root worker publishes (`TunnelFile`), what the
//! daemon answers to the IPC `status` request (`Status`), and the pure
//! assembly between them.

use serde::{Deserialize, Serialize};

/// `/run/yutani/tunnel.json`, written by `yutani tunnel run` once a second.
/// Never contains key material.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelFile {
    pub up: bool,
    pub iface: String,
    pub address: String,
    pub endpoint: String,
    /// 0 = no handshake yet.
    pub latest_handshake_unix: u64,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
    pub since_unix: u64,
}

/// Peer line of `wg show <iface> dump`: endpoint, latest handshake (unix
/// seconds, 0 if none), rx bytes, tx bytes. The first line (interface,
/// with the private key) is skipped and never stored.
pub fn parse_wg_dump(dump: &str) -> Option<(String, u64, u64, u64)> {
    let peer = dump.lines().nth(1)?;
    let f: Vec<&str> = peer.split('\t').collect();
    if f.len() < 7 {
        return None;
    }
    Some((f[2].to_string(), f[4].parse().ok()?, f[5].parse().ok()?, f[6].parse().ok()?))
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TunnelStatus {
    pub installed: bool,
    pub connected: bool,
    pub iface: String,
    pub location: String,
    pub address: Option<String>,
    pub endpoint: Option<String>,
    pub handshake_age_s: Option<u64>,
    pub rx_bytes: u64,
    pub tx_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientStatus {
    pub name: String,
    pub active: bool,
}

/// Reply body of the IPC `status` request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// In layout order (the index + 1 is what `focus <n>` takes).
    pub clients: Vec<ClientStatus>,
    pub hidden: bool,
    pub tunnel: TunnelStatus,
}

/// Combine the worker's file, whether `/sys/class/net/yutani0` exists,
/// sysfs byte counters (preferred when present: they need no root and stay
/// fresh even if the file is stale), and the unit-file presence.
pub fn assemble(
    file: Option<&TunnelFile>,
    iface_present: bool,
    sysfs: Option<(u64, u64)>,
    installed: bool,
    location: &str,
    now_unix: u64,
) -> TunnelStatus {
    let connected = iface_present && file.is_some_and(|f| f.up);
    let (rx, tx) = sysfs.unwrap_or_else(|| file.map_or((0, 0), |f| (f.rx_bytes, f.tx_bytes)));
    let handshake_age_s = match file {
        Some(f) if connected && f.latest_handshake_unix > 0 => Some(now_unix.saturating_sub(f.latest_handshake_unix)),
        _ => None,
    };
    TunnelStatus {
        installed,
        connected,
        iface: super::IFACE.to_string(),
        location: location.to_string(),
        address: file.map(|f| f.address.clone()),
        endpoint: file.map(|f| f.endpoint.clone()),
        handshake_age_s,
        rx_bytes: rx,
        tx_bytes: tx,
    }
}
```

Add `pub mod rules; pub mod status;` to `src/tunnel/mod.rs`.

- [ ] **Step 5: Tests, commit**

Run: `cargo test -q tunnel:: 2>&1 | tail -2` → `17 passed` (6 + 6 + 5); full → `93 passed`.

```bash
git add src/tunnel/mod.rs src/tunnel/rules.rs src/tunnel/status.rs
git commit -m "feat(tunnel): pure nft/ip command generators and status types

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 3: Root worker, install/uninstall, `yutani tunnel …` CLI

**Files:**
- Create: `src/tunnel/worker.rs`, `src/tunnel/install.rs`, `src/tunnel/control.rs`
- Modify: `src/tunnel/mod.rs`, `src/main.rs`

**Interfaces:**
- Consumes: Task 1 `WgConf`, constants; Task 2 `rules::*`, `status::{TunnelFile, parse_wg_dump}`.
- Produces:
```rust
// worker.rs
pub fn run() -> anyhow::Result<()>;                 // blocks until SIGTERM/SIGINT
pub fn dry_run(conf_path: &Path, uid: u32) -> anyhow::Result<String>;  // what `run` would execute, keys redacted
// install.rs
pub fn unit_text(exe: &str) -> String;
pub fn polkit_text(username: &str) -> String;
pub fn install(conf: &Path) -> anyhow::Result<()>;          // user side: pkexec
pub fn uninstall() -> anyhow::Result<()>;
pub fn install_root(conf: &Path, uid: u32, username: &str, exe: &str, dry_run: bool) -> anyhow::Result<String>;
pub fn uninstall_root(dry_run: bool) -> anyhow::Result<String>;
// control.rs
pub fn installed() -> bool;                                  // UNIT_PATH exists
pub fn connect() -> anyhow::Result<()>;                      // systemctl start
pub fn disconnect() -> anyhow::Result<()>;                   // systemctl stop
pub fn read_tunnel_file() -> Option<TunnelFile>;
pub fn iface_present() -> bool;
pub fn sysfs_counters() -> Option<(u64, u64)>;
pub fn current_tunnel_status(location: &str) -> TunnelStatus;
```

- [ ] **Step 1: Failing tests for the text generators** (`src/tunnel/install.rs` tests)

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_text_runs_the_given_binary_as_the_worker() {
        let u = unit_text("/opt/yutani/yutani");
        assert!(u.contains("[Unit]\nDescription=Yutani EVE tunnel (WireGuard, per-app routing)\n"));
        assert!(u.contains("After=network-online.target\nWants=network-online.target\n"));
        assert!(u.contains("[Service]\nType=simple\nExecStart=/opt/yutani/yutani tunnel run\n"));
        assert!(u.contains("RuntimeDirectory=yutani\nRuntimeDirectoryMode=0755\n"));
        assert!(u.contains("KillSignal=SIGTERM\nTimeoutStopSec=10\nRestart=no\n"));
        assert!(u.contains("[Install]\nWantedBy=multi-user.target\n"));
    }

    #[test]
    fn polkit_text_authorises_one_unit_for_one_user() {
        let p = polkit_text("daniel");
        assert!(p.contains(r#"action.id == "org.freedesktop.systemd1.manage-units""#));
        assert!(p.contains(r#"action.lookup("unit") == "yutani-tunnel.service""#));
        assert!(p.contains(r#"action.lookup("verb") == "start""#));
        assert!(p.contains(r#"action.lookup("verb") == "stop""#));
        assert!(p.contains(r#"action.lookup("verb") == "restart""#));
        assert!(p.contains(r#"subject.user == "daniel""#));
        assert!(p.contains("polkit.Result.YES"));
        assert!(!p.contains("polkit.Result.NO"));
    }

    #[test]
    fn exe_paths_with_spaces_are_quoted_in_the_unit() {
        assert!(unit_text("/home/me/My Apps/yutani").contains("ExecStart=\"/home/me/My Apps/yutani\" tunnel run\n"));
    }
}
```

and for the worker's dry run (`src/tunnel/worker.rs` tests):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dry_run_lists_commands_and_ruleset_without_secrets() {
        let dir = std::env::temp_dir().join(format!("yutani-dry-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let conf = dir.join("t.conf");
        std::fs::write(&conf, "[Interface]\nPrivateKey = U0VDUkVU\nAddress = 10.2.0.2/32\nDNS = 10.2.0.1\n[Peer]\n# UK#1\nPublicKey = p=\nAllowedIPs = 0.0.0.0/0\nEndpoint = 1.2.3.4:51820\n").unwrap();
        let out = dry_run(&conf, 1000).unwrap();
        assert!(out.contains("ip link add yutani0 type wireguard"));
        assert!(out.contains("wg setconf yutani0 /run/yutani/wg.conf"));
        assert!(out.contains("table inet yutani {"));
        assert!(out.contains("dnat ip to 10.2.0.1"));
        assert!(out.contains("nft delete table inet yutani"));
        assert!(out.contains("PrivateKey = <redacted>"));
        assert!(!out.contains("U0VDUkVU"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
```

- [ ] **Step 2: Implement `worker.rs`**

```rust
//! `yutani tunnel run`: the root worker behind `yutani-tunnel.service`.
//! Brings the tunnel up, publishes status once a second, tears everything
//! down on SIGTERM/SIGINT (and on any failure while coming up).

use anyhow::{Context as _, anyhow};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::conf::WgConf;
use super::status::{TunnelFile, parse_wg_dump};
use super::{CONF_PATH, IFACE, STATUS_PATH, rules};

const RUN_DIR: &str = "/run/yutani";
const WG_CONF_TMP: &str = "/run/yutani/wg.conf";

fn now_unix() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Run one command; `Err` carries the argv and stderr.
fn exec(argv: &[String]) -> anyhow::Result<String> {
    let out = Command::new(&argv[0]).args(&argv[1..]).output().with_context(|| format!("cannot exec {}", argv[0]))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    } else {
        Err(anyhow!("`{}` failed ({}): {}", argv.join(" "), out.status, String::from_utf8_lossy(&out.stderr).trim()))
    }
}

fn exec_stdin(argv: &[String], stdin: &str) -> anyhow::Result<()> {
    use std::io::Write;
    let mut child = Command::new(&argv[0])
        .args(&argv[1..])
        .stdin(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot exec {}", argv[0]))?;
    child.stdin.take().context("stdin")?.write_all(stdin.as_bytes())?;
    let out = child.wait_with_output()?;
    if out.status.success() {
        Ok(())
    } else {
        Err(anyhow!("`{}` failed ({}): {}", argv.join(" "), out.status, String::from_utf8_lossy(&out.stderr).trim()))
    }
}

/// Which uid's cgroup to match: the unit runs as root, so the target user is
/// recorded in the conf by `install-root` (`# yutani: uid = 1000`).
fn uid_from_conf(text: &str) -> Option<u32> {
    text.lines().find_map(|l| l.trim().strip_prefix("# yutani: uid =")).and_then(|v| v.trim().parse().ok())
}

fn load(conf_path: &Path) -> anyhow::Result<(WgConf, u32)> {
    let text = std::fs::read_to_string(conf_path).with_context(|| format!("read {}", conf_path.display()))?;
    let label = conf_path.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let conf = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf_path.display()))?;
    let uid = uid_from_conf(&text).context("conf has no `# yutani: uid = N` line; re-run `yutani tunnel install`")?;
    Ok((conf, uid))
}

fn down() {
    for argv in rules::down_commands() {
        if let Err(e) = exec(&argv) {
            tracing::debug!("teardown: {e:#}");
        }
    }
    let _ = std::fs::remove_file(STATUS_PATH);
    let _ = std::fs::remove_file(WG_CONF_TMP);
}

fn up(conf: &WgConf, uid: u32) -> anyhow::Result<()> {
    std::fs::create_dir_all(RUN_DIR)?;
    std::fs::write(WG_CONF_TMP, conf.wg_native())?;
    std::fs::set_permissions(WG_CONF_TMP, std::fs::Permissions::from_mode(0o600))?;
    for argv in rules::up_commands(conf, WG_CONF_TMP) {
        exec(&argv)?;
    }
    let _ = std::fs::remove_file(WG_CONF_TMP);
    exec_stdin(&["nft".to_string(), "-f".to_string(), "-".to_string()], &rules::nft_ruleset(uid, conf.dns))?;
    Ok(())
}

fn write_status(conf: &WgConf, since: u64) -> anyhow::Result<()> {
    let dump = exec(&["wg".to_string(), "show".to_string(), IFACE.to_string(), "dump".to_string()])?;
    let (endpoint, handshake, rx, tx) = parse_wg_dump(&dump).unwrap_or((conf.endpoint.clone(), 0, 0, 0));
    let file = TunnelFile {
        up: true,
        iface: IFACE.into(),
        address: conf.address.to_string(),
        endpoint,
        latest_handshake_unix: handshake,
        rx_bytes: rx,
        tx_bytes: tx,
        since_unix: since,
    };
    let tmp = format!("{STATUS_PATH}.tmp");
    std::fs::write(&tmp, serde_json::to_vec(&file)?)?;
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644))?;
    std::fs::rename(&tmp, STATUS_PATH)?;
    Ok(())
}

pub fn run() -> anyhow::Result<()> {
    let (conf, uid) = load(Path::new(CONF_PATH))?;
    tracing::info!("tunnel up: {} via {} for uid {uid}", conf.label, conf.endpoint);
    if let Err(e) = up(&conf, uid) {
        tracing::error!("tunnel start failed: {e:#}; tearing down");
        down();
        return Err(e);
    }
    let since = now_unix();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let result: anyhow::Result<()> = rt.block_on(async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let mut int = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::interrupt())?;
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if let Err(e) = write_status(&conf, since) {
                        tracing::warn!("status: {e:#}");
                    }
                }
                _ = term.recv() => break,
                _ = int.recv() => break,
            }
        }
        Ok(())
    });
    tracing::info!("tunnel down");
    down();
    result
}

/// Everything `run` would execute for `conf_path`, secrets redacted.
pub fn dry_run(conf_path: &Path, uid: u32) -> anyhow::Result<String> {
    let text = std::fs::read_to_string(conf_path).with_context(|| format!("read {}", conf_path.display()))?;
    let label = conf_path.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let conf = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf_path.display()))?;
    let mut out = format!("# conf ({})\n{}\n# up\n", conf.label, conf.redacted());
    for argv in rules::up_commands(&conf, WG_CONF_TMP) {
        out.push_str(&argv.join(" "));
        out.push('\n');
    }
    out.push_str("nft -f - <<EOF\n");
    out.push_str(&rules::nft_ruleset(uid, conf.dns));
    out.push_str("EOF\n# down\n");
    for argv in rules::down_commands() {
        out.push_str(&argv.join(" "));
        out.push('\n');
    }
    Ok(out)
}
```

`tokio::select!` needs the `macros` feature — add `"macros"` to the tokio features in `Cargo.toml` (still no new lock entries).

- [ ] **Step 3: Implement `install.rs`**

```rust
//! `yutani tunnel install|uninstall`: one-time privileged setup through
//! `pkexec`, and the root-side `install-root|uninstall-root` it invokes.

use anyhow::{Context as _, anyhow, ensure};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use super::conf::WgConf;
use super::{CONF_PATH, POLKIT_PATH, UNIT_NAME, UNIT_PATH};

fn quote_exec(exe: &str) -> String {
    if exe.chars().any(|c| c.is_whitespace()) { format!("\"{exe}\"") } else { exe.to_string() }
}

pub fn unit_text(exe: &str) -> String {
    format!(
        "[Unit]\nDescription=Yutani EVE tunnel (WireGuard, per-app routing)\nAfter=network-online.target\nWants=network-online.target\n\n\
[Service]\nType=simple\nExecStart={} tunnel run\nRuntimeDirectory=yutani\nRuntimeDirectoryMode=0755\nKillSignal=SIGTERM\nTimeoutStopSec=10\nRestart=no\n\n\
[Install]\nWantedBy=multi-user.target\n",
        quote_exec(exe)
    )
}

pub fn polkit_text(username: &str) -> String {
    format!(
        "// Installed by `yutani tunnel install`: lets {username} start/stop the EVE tunnel unit without a password.\n\
polkit.addRule(function(action, subject) {{\n\
    if (action.id == \"org.freedesktop.systemd1.manage-units\" &&\n\
        action.lookup(\"unit\") == \"{UNIT_NAME}\" &&\n\
        (action.lookup(\"verb\") == \"start\" || action.lookup(\"verb\") == \"stop\" ||\n\
         action.lookup(\"verb\") == \"restart\") &&\n\
        subject.user == \"{username}\") {{\n\
        return polkit.Result.YES;\n\
    }}\n\
}});\n"
    )
}

fn current_exe() -> anyhow::Result<String> {
    Ok(std::env::current_exe()?.canonicalize()?.to_string_lossy().into_owned())
}

fn whoami() -> anyhow::Result<(u32, String)> {
    let uid = crate::ipc::uid_for_tests_and_paths();
    let name = std::env::var("USER").or_else(|_| std::env::var("LOGNAME")).context("USER not set")?;
    Ok((uid, name))
}

/// User side: `pkexec <self> tunnel install-root …` (one password prompt).
pub fn install(conf: &Path) -> anyhow::Result<()> {
    let conf = conf.canonicalize().with_context(|| format!("{}", conf.display()))?;
    let exe = current_exe()?;
    let (uid, user) = whoami()?;
    let status = Command::new("pkexec")
        .args([&exe, "tunnel", "install-root", "--conf"])
        .arg(&conf)
        .args(["--uid", &uid.to_string(), "--user", &user, "--exe", &exe])
        .status()
        .context("pkexec")?;
    ensure!(status.success(), "install cancelled or failed (pkexec exit {status})");
    println!("Tunnel installed. Next: `yutani tunnel connect`. Delete {} (it holds the private key and is world-readable).", conf.display());
    Ok(())
}

pub fn uninstall() -> anyhow::Result<()> {
    let exe = current_exe()?;
    let status = Command::new("pkexec").args([&exe, "tunnel", "uninstall-root"]).status().context("pkexec")?;
    ensure!(status.success(), "uninstall cancelled or failed (pkexec exit {status})");
    println!("Tunnel uninstalled.");
    Ok(())
}

fn write(path: &str, text: &str, mode: u32) -> anyhow::Result<()> {
    let p = Path::new(path);
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(p, text)?;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode))?;
    Ok(())
}

/// Root side. With `dry_run`, returns what would be written instead of writing.
pub fn install_root(conf: &Path, uid: u32, username: &str, exe: &str, dry_run: bool) -> anyhow::Result<String> {
    ensure!(conf.is_absolute() && Path::new(exe).is_absolute(), "paths must be absolute");
    let text = std::fs::read_to_string(conf).with_context(|| format!("read {}", conf.display()))?;
    let label = conf.file_stem().and_then(|s| s.to_str()).unwrap_or("tunnel");
    let parsed = WgConf::parse(&text, label).map_err(|e| anyhow!("{}: {e}", conf.display()))?;
    let stored = format!("# yutani: label = {}\n# yutani: uid = {uid}\n{text}", parsed.label);
    let unit = unit_text(exe);
    let rule = polkit_text(username);
    let report = format!(
        "{CONF_PATH} (0600):\n{}\n{UNIT_PATH}:\n{unit}\n{POLKIT_PATH}:\n{rule}",
        stored.lines().map(|l| if l.trim_start().starts_with("PrivateKey") || l.trim_start().starts_with("PresharedKey") {
            format!("{} = <redacted>", l.split('=').next().unwrap_or("").trim())
        } else { l.to_string() }).collect::<Vec<_>>().join("\n")
    );
    if dry_run {
        return Ok(report);
    }
    write(CONF_PATH, &stored, 0o600)?;
    write(UNIT_PATH, &unit, 0o644)?;
    write(POLKIT_PATH, &rule, 0o644)?;
    let st = Command::new("systemctl").arg("daemon-reload").status().context("systemctl daemon-reload")?;
    ensure!(st.success(), "systemctl daemon-reload failed");
    if parsed.dns.is_none() {
        eprintln!("warning: the conf has no DNS entry; EVE's DNS lookups will not go through the tunnel");
    }
    Ok(report)
}

pub fn uninstall_root(dry_run: bool) -> anyhow::Result<String> {
    let report = format!("stop {UNIT_NAME}; remove {CONF_PATH}, {UNIT_PATH}, {POLKIT_PATH}; systemctl daemon-reload\n");
    if dry_run {
        return Ok(report);
    }
    let _ = Command::new("systemctl").args(["stop", UNIT_NAME]).status();
    for p in [CONF_PATH, UNIT_PATH, POLKIT_PATH] {
        match std::fs::remove_file(p) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("remove {p}")),
        }
    }
    let _ = Command::new("systemctl").arg("daemon-reload").status();
    Ok(report)
}
```

`crate::ipc::uid_for_tests_and_paths` does not exist — in `src/ipc.rs` rename the private `fn uid()` to `pub fn uid() -> u32` and call `crate::ipc::uid()` here.

- [ ] **Step 4: Implement `control.rs`**

```rust
//! Unprivileged side: start/stop the unit (polkit-authorised) and read the
//! tunnel's state without root.

use anyhow::{Context as _, ensure};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use super::status::{TunnelFile, TunnelStatus, assemble};
use super::{IFACE, STATUS_PATH, UNIT_NAME, UNIT_PATH};

pub fn installed() -> bool {
    std::path::Path::new(UNIT_PATH).exists()
}

fn systemctl(verb: &str) -> anyhow::Result<()> {
    ensure!(installed(), "tunnel is not installed; run `yutani tunnel install <conf>`");
    let out = Command::new("systemctl").args([verb, UNIT_NAME]).output().context("systemctl")?;
    ensure!(out.status.success(), "systemctl {verb} {UNIT_NAME}: {}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

pub fn connect() -> anyhow::Result<()> {
    systemctl("start")
}

pub fn disconnect() -> anyhow::Result<()> {
    systemctl("stop")
}

pub fn read_tunnel_file() -> Option<TunnelFile> {
    serde_json::from_slice(&std::fs::read(STATUS_PATH).ok()?).ok()
}

pub fn iface_present() -> bool {
    std::path::Path::new(&format!("/sys/class/net/{IFACE}")).exists()
}

pub fn sysfs_counters() -> Option<(u64, u64)> {
    let read = |n: &str| std::fs::read_to_string(format!("/sys/class/net/{IFACE}/statistics/{n}")).ok()?.trim().parse::<u64>().ok();
    Some((read("rx_bytes")?, read("tx_bytes")?))
}

pub fn current_tunnel_status(location: &str) -> TunnelStatus {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    assemble(read_tunnel_file().as_ref(), iface_present(), sysfs_counters(), installed(), location, now)
}
```

Add `pub mod control; pub mod install; pub mod worker;` to `src/tunnel/mod.rs`.

- [ ] **Step 5: CLI in `src/main.rs`**

Add to `Command`:

```rust
    /// EVE-only WireGuard tunnel
    Tunnel {
        #[command(subcommand)]
        action: TunnelAction,
    },
```

```rust
#[derive(Subcommand)]
enum TunnelAction {
    /// Install the tunnel from a wg-quick .conf (asks for your password once)
    Install {
        conf: std::path::PathBuf,
        /// Print what would be installed instead of installing (no root needed)
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove the tunnel unit, conf and polkit rule
    Uninstall,
    /// Start the tunnel (EVE traffic goes via London)
    Connect,
    /// Stop the tunnel (EVE traffic goes direct)
    Disconnect,
    /// Show tunnel state as JSON
    Status,
    /// [root] the worker behind yutani-tunnel.service
    #[command(hide = true)]
    Run,
    /// [root] called by `install` through pkexec
    #[command(hide = true)]
    InstallRoot {
        #[arg(long)] conf: std::path::PathBuf,
        #[arg(long)] uid: u32,
        #[arg(long)] user: String,
        #[arg(long)] exe: String,
    },
    /// [root] called by `uninstall` through pkexec
    #[command(hide = true)]
    UninstallRoot,
}
```

Match arms:

```rust
        Some(Command::Tunnel { action }) => match action {
            TunnelAction::Install { conf, dry_run: true } => {
                let (uid, user) = (ipc::uid(), std::env::var("USER").unwrap_or_default());
                let exe = std::env::current_exe().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
                tunnel::install::install_root(&conf.canonicalize().unwrap_or(conf.clone()), uid, &user, &exe, true)
                    .and_then(|report| { print!("{report}"); tunnel::worker::dry_run(&conf, uid) })
                    .map(|plan| { print!("\n{plan}"); ExitCode::SUCCESS })
            }
            TunnelAction::Install { conf, dry_run: false } => tunnel::install::install(&conf).map(|()| ExitCode::SUCCESS),
            TunnelAction::Uninstall => tunnel::install::uninstall().map(|()| ExitCode::SUCCESS),
            TunnelAction::Connect => tunnel::control::connect().map(|()| ExitCode::SUCCESS),
            TunnelAction::Disconnect => tunnel::control::disconnect().map(|()| ExitCode::SUCCESS),
            TunnelAction::Status => {
                let config = model::config::Config::load();
                let st = tunnel::control::current_tunnel_status(&config.tunnel.location);
                println!("{}", serde_json::to_string_pretty(&st).unwrap_or_default());
                Ok(ExitCode::SUCCESS)
            }
            TunnelAction::Run => tunnel::worker::run().map(|()| ExitCode::SUCCESS),
            TunnelAction::InstallRoot { conf, uid, user, exe } => {
                tunnel::install::install_root(&conf, uid, &user, &exe, false).map(|report| { print!("{report}"); ExitCode::SUCCESS })
            }
            TunnelAction::UninstallRoot => tunnel::install::uninstall_root(false).map(|r| { print!("{r}"); ExitCode::SUCCESS }),
        },
```

- [ ] **Step 6: Build, test, dry-run**

Run: `cargo build -q 2>&1 | grep -E '^(warning|error)' -A5; cargo test -q 2>&1 | grep 'test result'` → clean (transitional: `control::*` readers unused until Task 4 — list them); `97 passed` (93 + 3 + 1).

Dry run against Daniel's real conf (reads it, never writes):
`./target/debug/yutani tunnel install --dry-run ~/Downloads/EVE-UK-455.conf | sed -n 1,80p`
Expected: the redacted conf with `# yutani: label = UK#455` and `# yutani: uid = 1000`, the unit text with `ExecStart=/home/user/Yutani/target/debug/yutani tunnel run`, the polkit rule for `daniel`, then the command plan (`ip link add yutani0 …`, the nft ruleset with `level 5 "user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice"`, `dnat ip to 10.2.0.1`, the down list). The private key string must not appear anywhere: `./target/debug/yutani tunnel install --dry-run ~/Downloads/EVE-UK-455.conf | grep -c "$(grep PrivateKey ~/Downloads/EVE-UK-455.conf | cut -d= -f2- | tr -d ' ')"` → `0`.

`./target/debug/yutani tunnel status` → JSON with `"installed": false, "connected": false`.
`./target/debug/yutani tunnel connect` → `yutani: tunnel is not installed; run …`, exit 1.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml src/tunnel src/ipc.rs src/main.rs
git commit -m "feat(tunnel): root worker (up/status loop/down), pkexec install/uninstall with dry-run, connect/disconnect/status CLI

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 4: IPC `status` / `tunnel connect|disconnect` in the daemon and CLI

**Files:**
- Modify: `src/ipc.rs`, `src/cli.rs`, `src/ui/mod.rs`, `src/main.rs`

**Interfaces:**
- Consumes: `tunnel::control::{connect, disconnect, current_tunnel_status}`, `tunnel::status::{Status, ClientStatus}`, `App::focus_order`, `App.hidden`, `App.clients`.
- Produces: `Request::{Status, TunnelConnect, TunnelDisconnect}`; `Response::OkData(String)`; CLI `yutani status` (prints the JSON).

- [ ] **Step 1: Protocol tests** (extend the tests in `src/ipc.rs`)

Add to `parses_every_command`:
```rust
        assert_eq!(Request::parse("status"), Ok(Request::Status));
        assert_eq!(Request::parse("tunnel connect"), Ok(Request::TunnelConnect));
        assert_eq!(Request::parse("tunnel disconnect"), Ok(Request::TunnelDisconnect));
```
to `rejects_bad_requests_with_a_reason`:
```rust
        assert_eq!(Request::parse("tunnel").unwrap_err(), "tunnel needs connect or disconnect");
        assert_eq!(Request::parse("tunnel up").unwrap_err(), "tunnel needs connect or disconnect");
```
to `request_lines_round_trip`'s list: `Request::Status, Request::TunnelConnect, Request::TunnelDisconnect`, and to `responses_round_trip_and_tolerate_garbage`:
```rust
        assert_eq!(Response::parse("ok {\"a\":1}\n"), Response::OkData("{\"a\":1}".into()));
        assert_eq!(Response::OkData("x".into()).to_line(), "ok x\n");
```

- [ ] **Step 2: Protocol implementation**

`Request` gains `Status, TunnelConnect, TunnelDisconnect`. In `parse`, before the `("", _)` arm:
```rust
            ("status", None) => Ok(Request::Status),
            ("tunnel", Some("connect")) => Ok(Request::TunnelConnect),
            ("tunnel", Some("disconnect")) => Ok(Request::TunnelDisconnect),
            ("tunnel", _) => Err("tunnel needs connect or disconnect".into()),
```
and add `"status"` to the takes-no-argument list. `to_line`: `"status\n"`, `"tunnel connect\n"`, `"tunnel disconnect\n"`.

`Response` gains `OkData(String)`; `parse`: `"ok"` → `Ok`, `"ok <rest>"` → `OkData(rest)` (only when the char after `ok` is a space); `to_line`: `format!("ok {}\n", data.replace('\n', " "))`.

`src/cli.rs` `send`: `Response::OkData(data) => { println!("{data}"); ExitCode::SUCCESS }`.

`src/main.rs`: add `/// Print the daemon's status (clients, visibility, tunnel) as JSON` `Status` subcommand → `cli::send(&ipc::Request::Status)`; keep `tunnel status` as the daemon-less variant.

- [ ] **Step 3: Daemon handling** (`src/ui/mod.rs`, in `handle_request`)

```rust
            Request::Status => {
                let order = self.focus_order();
                let clients = order
                    .iter()
                    .filter_map(|h| self.clients.get(h))
                    .map(|c| crate::tunnel::status::ClientStatus { name: c.info.login.label().to_string(), active: c.info.activated })
                    .collect();
                let status = crate::tunnel::status::Status {
                    clients,
                    hidden: self.hidden,
                    tunnel: crate::tunnel::control::current_tunnel_status(&self.config.tunnel.location),
                };
                match serde_json::to_string(&status) {
                    Ok(json) => (Ok(Some(json)), Task::none()),
                    Err(e) => (Err(format!("status: {e}")), Task::none()),
                }
            }
            Request::TunnelConnect => (crate::tunnel::control::connect().map(|()| None).map_err(|e| format!("{e:#}")), Task::none()),
            Request::TunnelDisconnect => (crate::tunnel::control::disconnect().map(|()| None).map_err(|e| format!("{e:#}")), Task::none()),
```

This changes `handle_request`'s result type to `(Result<Option<String>, String>, Task<…>)`: `Ok(None)` → `Response::Ok`, `Ok(Some(data))` → `Response::OkData(data)`. Update every existing arm (`Ok(())` → `Ok(None)`) and the `Msg::Ipc` arm in `update`.

`systemctl start` blocks until the unit is up (the worker's `up()` — a second or two). That runs on the UI thread inside `update`; acceptable for v1 (the applet shows the sync icon meanwhile). Note it in the report as a follow-up candidate (`Task::perform` off-thread).

- [ ] **Step 4: Build, test, smoke**

`cargo test -q 2>&1 | grep 'test result'` → `97 passed` (same count: extended tests). Smoke with the daemon (EVE running):
```bash
(RUST_LOG=yutani=info setsid nohup ./target/debug/yutani >/dev/null 2>&1 &); sleep 4
./target/debug/yutani status | python3 -m json.tool | head -30    # clients: [KestrelVance], hidden false, tunnel.installed false
./target/debug/yutani tunnel connect; echo "exit=$?"               # CLI path: not installed, exit 1
printf 'tunnel connect\n' | python3 -c 'import socket,os,sys;s=socket.socket(socket.AF_UNIX);s.connect(os.environ["XDG_RUNTIME_DIR"]+"/yutani.sock");s.sendall(sys.stdin.buffer.read());print(s.recv(200))'   # b'err tunnel is not installed; …'
./target/debug/yutani quit
```

- [ ] **Step 5: Commit**

```bash
git add src/ipc.rs src/cli.rs src/ui/mod.rs src/main.rs
git commit -m "feat(ipc): status (clients + tunnel JSON) and tunnel connect/disconnect requests

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 5: `yutani launch` and auto-adoption

**Files:**
- Create: `src/launch.rs`, `src/adopt.rs`
- Modify: `src/main.rs`, `src/ui/mod.rs` (subscription), `src/ui/mod.rs` `Msg`

**Interfaces (produced):**
```rust
// launch.rs
pub fn systemd_run_argv(command: &[String]) -> Vec<String>;   // pure
pub fn run(command: Vec<String>) -> ExitCode;                 // exec systemd-run, fall back to direct
// adopt.rs
pub struct Candidate { pub pid: u32, pub name: String }
pub fn is_eve_process(exe_name: &str, patterns: &[String]) -> bool;                      // pure, case-insensitive
pub fn in_slice(cgroup_file: &str) -> bool;                                              // pure: /proc/<pid>/cgroup text
pub fn busctl_adopt_argv(pid: u32) -> Vec<String>;                                        // pure
pub fn scan(patterns: &[String], uid: u32) -> Vec<Candidate>;                           // /proc
pub fn adopt(c: &Candidate) -> anyhow::Result<()>;
pub fn subscription(patterns: Vec<String>) -> Subscription<AdoptEvent>;                 // every 2 s
pub struct AdoptEvent { pub pid: u32, pub name: String, pub result: Result<(), String> }
```

- [ ] **Step 1: Failing tests**

`src/launch.rs`:
```rust
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
```

`src/adopt.rs`:
```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn pats() -> Vec<String> {
        vec!["exefile.exe".into(), "eve-online.exe".into()]
    }

    #[test]
    fn matches_eve_executables_case_insensitively() {
        assert!(is_eve_process("exefile.exe", &pats()));
        assert!(is_eve_process("EXEFILE.EXE", &pats()));
        assert!(is_eve_process("C:\\CCP\\EVE\\tq\\bin64\\exefile.exe", &pats()));
        assert!(is_eve_process("/some/pfx/drive_c/EVE/eve-online.exe", &pats()));
        assert!(!is_eve_process("wineserver", &pats()));
        assert!(!is_eve_process("exefile.exe.old", &pats()));
    }

    #[test]
    fn detects_slice_membership_from_proc_cgroup() {
        assert!(in_slice("0::/user.slice/user-1000.slice/user@1000.service/yutani-eve.slice/yutani-eve-123.scope\n"));
        assert!(in_slice("0::/user.slice/user-1000.slice/user@1000.service/yutani-eve.slice/yutani-eve-adopt-9.scope\n"));
        // systemd nests yutani-eve.slice under yutani.slice (dash-implied
        // parent), so the real cgroup path has five components; a sibling
        // scope directly under the parent slice must not count.
        assert!(in_slice("0::/user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice/yutani-eve-adopt-9.scope\n"));
        assert!(!in_slice("0::/user.slice/user-1000.slice/user@1000.service/yutani.slice/other.scope\n"));
        assert!(!in_slice("0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-cosmic-x.scope\n"));
    }

    #[test]
    fn busctl_argv_moves_one_pid_into_a_scope_under_the_slice() {
        let a = busctl_adopt_argv(4242);
        assert_eq!(
            a,
            vec![
                "busctl", "--user", "call", "org.freedesktop.systemd1", "/org/freedesktop/systemd1",
                "org.freedesktop.systemd1.Manager", "StartTransientUnit", "ssa(sv)a(sa(sv))",
                "yutani-eve-adopt-4242.scope", "fail", "3",
                "PIDs", "au", "1", "4242",
                "Slice", "s", "yutani-eve.slice",
                "CollectMode", "s", "inactive-or-failed",
                "0",
            ]
        );
    }
}
```

- [ ] **Step 2: Implement `src/launch.rs`**

```rust
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
```

(`systemd-run --scope` waits for the command and propagates its exit status, so Steam sees the game's lifetime correctly.)

- [ ] **Step 3: Implement `src/adopt.rs`**

```rust
//! Auto-adoption: move running EVE processes into `yutani-eve.slice` so the
//! tunnel rules apply to them even when they weren't started through
//! `yutani launch`. Uses the user's own systemd manager (no root).

use cosmic::iced::futures::{SinkExt, StreamExt, channel::mpsc};
use cosmic::iced::{self, Subscription};
use std::collections::HashSet;
use std::process::Command;
use std::time::Duration;

use crate::tunnel::SLICE;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Candidate {
    pub pid: u32,
    pub name: String,
}

#[derive(Clone, Debug)]
pub struct AdoptEvent {
    pub pid: u32,
    pub name: String,
    pub result: Result<(), String>,
}

/// `exe_name` is `/proc/<pid>/comm` or a cmdline argv[0] (Windows or Unix
/// path); matches when its basename equals one of `patterns`, ignoring case.
pub fn is_eve_process(exe_name: &str, patterns: &[String]) -> bool {
    let base = exe_name.rsplit(['/', '\\']).next().unwrap_or(exe_name);
    patterns.iter().any(|p| p.eq_ignore_ascii_case(base))
}

/// `/proc/<pid>/cgroup` (cgroup v2: one `0::/path` line) is under our slice.
pub fn in_slice(cgroup_file: &str) -> bool {
    cgroup_file.lines().any(|l| l.split("::").nth(1).is_some_and(|p| p.contains(&format!("/{SLICE}/")) || p.ends_with(&format!("/{SLICE}"))))
}

pub fn busctl_adopt_argv(pid: u32) -> Vec<String> {
    [
        "busctl", "--user", "call", "org.freedesktop.systemd1", "/org/freedesktop/systemd1",
        "org.freedesktop.systemd1.Manager", "StartTransientUnit", "ssa(sv)a(sa(sv))",
        &format!("yutani-eve-adopt-{pid}.scope"), "fail", "3",
        "PIDs", "au", "1", &pid.to_string(),
        "Slice", "s", SLICE,
        "CollectMode", "s", "inactive-or-failed",
        "0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

/// EVE processes owned by `uid` that are not yet in the slice.
pub fn scan(patterns: &[String], uid: u32) -> Vec<Candidate> {
    use std::os::unix::fs::MetadataExt;
    let Ok(dir) = std::fs::read_dir("/proc") else { return Vec::new() };
    let mut out = Vec::new();
    for entry in dir.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<u32>() else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.uid() != uid {
            continue;
        }
        let comm = std::fs::read_to_string(entry.path().join("comm")).unwrap_or_default();
        let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        let argv0 = cmdline.split(|b| *b == 0).next().map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        let name = if is_eve_process(comm.trim(), patterns) {
            comm.trim().to_string()
        } else if is_eve_process(&argv0, patterns) {
            argv0.rsplit(['/', '\\']).next().unwrap_or(&argv0).to_string()
        } else {
            continue;
        };
        let cgroup = std::fs::read_to_string(entry.path().join("cgroup")).unwrap_or_default();
        if in_slice(&cgroup) {
            continue;
        }
        out.push(Candidate { pid, name });
    }
    out
}

pub fn adopt(c: &Candidate) -> anyhow::Result<()> {
    let argv = busctl_adopt_argv(c.pid);
    let out = Command::new(&argv[0]).args(&argv[1..]).output()?;
    anyhow::ensure!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr).trim());
    Ok(())
}

/// Scan every 2 s; adopt what's new; report each attempt once per pid.
pub fn subscription(patterns: Vec<String>) -> Subscription<AdoptEvent> {
    Subscription::run_with(patterns, run)
}

fn run(patterns: &Vec<String>) -> iced::futures::stream::BoxStream<'static, AdoptEvent> {
    let patterns = patterns.clone();
    let uid = crate::ipc::uid();
    let (mut tx, rx) = mpsc::channel::<AdoptEvent>(16);
    let worker = async move {
        let mut seen: HashSet<u32> = HashSet::new();
        loop {
            for c in scan(&patterns, uid) {
                if !seen.insert(c.pid) {
                    continue;
                }
                let result = adopt(&c).map_err(|e| format!("{e:#}"));
                let _ = tx.send(AdoptEvent { pid: c.pid, name: c.name.clone(), result }).await;
            }
            // Forget pids that are gone so a reused pid can be adopted again.
            seen.retain(|pid| std::path::Path::new(&format!("/proc/{pid}")).exists());
            futures_timer::Delay::new(Duration::from_secs(2)).await;
        }
    };
    Box::pin(iced::futures::stream::select(
        iced::futures::stream::once(worker).filter_map(|()| async { None }),
        rx,
    ))
}
```

`Subscription::run_with` in this pinned iced takes a `fn(&D) -> S` where `D: Hash` — `Vec<String>` hashes fine; the pattern mirrors `backend::subscription`. If the trait bound demands `S: Stream + Send + 'static`, the `BoxStream` satisfies it.

- [ ] **Step 4: Wire up**

`src/main.rs`: `mod adopt; mod launch;`; subcommand
```rust
    /// Run a command (Steam's %command%) inside the tunnel cgroup
    Launch {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
```
→ `Some(Command::Launch { command }) => Ok(launch::run(command)),`.

`src/ui/mod.rs`: `Msg::Adopt(adopt::AdoptEvent)`; in `subscription`, `if self.config.tunnel.auto_adopt { subs.push(adopt::subscription(self.config.tunnel.adopt_processes.clone()).map(Msg::Adopt)); }`; in `update`:
```rust
            Msg::Adopt(ev) => {
                match ev.result {
                    Ok(()) => tracing::info!(pid = ev.pid, name = %ev.name, "adopted into yutani-eve.slice"),
                    Err(e) => tracing::warn!(pid = ev.pid, name = %ev.name, "adoption failed: {e}"),
                }
                Task::none()
            }
```

- [ ] **Step 5: Build, test, smoke**

`cargo test -q 2>&1 | grep 'test result'` → `101 passed` (97 + 1 + 3). Smoke (EVE running):
```bash
./target/debug/yutani launch -- /bin/sh -c 'cat /proc/self/cgroup'    # 0::/user.slice/user-1000.slice/user@1000.service/yutani.slice/yutani-eve.slice/run-….scope
systemctl --user list-units 'yutani-eve*' --all | head -5
(RUST_LOG=yutani=info setsid nohup ./target/debug/yutani >/tmp/claude-1000/-home-user-Yutani/eb30ce00-6f26-46d7-ad81-bfd5bce301f6/scratchpad/yutani_adopt.log 2>&1 &); sleep 6
sed 's/\x1b\[[0-9;]*m//g' /tmp/claude-1000/-home-user-Yutani/eb30ce00-6f26-46d7-ad81-bfd5bce301f6/scratchpad/yutani_adopt.log | grep -E 'adopt|panic|error'   # "adopted into yutani-eve.slice pid=… name=exefile.exe"
cat /proc/$(pgrep -f exefile.exe | head -1)/cgroup                       # …/yutani-eve.slice/yutani-eve-adopt-<pid>.scope
systemctl --user status yutani-eve.slice --no-pager | head -8
./target/debug/yutani quit
```
Adoption moves the live EVE process — with no tunnel installed this is harmless (no rules exist), and it verifies the D-Bus path. Note in the report whether `StartTransientUnit` needed any different property names (systemd 261).

- [ ] **Step 6: Commit**

```bash
git add src/launch.rs src/adopt.rs src/main.rs src/ui/mod.rs
git commit -m "feat(tunnel): yutani launch wrapper and auto-adoption of EVE processes into yutani-eve.slice

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 6: Hands-on acceptance (Daniel) and spec status

- [ ] **Step 1: Install and verify routing** (Daniel at the keyboard; Claude reads outputs)

The binary the unit runs must be root-owned and not writable by others (spec
§5 and §9), so install it before installing the tunnel — `yutani tunnel
install` refuses a `target/debug/yutani` and says so:

```bash
cargo build --release
sudo install -o root -g root -m 0755 target/release/yutani /usr/local/bin/yutani
/usr/local/bin/yutani tunnel install --dry-run ~/Downloads/EVE-UK-455.conf   # no root: check what it would write
```

```bash
yutani tunnel install ~/Downloads/EVE-UK-455.conf          # password prompt; then delete the Downloads copy
yutani tunnel connect && sleep 3 && yutani tunnel status   # connected: true, handshake_age_s small
curl -s https://ifconfig.me; echo                          # home IP
systemd-run --user --scope --quiet --slice=yutani-eve.slice curl -s https://ifconfig.me; echo    # London IP
sudo nft list table inet yutani                            # the three chains, as generated; drop counter ~0 in normal use
ip rule show                                               # fwmark 0x59 -> 51820 (prio 1000), from 10.2.0.2 -> 51820 (prio 1001)
yutani tunnel disconnect && systemd-run --user --scope --quiet --slice=yutani-eve.slice curl -s https://ifconfig.me; echo   # home IP again
```

**DNS check — do not use `dig`.** `dig` speaks DNS directly and therefore
goes through the DNAT, so it passes even while the real leak is open. The
game uses glibc's `getaddrinfo`, which on this machine talks to
`systemd-resolved` over a unix socket and emits no IP packet from the cgroup
at all (spec §2, "Known gap"). Check that path instead, with
`sudo resolvectl monitor` (or `journalctl -u systemd-resolved -f`) running in
another terminal so you can see what resolved actually sent upstream:

```bash
systemd-run --user --scope --quiet --slice=yutani-eve.slice getent hosts whoami.akamai.net
resolvectl query whoami.akamai.net
```

Expect the lookup to succeed and resolved to have queried over the *normal*
route — that is the known gap, not a regression. Record what you observed.
Kill-switch check: `yutani tunnel connect`, then `sudo ip link set yutani0 down` (simulating a dead tunnel) → the slice `curl` must time out (`curl -m 5 …` exit 28) while plain `curl` works. `sudo ip link set yutani0 up` restores it: the kernel deletes `default dev yutani0 table 51820` when the link goes down and does *not* put it back when it comes up, so the worker's 1 s status tick re-adds it with `ip route replace` — give it a second, then re-run the slice `curl` (check with `ip route show table 51820`). If anything else was poked by hand, `yutani tunnel disconnect && yutani tunnel connect` is the clean reset. Then EVE: set the Steam launch option to `PROTON_ENABLE_WAYLAND=1 yutani launch -- %command%`, start EVE, log in; `yutani status` shows the client and `tx_bytes`/`rx_bytes` climbing; `journalctl -u yutani-tunnel -n 20` clean.

- [ ] **Step 2: Spec status**

Append to `docs/superpowers/specs/2026-09-12-yutani-tunnel-design.md`: *Status after plan A (date): implemented; acceptance results (IPs observed, kill-switch behaviour, any deviations).* Commit `docs: tunnel acceptance`.

---

## Self-review

**Spec coverage:** §3 pieces → Tasks 3 (worker, install, control), 5 (launch, adopt); §4 up/loop/down sequence and nft ruleset → Task 2 (text) + Task 3 (execution, `RuntimeDirectory`); §5 install/polkit/uninstall/idempotence → Task 3; §6 CLI + IPC `status` + config → Tasks 1, 3, 4; §7 wrapper + adoption (patterns, `StartTransientUnit`, 2 s cadence, one log per pid) → Task 5; §8/§9 (no keys in outputs, failure → teardown, not-installed errors) → Tasks 1, 3; §10 tests → each task; integration → Task 6.

**Placeholder scan:** none. The uid line in the stored conf (`# yutani: uid = N`) is an addition the spec didn't state explicitly (the worker runs as root and must know whose cgroup to match) — recorded here; Task 6 Step 2 notes it in the spec.

**Type consistency:** `WgConf` fields used identically in Tasks 1–3; `rules::up_commands(&WgConf, &str)`/`down_commands()`/`nft_ruleset(u32, Option<Ipv4Addr>)` identical in Tasks 2–3; `status::{TunnelFile, parse_wg_dump, assemble, Status, ClientStatus, TunnelStatus}` identical in Tasks 2–4; `control::{connect, disconnect, current_tunnel_status}` in Tasks 3–4; `ipc::uid()` made `pub` in Task 3 and used in Task 5; `handle_request` result type change confined to Task 4; test totals 75 → 82 → 93 → 97 → 97 → 101.
