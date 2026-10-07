//! `YUTANI_BUILD`: the short commit hash, upper-cased, for the popover's
//! footer ("YUTANI OS · BUILD 8A03B1F"). `UNKNOWN` outside a git checkout.

fn main() {
    let hash = std::process::Command::new("git")
        .args(["rev-parse", "--short=7", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_uppercase())
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "UNKNOWN".to_string());
    println!("cargo:rustc-env=YUTANI_BUILD={hash}");
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs/heads");
}
