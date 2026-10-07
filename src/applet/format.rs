//! Number and label formatting for the Nostromo popover: decimal SI units
//! (matching `wg`'s human output style). Sub-unit values truncate rather
//! than round, so a rate of 999 999 B/s reads `999`, never `1000`.

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
        assert!(b == "UNKNOWN" || (b.len() >= 7 && b.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())), "{b}");
    }
}
