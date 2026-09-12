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
    fn totals_use_the_handoff_ladder() {
        assert_eq!(bytes(0), "0 KB");
        assert_eq!(bytes(999), "0 KB");
        assert_eq!(bytes(222_000), "222 KB");
        assert_eq!(bytes(999_999), "999 KB");
        assert_eq!(bytes(1_000_000), "1.0 MB");
        assert_eq!(bytes(413_100_000), "413.1 MB");
        assert_eq!(bytes(999_999_999), "1000.0 MB");
        assert_eq!(bytes(1_000_000_000), "1.00 GB");
        assert_eq!(bytes(2_790_000_000), "2.79 GB");
    }

    #[test]
    fn rates_use_the_handoff_ladder() {
        assert_eq!(rate(0.0), "0 KB/s");
        assert_eq!(rate(-5.0), "0 KB/s");
        assert_eq!(rate(222_000.0), "222 KB/s");
        assert_eq!(rate(999_999.0), "999 KB/s");
        assert_eq!(rate(1_000_000.0), "1.0 MB/s");
        assert_eq!(rate(2_500_000.0), "2.5 MB/s");
    }

    #[test]
    fn handshake_age_reads_as_the_handoff_writes_it() {
        assert_eq!(handshake(Some(0)), "hs 0s ago");
        assert_eq!(handshake(Some(21)), "hs 21s ago");
        assert_eq!(handshake(Some(112)), "hs 112s ago");
        assert_eq!(handshake(None), "hs —");
    }

    #[test]
    fn the_accounts_label_is_singular_for_one() {
        assert_eq!(accounts_label(0), "Accounts connected");
        assert_eq!(accounts_label(1), "Account connected");
        assert_eq!(accounts_label(2), "Accounts connected");
    }
}
