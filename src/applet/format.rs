//! Number formatting for the Nostromo popover's HOST card.

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
