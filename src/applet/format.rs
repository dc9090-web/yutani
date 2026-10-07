//! Number and label formatting, exactly as the design handoff specifies:
//! decimal SI (matching `wg`'s human output style), ≥1e9 → `X.XX GB`,
//! ≥1e6 → `X.X MB`, else `N KB`; rates ≥1e6 → `X.X MB/s`, else `N KB/s`.
//! Sub-unit values truncate rather than round, so 999 999 B reads
//! `999 KB` and never the nonsensical `1000 KB`.

/// A cumulative byte counter.
pub fn bytes(n: u64) -> String {
    if n >= 1_000_000_000 {
        format!("{:.2} GB", n as f64 / 1_000_000_000.0)
    } else if n >= 1_000_000 {
        format!("{:.1} MB", n as f64 / 1_000_000.0)
    } else {
        format!("{} KB", n / 1_000)
    }
}

/// A transfer rate in bytes per second. Negative (impossible) input and
/// everything under 1 kB/s read `0 KB/s`.
pub fn rate(bytes_per_s: f64) -> String {
    let b = if bytes_per_s.is_finite() && bytes_per_s > 0.0 { bytes_per_s } else { 0.0 };
    if b >= 1_000_000.0 {
        format!("{:.1} MB/s", b / 1_000_000.0)
    } else {
        format!("{} KB/s", (b / 1_000.0) as u64)
    }
}

/// A rate as the throughput card prints it: the number and its unit apart,
/// so the unit can be set smaller — `("41", "KB/s")`, `("1.4", "MB/s")`.
pub fn rate_parts(bytes_per_s: f64) -> (String, &'static str) {
    let b = if bytes_per_s.is_finite() && bytes_per_s > 0.0 { bytes_per_s } else { 0.0 };
    if b >= 1_000_000.0 {
        (format!("{:.1}", b / 1_000_000.0), "MB/s")
    } else {
        (format!("{}", (b / 1_000.0) as u64), "KB/s")
    }
}

/// A session length as the tunnel line prints it: `1h 12m 22s`, `12m 05s`.
pub fn uptime(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 { format!("{h}h {m}m {s:02}s") } else { format!("{m}m {s:02}s") }
}

/// Seconds since the last handshake, or the em-dash when there is none.
pub fn handshake(age_s: Option<u64>) -> String {
    match age_s {
        Some(age) => format!("hs {age}s ago"),
        None => "hs —".to_string(),
    }
}

/// The handoff's copy, singular only for exactly one account.
pub fn accounts_label(n: usize) -> &'static str {
    if n == 1 { "Account connected" } else { "Accounts connected" }
}

/// A rate as whole KB/s (Nostromo readouts: digits only, the unit is a
/// separate label).
pub fn kb_int(bytes_per_s: f64) -> String {
    let b = if bytes_per_s.is_finite() && bytes_per_s > 0.0 { bytes_per_s } else { 0.0 };
    format!("{}", (b / 1_000.0) as u64)
}

/// A session total: MB with one decimal, however large.
pub fn mb1(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0)
}

/// Tunnel uptime as the mission clock: `T+ 1H 38M`, `T+ 4M 05S`, `T+ 4S`.
pub fn mission_clock(secs: u64) -> String {
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("T+ {h}H {m:02}M")
    } else if m > 0 {
        format!("T+ {m}M {s:02}S")
    } else {
        format!("T+ {s}S")
    }
}

const GIB: f64 = 1_073_741_824.0;

/// Installed RAM from `MemTotal`: whole GiB, rounded up.
pub fn gb_ceil(bytes: u64) -> String {
    format!("{} GB", (bytes as f64 / GIB).ceil() as u64)
}

/// RAM in use: GiB with one decimal.
pub fn gb1(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / GIB)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_follow_the_handoffs_units() {
        assert_eq!(bytes(0), "0 KB");
        assert_eq!(bytes(999_999), "999 KB");
        assert_eq!(bytes(413_100_000), "413.1 MB");
        assert_eq!(bytes(2_790_000_000), "2.79 GB");
    }

    #[test]
    fn rates_split_into_number_and_unit() {
        assert_eq!(rate_parts(0.0), ("0".to_string(), "KB/s"));
        assert_eq!(rate_parts(-5.0), ("0".to_string(), "KB/s"));
        assert_eq!(rate_parts(f64::NAN), ("0".to_string(), "KB/s"));
        assert_eq!(rate_parts(41_000.0), ("41".to_string(), "KB/s"));
        assert_eq!(rate_parts(1_420_000.0), ("1.4".to_string(), "MB/s"));
        assert_eq!(rate(41_000.0), "41 KB/s");
        assert_eq!(rate(1_420_000.0), "1.4 MB/s");
    }

    #[test]
    fn uptime_reads_like_the_mock() {
        assert_eq!(uptime(5), "0m 05s");
        assert_eq!(uptime(742), "12m 22s");
        assert_eq!(uptime(4342), "1h 12m 22s");
    }

    #[test]
    fn handshake_and_account_labels() {
        assert_eq!(handshake(Some(21)), "hs 21s ago");
        assert_eq!(handshake(None), "hs —");
        assert_eq!(accounts_label(1), "Account connected");
        assert_eq!(accounts_label(2), "Accounts connected");
    }

    #[test]
    fn nostromo_readouts() {
        // KB/s are whole numbers; negative/NaN read 0.
        assert_eq!(kb_int(0.0), "0");
        assert_eq!(kb_int(8_999.0), "8");
        assert_eq!(kb_int(1_420_000.0), "1420");
        assert_eq!(kb_int(f64::NAN), "0");
        // Totals: MB with one decimal, however large.
        assert_eq!(mb1(0), "0.0 MB");
        assert_eq!(mb1(105_800_000), "105.8 MB");
        assert_eq!(mb1(2_790_000_000), "2790.0 MB");
    }

    /// Handoff: `T+ {h}H {mm}M` from an hour, `T+ {m}M {ss}S` from a
    /// minute, `T+ {s}S` below.
    #[test]
    fn the_mission_clock() {
        assert_eq!(mission_clock(4), "T+ 4S");
        assert_eq!(mission_clock(65), "T+ 1M 05S");
        assert_eq!(mission_clock(3_599), "T+ 59M 59S");
        assert_eq!(mission_clock(5_880), "T+ 1H 38M");
        assert_eq!(mission_clock(36_000 + 60 * 7), "T+ 10H 07M");
    }

    /// RAM: the total rounds *up* to whole GiB (`MemTotal` is always a
    /// little under the installed size), usage has one decimal.
    #[test]
    fn memory_sizes() {
        assert_eq!(gb_ceil(33_438_498_816), "32 GB"); // MemTotal of a 32 GiB box
        assert_eq!(gb1(10_952_166_604), "10.2 GB");
        assert_eq!(gb1(0), "0.0 GB");
    }

    #[test]
    fn the_build_hash_is_baked_in() {
        let b = env!("YUTANI_BUILD");
        assert!(b == "UNKNOWN" || (b.len() == 7 && b.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())), "{b}");
    }
}
