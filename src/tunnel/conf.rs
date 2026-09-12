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
