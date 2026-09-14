//! Dates as the settings window prints them, in local time through
//! `libc::localtime_r` (no time-zone crate; the C library already knows).

use std::time::{SystemTime, UNIX_EPOCH};

const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/// Local broken-down time: `(year, month 1-12, day, hour, minute)`.
fn local(t: SystemTime) -> (i32, u32, u32, u32, u32) {
    let secs = t.duration_since(UNIX_EPOCH).map(|d| d.as_secs() as libc::time_t).unwrap_or(0);
    // SAFETY: `localtime_r` writes only into the `tm` we hand it.
    let tm = unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        tm
    };
    (tm.tm_year + 1900, (tm.tm_mon + 1) as u32, tm.tm_mday as u32, tm.tm_hour as u32, tm.tm_min as u32)
}

/// `12 Sep`.
pub fn short_date(t: SystemTime) -> String {
    let (_, month, day, _, _) = local(t);
    format!("{day} {}", MONTHS[(month as usize - 1).min(11)])
}

/// `13 Sep 2026, 05:38`.
pub fn date_time(t: SystemTime) -> String {
    let (year, month, day, hour, minute) = local(t);
    format!("{day} {} {year}, {hour:02}:{minute:02}", MONTHS[(month as usize - 1).min(11)])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Formats are pinned in shape; the exact day depends on the machine's
    /// zone, so the check is structural.
    #[test]
    fn dates_read_like_the_handoff() {
        // 2026-09-13T05:38:07Z
        let t = UNIX_EPOCH + Duration::from_secs(1_789_191_487);
        let short = short_date(t);
        assert!(short.ends_with(" Sep"), "{short}");
        let full = date_time(t);
        assert!(full.contains(" Sep 2026, "), "{full}");
        assert_eq!(full.len(), "13 Sep 2026, 05:38".len(), "{full}");
        assert_eq!(short_date(UNIX_EPOCH).split(' ').count(), 2);
    }
}
