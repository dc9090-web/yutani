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
}
