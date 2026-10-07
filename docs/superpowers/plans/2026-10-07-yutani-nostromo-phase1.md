# Nostromo popover — Phase 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the applet popover with the v6 "Nostromo" skin, fed by every datum the daemon already reports, plus host CPU/GPU/RAM gauges. Ping is shown idle, and Launch EVE is hidden; both are later phases.

**Architecture:**
- **Library (`src/applet/`).** Pure, unit-tested modules: fonts and glyph metrics, formatting, the host sampler, the ping window, the sand-field simulation, the rocker maths, the Nostromo skin, and the `Console` view model.
- **Binary (`src/bin/yutani-applet/`).** It only renders `Console`. Animation is self-driven inside iced `canvas` programs (`Action::request_redraw` on `RedrawRequested`), so nothing re-runs `view()` per frame.
- **Swap order.** The old popover keeps working until Task 8 swaps the popup to the new view. Task 10 then deletes the old code.

**Tech Stack:** Rust 2024, libcosmic `a401af8` (iced fork with `canvas`), `ttf-parser` 0.25 (already in `Cargo.lock`), fonttools (`pyftsubset`, `fonttools varLib.instancer`) for the font subset — run once, outputs committed.

**Specs:**
- `docs/superpowers/specs/2026-10-07-yutani-nostromo-design.md` (decisions);
- `docs/superpowers/specs/2026-10-07-yutani-nostromo-handoff.md` (the **handoff**: every size, colour and copy string).

**Reference renders:** `temp/v2.zip` → `design_handoff_yutani_v6_nostromo/previews/01–05.png`.

## Global Constraints

- **Never run `cargo fmt`.** The repo is not rustfmt-clean. Match the surrounding style by hand.
- **Tests:** `cargo test --lib` and `cargo test --bin yutani-applet` must pass after every task.
- **Clippy:** `cargo clippy --all-targets` must be warning-free after every task.
- **Commit trailer:** every commit ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. Keep that exact line; do not substitute your own model name.
- **Palette** (fixed; never read from the COSMIC theme inside the popover):

  | Token | Hex |
  |---|---|
  | `BG` | `#050907` |
  | `CARD` | `#081109` |
  | `LINE` | `#173b22` |
  | `LINE_2` | `#245233` |
  | `PHOSPHOR` | `#7ce38b` |
  | `DIM` | `#4f9c5c` |
  | `DIMMER` | `#3c7a47` |
  | `WHITE` | `#e9f5e6` |
  | `AMBER` | `#ffb000` |
  | `RED` | `#ff4a3d` |

- **Copy:** all popover copy is UPPERCASE, as in the handoff. Account names are shown upper-cased.
- **Nothing round:** radii are 4 (popover), 2 (cards, buttons), 1 (chips, rows, LEDs, cells).
- **Popover width:** 360 (`POPOVER_WIDTH`).
- **Phase 1 scope:**
  - Ping always renders IDLE with "—" values (the `PingWindow` exists but is never fed).
  - `LaunchButton::Hidden` is the only launch state produced.
- **Known deviations from the handoff, accepted:**
  - No radial vignette (iced canvas has no radial gradients). Scanlines and the 70 % phosphor top line are kept.
  - No text glow on the big readouts.
  - Corner brackets sit on the card edge, not 1 px outside it (iced clips children to their bounds).

---

## File map

| File | Status | Responsibility |
|---|---|---|
| `assets/fonts/*.ttf`, `assets/fonts/OFL-*.txt` | create | vendored fonts + licences |
| `src/applet/fonts.rs` | create | font bytes, `Font` handles, load task, per-glyph advances for tracking |
| `src/applet/format.rs` | modify | add `kb_int`, `mb1`, `mission_clock`, `gb_ceil`, `gb1` |
| `build.rs` | create | `YUTANI_BUILD` env (short git hash, upper-case) |
| `src/applet/host.rs` | create | `/proc` + sysfs parsers, `Sources::discover`, `HostSampler` |
| `src/applet/ping.rs` | create | 34-sample `PingWindow`, `PingSummary`, `Quality` |
| `src/applet/sand.rs` | create | the sand-field simulation (pure) |
| `src/applet/rocker.rs` | create | rocker knob position maths |
| `src/applet/skin.rs` | create | Nostromo palette, geometry, type scale, style classes |
| `src/applet/console.rs` | create | `Console` view model built from `Status` + local inputs |
| `src/applet/mod.rs` | modify | declare the new modules |
| `src/bin/yutani-applet/widgets.rs` | create | canvas programs: `Scope`, `Sparkline`, `Rocker`, `DottedRule`, `Brackets`, `Glass`, `Cursor` |
| `src/bin/yutani-applet/console_view.rs` | create | renders `Console` |
| `src/bin/yutani-applet/app.rs` | modify | fonts load, host sampling, totals, service pending, popup → `console_view` |
| `src/applet/theme.rs` | modify (Task 9) | square tray badge; (Task 10) drop popover-only items |
| `src/applet/display.rs`, `src/applet/history.rs`, `src/bin/yutani-applet/view.rs` | delete/trim (Task 10) | the v5 popover |

---

### Task 1: Vendored fonts and glyph metrics

**Files:**
- Create: `assets/fonts/B612Mono-Regular.ttf`, `assets/fonts/B612Mono-Bold.ttf`, `assets/fonts/Michroma-Regular.ttf`, `assets/fonts/NotoSansJP-Yutani.ttf`, `assets/fonts/OFL-B612.txt`, `assets/fonts/OFL-Michroma.txt`, `assets/fonts/OFL-NotoSansJP.txt`, `assets/fonts/README.md`
- Create: `src/applet/fonts.rs`
- Modify: `src/applet/mod.rs` (add `pub mod fonts;`), `Cargo.toml` (add `ttf-parser = "0.25"`)

**Interfaces:**
- Produces:
  - `fonts::Face { Mono, MonoBold, Display, Jp }`;
  - `fonts::font(Face) -> cosmic::iced::Font`;
  - `fonts::advance_em(Face, char) -> f32`;
  - `fonts::load_all() -> cosmic::iced::Task<bool>` (true = all loaded);
  - `fonts::set_missing()`;
  - `fonts::ALL: [&[u8]; 4]`.

- [ ] **Step 1: Fetch and prepare the fonts.**
  Fonttools goes in a throwaway venv in the session scratchpad, not the repo.

```bash
cd ~/Yutani
SCR=$(mktemp -d)
python3 -m venv "$SCR/venv" && "$SCR/venv/bin/pip" -q install fonttools
G=https://github.com/google/fonts/raw/main/ofl
mkdir -p assets/fonts
curl -sSfL -o assets/fonts/B612Mono-Regular.ttf "$G/b612mono/B612Mono-Regular.ttf"
curl -sSfL -o assets/fonts/B612Mono-Bold.ttf    "$G/b612mono/B612Mono-Bold.ttf"
curl -sSfL -o assets/fonts/OFL-B612.txt         "$G/b612mono/OFL.txt"
curl -sSfL -o assets/fonts/Michroma-Regular.ttf "$G/michroma/Michroma-Regular.ttf"
curl -sSfL -o assets/fonts/OFL-Michroma.txt     "$G/michroma/OFL.txt"
curl -sSfL -o "$SCR/NotoSansJP.ttf"             "$G/notosansjp/NotoSansJP%5Bwght%5D.ttf"
curl -sSfL -o assets/fonts/OFL-NotoSansJP.txt   "$G/notosansjp/OFL.txt"
# Pin the variable font to weight 500, then keep only the five glyphs the
# subline uses.
"$SCR/venv/bin/fonttools" varLib.instancer "$SCR/NotoSansJP.ttf" wght=500 -o "$SCR/NotoSansJP-500.ttf"
"$SCR/venv/bin/pyftsubset" "$SCR/NotoSansJP-500.ttf" --text="ユタニ重工" \
  --layout-features='*' --name-IDs='*' --output-file=assets/fonts/NotoSansJP-Yutani.ttf
ls -l assets/fonts
```

  Expected: four `.ttf` files. `NotoSansJP-Yutani.ttf` must be under 20 KB.

  Then write `assets/fonts/README.md`:

```markdown
# Vendored fonts (SIL OFL 1.1 — see the OFL-*.txt files)

- B612 Mono Regular/Bold, Michroma Regular — unmodified, from github.com/google/fonts (ofl/).
- NotoSansJP-Yutani.ttf — Noto Sans JP instanced at wght=500 and subset to
  "ユタニ重工" with fonttools (`varLib.instancer`, `pyftsubset`). It is a
  Modified Version under the OFL; the family name is unchanged as the OFL
  permits for fonts without a Reserved Font Name.
```

- [ ] **Step 2: Check the family names the fonts declare.**
  `Font { family: Family::Name(..) }` must match these exactly.

```bash
"$SCR/venv/bin/python" - <<'EOF'
from fontTools.ttLib import TTFont
for f in ["B612Mono-Regular","B612Mono-Bold","Michroma-Regular","NotoSansJP-Yutani"]:
    t = TTFont(f"assets/fonts/{f}.ttf"); n = t["name"]
    print(f, "|", n.getDebugName(1), "|", n.getDebugName(2), "|", t["OS/2"].usWeightClass)
EOF
```

  Expected (family | subfamily | weight):
  - `B612 Mono | Regular | 400`
  - `B612 Mono | Bold | 700`
  - `Michroma | Regular | 400`
  - `Noto Sans JP | … | 500`

  If any family differs, use the printed family in Step 4's `FAMILY_*` constants.

- [ ] **Step 3: Write the failing test** in `src/applet/fonts.rs`.

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_parses_and_has_the_glyphs_it_is_used_for() {
        for face in [Face::Mono, Face::MonoBold] {
            for ch in "YUTANI 0123456789·—▲▼◄▶…%°/+:.".chars() {
                assert!(advance_em(face, ch) > 0.0, "{face:?} {ch:?}");
            }
        }
        for ch in "YUTANI01234".chars() {
            assert!(advance_em(Face::Display, ch) > 0.0, "{ch:?}");
        }
        for ch in "ユタニ重工".chars() {
            assert!(advance_em(Face::Jp, ch) > 0.0, "{ch:?}");
        }
    }

    /// B612 Mono is monospaced: every digit has the same advance, so the
    /// readouts never jitter.
    #[test]
    fn the_mono_face_is_monospaced() {
        let w = advance_em(Face::Mono, '0');
        for ch in "123456789ABCXYZ".chars() {
            assert!((advance_em(Face::Mono, ch) - w).abs() < 1e-6, "{ch:?}");
        }
    }

    /// A glyph a face lacks falls back to the advance of `0` rather than
    /// collapsing to zero width.
    #[test]
    fn a_missing_glyph_falls_back_to_a_digit_width() {
        assert_eq!(advance_em(Face::Jp, 'Q'), advance_em(Face::Jp, '0').max(advance_em(Face::Mono, '0')));
    }

    #[test]
    fn handles_name_the_bundled_families() {
        assert_eq!(font(Face::MonoBold).weight, cosmic::iced::font::Weight::Bold);
        assert_eq!(font(Face::Mono).family, cosmic::iced::font::Family::Name(FAMILY_MONO));
        assert_eq!(ALL.len(), 4);
    }
}
```

- [ ] **Step 4: Run it** — `cargo test --lib applet::fonts` → FAIL (module missing).

- [ ] **Step 5: Implement `src/applet/fonts.rs`.**

```rust
//! The popover's bundled faces (Nostromo handoff "Typography"): B612 Mono
//! for everything, Michroma for the wordmark and the section indices, and a
//! five-glyph Noto Sans JP for the one Japanese subline. Loaded once at
//! applet start; should a load fail, every face falls back to COSMIC's
//! monospace (`set_missing`).
//!
//! iced has no letter-spacing, so the view tracks text itself, one glyph per
//! fixed-width cell. `advance_em` is the glyph's own advance for that.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

use cosmic::iced::Font;
use cosmic::iced::font::{Family, Weight};

pub const B612_MONO: &[u8] = include_bytes!("../../assets/fonts/B612Mono-Regular.ttf");
pub const B612_MONO_BOLD: &[u8] = include_bytes!("../../assets/fonts/B612Mono-Bold.ttf");
pub const MICHROMA: &[u8] = include_bytes!("../../assets/fonts/Michroma-Regular.ttf");
pub const NOTO_JP: &[u8] = include_bytes!("../../assets/fonts/NotoSansJP-Yutani.ttf");
pub const ALL: [&[u8]; 4] = [B612_MONO, B612_MONO_BOLD, MICHROMA, NOTO_JP];

pub const FAMILY_MONO: &str = "B612 Mono";
pub const FAMILY_DISPLAY: &str = "Michroma";
pub const FAMILY_JP: &str = "Noto Sans JP";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Face {
    Mono,
    MonoBold,
    Display,
    Jp,
}

static MISSING: AtomicBool = AtomicBool::new(false);

/// A font failed to load: from now on every face is COSMIC's monospace.
pub fn set_missing() {
    MISSING.store(true, Ordering::Relaxed);
}

pub fn font(face: Face) -> Font {
    if MISSING.load(Ordering::Relaxed) {
        let mono = cosmic::font::mono();
        return if face == Face::MonoBold { Font { weight: Weight::Bold, ..mono } } else { mono };
    }
    match face {
        Face::Mono => Font { family: Family::Name(FAMILY_MONO), ..Font::DEFAULT },
        Face::MonoBold => Font { family: Family::Name(FAMILY_MONO), weight: Weight::Bold, ..Font::DEFAULT },
        Face::Display => Font { family: Family::Name(FAMILY_DISPLAY), ..Font::DEFAULT },
        Face::Jp => Font { family: Family::Name(FAMILY_JP), weight: Weight::Medium, ..Font::DEFAULT },
    }
}

fn bytes(face: Face) -> &'static [u8] {
    match face {
        Face::Mono => B612_MONO,
        Face::MonoBold => B612_MONO_BOLD,
        Face::Display => MICHROMA,
        Face::Jp => NOTO_JP,
    }
}

fn parsed(face: Face) -> Option<&'static ttf_parser::Face<'static>> {
    static FACES: [OnceLock<Option<ttf_parser::Face<'static>>>; 4] =
        [OnceLock::new(), OnceLock::new(), OnceLock::new(), OnceLock::new()];
    FACES[face as usize].get_or_init(|| ttf_parser::Face::parse(bytes(face), 0).ok()).as_ref()
}

fn raw_advance(face: Face, ch: char) -> Option<f32> {
    let f = parsed(face)?;
    let gid = f.glyph_index(ch)?;
    Some(f32::from(f.glyph_hor_advance(gid)?) / f32::from(f.units_per_em()))
}

/// `ch`'s advance in em. A glyph the face lacks (it will render in a
/// fallback font) takes the wider of this face's and B612 Mono's `0`, so a
/// tracked cell is never narrower than what lands in it.
pub fn advance_em(face: Face, ch: char) -> f32 {
    raw_advance(face, ch).unwrap_or_else(|| {
        raw_advance(face, '0').unwrap_or(0.0).max(raw_advance(Face::Mono, '0').unwrap_or(0.6))
    })
}

/// Load every bundled face; `true` when all of them loaded.
pub fn load_all() -> cosmic::iced::Task<bool> {
    let tasks = ALL.map(|b| cosmic::iced::font::load(b).map(|r| r.is_ok()));
    cosmic::iced::Task::batch(tasks).collect().map(|oks: Vec<bool>| oks.into_iter().all(|ok| ok))
}
```

  Add `pub mod fonts;` to `src/applet/mod.rs` (alphabetical, after `pub mod display;`). Add `ttf-parser = "0.25"` under `[dependencies]` in `Cargo.toml`, with the comment `# Glyph advances for the popover's tracked text (already in the tree via cosmic-text).`

  > If `Task::collect` does not exist in this iced fork: check `grep -n "pub fn collect" ~/.cargo/git/checkouts/libcosmic-*/a401af8/iced/runtime/src/task.rs`. Otherwise return `Task<bool>` from `Task::batch(tasks)` and let the app treat each `false` with `set_missing()` (Task 8 handles each message individually).

- [ ] **Step 6: Run the tests.** `cargo test --lib applet::fonts` → PASS. Then `cargo clippy --all-targets` → clean.

- [ ] **Step 7: Commit.**

```bash
git add assets/fonts src/applet/fonts.rs src/applet/mod.rs Cargo.toml Cargo.lock
git commit -m "feat(applet): bundle B612 Mono, Michroma and a Noto Sans JP subset

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Formatting helpers and the build hash

**Files:**
- Modify: `src/applet/format.rs` (add functions and tests; keep the existing ones until Task 10)
- Create: `build.rs`

**Interfaces:**
- Produces:
  - `format::kb_int(f64) -> String`;
  - `format::mb1(u64) -> String`;
  - `format::mission_clock(u64) -> String`;
  - `format::gb_ceil(u64) -> String`;
  - `format::gb1(u64) -> String`;
  - `env!("YUTANI_BUILD")`.

- [ ] **Step 1: Write the failing tests.** Append them to `format.rs`'s `tests` module.

```rust
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
```

- [ ] **Step 2: Run it** — `cargo test --lib applet::format` → FAIL (not defined).

- [ ] **Step 3: Implement.** Add to `format.rs`, above `#[cfg(test)]`:

```rust
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
```

  Create `build.rs` at the repo root:

```rust
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
```

  And this test in `format.rs` tests:

```rust
    #[test]
    fn the_build_hash_is_baked_in() {
        let b = env!("YUTANI_BUILD");
        assert!(b == "UNKNOWN" || (b.len() == 7 && b.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase())), "{b}");
    }
```

- [ ] **Step 4: Run the tests.** `cargo test --lib applet::format` → PASS. Then `cargo clippy --all-targets` → clean.

- [ ] **Step 5: Commit.**
  Message: `feat(applet): Nostromo number formats and the build hash`, with the trailer.

---

### Task 3: Host sampler (CPU · GPU · RAM)

**Files:**
- Create: `src/applet/host.rs`
- Modify: `src/applet/mod.rs` (`pub mod host;`)

**Interfaces:**
- Produces:

```rust
pub struct HostReading { pub cpu: Option<f32>, pub gpu: Option<f32>, pub ram_used: Option<u64>, pub ram_total: Option<u64>, pub cpu_temp: Option<f32>, pub gpu_temp: Option<f32> }  // Clone, Copy, Debug, Default, PartialEq
pub struct Sources { .. }  // Sources::discover() / Sources::discover_in(sys: &Path, proc_: &Path)
pub struct HostSampler     // HostSampler::new(Sources), .sample(&mut self) -> HostReading
pub fn parse_cpu_times(stat: &str) -> Option<(u64, u64)>          // (busy, total)
pub fn cpu_percent(prev: (u64, u64), now: (u64, u64)) -> Option<f32>
pub fn parse_meminfo(s: &str) -> Option<(u64, u64)>                // (used, total) bytes
pub fn parse_number(s: &str) -> Option<f32>                         // trimmed f32
pub fn ease(prev: Option<f32>, target: Option<f32>) -> Option<f32> // 20 % per tick
```

- [ ] **Step 1: Write the failing tests** (the bottom of `host.rs`).

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &str = "cpu  100 20 50 800 30 0 10 0 0 0\ncpu0 1 2 3 4 5 6 7 8 0 0\nintr 1\n";

    #[test]
    fn cpu_times_count_iowait_as_idle() {
        // busy = user+nice+system+irq+softirq+steal = 100+20+50+0+10+0
        assert_eq!(parse_cpu_times(STAT), Some((180, 1_010)));
        assert_eq!(parse_cpu_times("intr 1\n"), None);
    }

    #[test]
    fn cpu_percent_is_the_busy_share_of_the_delta() {
        assert_eq!(cpu_percent((180, 1_010), (230, 1_110)), Some(50.0));
        assert_eq!(cpu_percent((180, 1_010), (180, 1_010)), None, "no time passed");
        assert_eq!(cpu_percent((180, 1_010), (100, 900)), None, "counters went backwards");
    }

    #[test]
    fn meminfo_used_is_total_minus_available() {
        let s = "MemTotal:       32654784 kB\nMemFree:  1 kB\nMemAvailable:   21959148 kB\n";
        assert_eq!(parse_meminfo(s), Some(((32_654_784 - 21_959_148) * 1024, 32_654_784 * 1024)));
        assert_eq!(parse_meminfo("MemTotal: 5 kB\n"), None);
    }

    #[test]
    fn numbers_and_easing() {
        assert_eq!(parse_number("40\n"), Some(40.0));
        assert_eq!(parse_number("x"), None);
        assert_eq!(ease(None, Some(50.0)), Some(50.0), "the first reading is taken as is");
        assert_eq!(ease(Some(0.0), Some(50.0)), Some(10.0));
        assert_eq!(ease(Some(10.0), None), None, "a source that vanished reads as missing");
    }

    fn write(p: &std::path::Path, s: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    }

    /// A fake /sys + /proc: amdgpu busy + edge temp, k10temp Tctl.
    #[test]
    fn discovery_and_sampling_on_a_fixture_tree() {
        let root = std::env::temp_dir().join(format!("yutani-host-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (sys, proc_) = (root.join("sys"), root.join("proc"));
        write(&sys.join("class/drm/card1/device/gpu_busy_percent"), "40\n");
        write(&sys.join("class/hwmon/hwmon2/name"), "amdgpu\n");
        write(&sys.join("class/hwmon/hwmon2/temp1_input"), "52000\n");
        write(&sys.join("class/hwmon/hwmon3/name"), "k10temp\n");
        write(&sys.join("class/hwmon/hwmon3/temp1_input"), "46125\n");
        write(&sys.join("class/hwmon/hwmon4/name"), "nvme\n");
        write(&proc_.join("stat"), STAT);
        write(&proc_.join("meminfo"), "MemTotal: 1000 kB\nMemAvailable: 750 kB\n");

        let mut s = HostSampler::new(Sources::discover_in(&sys, &proc_));
        let r = s.sample();
        assert_eq!(r.cpu, None, "CPU needs two samples");
        assert_eq!(r.gpu, Some(40.0));
        assert_eq!((r.ram_used, r.ram_total), (Some(250 * 1024), Some(1000 * 1024)));
        assert_eq!(r.cpu_temp, Some(46.125));
        assert_eq!(r.gpu_temp, Some(52.0));

        write(&proc_.join("stat"), "cpu  150 20 50 850 30 0 10 0 0 0\n");
        let r = s.sample();
        assert_eq!(r.cpu, Some(50.0), "first CPU reading is taken as is");

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_machine_without_gpu_or_sensors_reads_none() {
        let root = std::env::temp_dir().join(format!("yutani-host-bare-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let mut s = HostSampler::new(Sources::discover_in(&root.join("sys"), &root.join("proc")));
        assert_eq!(s.sample(), HostReading::default());
    }
}
```

- [ ] **Step 2: Run it** — `cargo test --lib applet::host` → FAIL.

- [ ] **Step 3: Implement** (above the tests).

```rust
//! The popover's HOST card (Nostromo handoff "04 · Host"): CPU, GPU and RAM
//! load with CPU/GPU temperatures, read straight from `/proc` and sysfs at
//! 1 Hz while the popover is open, eased 20 % per tick like the mock. Every
//! reading is optional: a missing source shows "—" and an unlit gauge.

use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HostReading {
    pub cpu: Option<f32>,
    pub gpu: Option<f32>,
    pub ram_used: Option<u64>,
    pub ram_total: Option<u64>,
    pub cpu_temp: Option<f32>,
    pub gpu_temp: Option<f32>,
}

/// `(busy, total)` jiffies from `/proc/stat`'s aggregate `cpu` line. Idle
/// and iowait are idle; the first eight fields are the total (guest time
/// is already inside user/nice).
pub fn parse_cpu_times(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|l| l.starts_with("cpu "))?;
    let f: Vec<u64> = line.split_whitespace().skip(1).take(8).map(|v| v.parse().ok()).collect::<Option<_>>()?;
    if f.len() < 8 {
        return None;
    }
    let total: u64 = f.iter().sum();
    Some((total - f[3] - f[4], total))
}

pub fn cpu_percent(prev: (u64, u64), now: (u64, u64)) -> Option<f32> {
    let total = now.1.checked_sub(prev.1).filter(|t| *t > 0)?;
    let busy = now.0.checked_sub(prev.0)?;
    Some((busy as f32 / total as f32 * 100.0).clamp(0.0, 100.0))
}

/// `(used, total)` bytes: `MemTotal − MemAvailable`.
pub fn parse_meminfo(s: &str) -> Option<(u64, u64)> {
    let field = |name: &str| {
        s.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|v| v.parse::<u64>().ok())
            .map(|kb| kb * 1024)
    };
    let (total, avail) = (field("MemTotal:")?, field("MemAvailable:")?);
    Some((total.saturating_sub(avail), total))
}

pub fn parse_number(s: &str) -> Option<f32> {
    s.trim().parse().ok()
}

/// The mock's smoothing: 20 % of the way to the new reading per tick.
pub fn ease(prev: Option<f32>, target: Option<f32>) -> Option<f32> {
    match (prev, target) {
        (Some(p), Some(t)) => Some(p + (t - p) * 0.2),
        (_, t) => t,
    }
}

/// Where the readings come from, found once at start.
#[derive(Clone, Debug, Default)]
pub struct Sources {
    stat: PathBuf,
    meminfo: PathBuf,
    gpu_busy: Option<PathBuf>,
    cpu_temp: Option<PathBuf>,
    gpu_temp: Option<PathBuf>,
}

/// hwmon drivers whose `temp1_input` is the CPU package temperature.
const CPU_HWMON: [&str; 3] = ["k10temp", "zenpower", "coretemp"];

impl Sources {
    pub fn discover() -> Self {
        Self::discover_in(Path::new("/sys"), Path::new("/proc"))
    }

    pub fn discover_in(sys: &Path, proc_: &Path) -> Self {
        let sorted = |dir: PathBuf| {
            let mut v: Vec<PathBuf> = std::fs::read_dir(dir).map(|d| d.flatten().map(|e| e.path()).collect()).unwrap_or_default();
            v.sort();
            v
        };
        let gpu_busy = sorted(sys.join("class/drm"))
            .into_iter()
            .map(|card| card.join("device/gpu_busy_percent"))
            .find(|p| p.is_file());
        let mut cpu_temp = None;
        let mut gpu_temp = None;
        for hw in sorted(sys.join("class/hwmon")) {
            let name = std::fs::read_to_string(hw.join("name")).unwrap_or_default();
            let input = hw.join("temp1_input");
            if !input.is_file() {
                continue;
            }
            match name.trim() {
                n if CPU_HWMON.contains(&n) && cpu_temp.is_none() => cpu_temp = Some(input),
                "amdgpu" if gpu_temp.is_none() => gpu_temp = Some(input),
                _ => {}
            }
        }
        Sources { stat: proc_.join("stat"), meminfo: proc_.join("meminfo"), gpu_busy, cpu_temp, gpu_temp }
    }
}

pub struct HostSampler {
    sources: Sources,
    prev_cpu: Option<(u64, u64)>,
    eased: HostReading,
}

fn read(p: &Path) -> Option<String> {
    std::fs::read_to_string(p).ok()
}

impl HostSampler {
    pub fn new(sources: Sources) -> Self {
        HostSampler { sources, prev_cpu: None, eased: HostReading::default() }
    }

    /// One 1 Hz reading, eased. Blocking file reads of a few bytes each.
    pub fn sample(&mut self) -> HostReading {
        let s = &self.sources;
        let times = read(&s.stat).as_deref().and_then(parse_cpu_times);
        let cpu = match (self.prev_cpu, times) {
            (Some(p), Some(n)) => cpu_percent(p, n),
            _ => None,
        };
        self.prev_cpu = times;
        let gpu = s.gpu_busy.as_deref().and_then(read).as_deref().and_then(parse_number);
        let mem = read(&s.meminfo).as_deref().and_then(parse_meminfo);
        let temp = |p: &Option<PathBuf>| p.as_deref().and_then(read).as_deref().and_then(parse_number).map(|m| m / 1000.0);
        let (cpu_temp, gpu_temp) = (temp(&s.cpu_temp), temp(&s.gpu_temp));
        let e = &mut self.eased;
        e.cpu = ease(e.cpu, cpu);
        e.gpu = ease(e.gpu, gpu);
        e.ram_used = mem.map(|m| m.0);
        e.ram_total = mem.map(|m| m.1);
        e.cpu_temp = cpu_temp;
        e.gpu_temp = gpu_temp;
        *e
    }
}
```

  Note: RAM and temperatures are not eased. The handoff eases the gauges, which are CPU and GPU. RAM changes slowly anyway, and temperatures are shown as text.

- [ ] **Step 4: Run the tests.** `cargo test --lib applet::host` → PASS. Then `cargo clippy --all-targets` → clean.
- [ ] **Step 5: Commit.**
  Message: `feat(applet): host load sampler from /proc and sysfs`, with the trailer.

---

### Task 4: Ping window

**Files:**
- Create: `src/applet/ping.rs`
- Modify: `src/applet/mod.rs` (`pub mod ping;`)

**Interfaces:**
- Produces:

```rust
pub const WINDOW: usize = 34;
pub const SPARK_W: f32 = 240.0; pub const SPARK_H: f32 = 22.0;
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Quality { Nominal, Degraded, Poor, Idle }  // Quality::word() -> &'static str
#[derive(Clone, Copy, Debug, PartialEq)] pub struct Dot { pub x: f32, pub y: f32, pub size: f32, pub opacity: f32 }
#[derive(Clone, Debug, PartialEq)] pub struct PingSummary { pub value: String, pub quality: Quality, pub stats: String, pub jitter_loss: String, pub dots: Vec<Dot>, pub avg_y: f32 }
#[derive(Clone, Debug, Default)] pub struct PingWindow { .. }
impl PingWindow { pub fn push_seq(&mut self, seq: u64, sample: Option<f32>); pub fn clear(&mut self); pub fn summary(&self, live: bool) -> PingSummary }
```

- [ ] **Step 1: Write the failing tests.**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn window(samples: &[Option<f32>]) -> PingWindow {
        let mut w = PingWindow::default();
        for (i, s) in samples.iter().enumerate() {
            w.push_seq(i as u64 + 1, *s);
        }
        w
    }

    #[test]
    fn a_down_tunnel_is_idle_with_dashes_and_dots_on_the_baseline() {
        let s = window(&[Some(5.0); 10]).summary(false);
        assert_eq!(s.quality, Quality::Idle);
        assert_eq!((s.value.as_str(), s.stats.as_str(), s.jitter_loss.as_str()), ("—", "MIN — · AVG — · MAX —", "JIT — · LOSS —"));
        assert!(s.dots.iter().all(|d| d.y == SPARK_H - 2.0 - d.size / 2.0));
        assert_eq!(PingWindow::default().summary(true).quality, Quality::Idle, "no samples yet");
    }

    #[test]
    fn stats_over_the_window() {
        let s = window(&[Some(3.0), Some(5.0), None, Some(12.0), Some(4.0)]).summary(true);
        assert_eq!(s.value, "4");
        assert_eq!(s.stats, "MIN 3 · AVG 6 · MAX 12");
        // jitter = mean |Δ| over consecutive successes: (2 + 7 + 8) / 3
        assert_eq!(s.jitter_loss, "JIT 5.7 · LOSS 20.0%");
        assert_eq!(s.quality, Quality::Nominal);
    }

    #[test]
    fn quality_boundaries_follow_the_latest_success() {
        let q = |ms: f32| window(&[Some(ms)]).summary(true).quality;
        assert_eq!(q(29.9), Quality::Nominal);
        assert_eq!(q(30.0), Quality::Degraded);
        assert_eq!(q(99.9), Quality::Degraded);
        assert_eq!(q(100.0), Quality::Poor);
        assert_eq!(window(&[None, None]).summary(true).quality, Quality::Poor, "all lost");
        assert_eq!(Quality::Nominal.word(), "NOMINAL");
    }

    #[test]
    fn the_window_keeps_34_and_ignores_a_repeated_sequence() {
        let mut w = window(&[Some(1.0); 40]);
        assert_eq!(w.summary(true).dots.len(), WINDOW);
        w.push_seq(40, Some(500.0));
        assert_eq!(w.summary(true).value, "1", "seq 40 was already taken");
        w.push_seq(41, Some(500.0));
        assert_eq!(w.summary(true).value, "500");
    }

    /// Newest at the right edge, 3 px and opaque; oldest 2 px at .3.
    #[test]
    fn dots_run_oldest_to_newest_left_to_right() {
        let s = window(&[Some(10.0); WINDOW]).summary(true);
        let (first, last) = (s.dots[0], s.dots[WINDOW - 1]);
        assert_eq!((first.x, first.size, first.opacity), (0.0, 2.0, 0.3));
        assert_eq!((last.x, last.size), (SPARK_W - 3.0, 3.0));
        assert!((last.opacity - 1.0).abs() < 1e-6);
        let short = window(&[Some(10.0); 2]).summary(true);
        assert_eq!(short.dots.last().unwrap().x, SPARK_W - 3.0, "a short window still ends at now");
    }
}
```

- [ ] **Step 2: Run it** — FAIL.
- [ ] **Step 3: Implement.**

```rust
//! The ping row (Nostromo handoff "Ping row"): the last 34 probes to
//! Tranquility, their stats, the quality word and the sparkline's dots.
//! Samples arrive with the daemon's probe sequence number (Phase 2); in
//! Phase 1 the window is never fed and the row reads IDLE.

use std::collections::VecDeque;

pub const WINDOW: usize = 34;
pub const SPARK_W: f32 = 240.0;
pub const SPARK_H: f32 = 22.0;
const DASH: &str = "—";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Quality {
    Nominal,
    Degraded,
    Poor,
    Idle,
}

impl Quality {
    pub fn word(self) -> &'static str {
        match self {
            Quality::Nominal => "NOMINAL",
            Quality::Degraded => "DEGRADED",
            Quality::Poor => "POOR",
            Quality::Idle => "IDLE",
        }
    }

    fn of(ms: f32) -> Self {
        if ms < 30.0 {
            Quality::Nominal
        } else if ms < 100.0 {
            Quality::Degraded
        } else {
            Quality::Poor
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dot {
    pub x: f32,
    pub y: f32,
    pub size: f32,
    pub opacity: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PingSummary {
    pub value: String,
    pub quality: Quality,
    pub stats: String,
    pub jitter_loss: String,
    pub dots: Vec<Dot>,
    /// The dashed average line's y inside the 240×22 box.
    pub avg_y: f32,
}

/// `None` samples are losses (the probe timed out or was refused).
#[derive(Clone, Debug, Default)]
pub struct PingWindow {
    samples: VecDeque<Option<f32>>,
    last_seq: Option<u64>,
}

fn baseline(size: f32) -> f32 {
    SPARK_H - 2.0 - size / 2.0
}

impl PingWindow {
    /// Take `sample` if `seq` is a probe this window has not seen.
    pub fn push_seq(&mut self, seq: u64, sample: Option<f32>) {
        if self.last_seq.is_some_and(|last| seq <= last) {
            return;
        }
        self.last_seq = Some(seq);
        if self.samples.len() == WINDOW {
            self.samples.pop_front();
        }
        self.samples.push_back(sample);
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    pub fn summary(&self, live: bool) -> PingSummary {
        let ok: Vec<f32> = self.samples.iter().flatten().copied().collect();
        let n = self.samples.len();
        let positions = |i: usize, size: f32| {
            // Right-aligned: the newest sample sits at slot 33.
            let slot = WINDOW - n + i;
            (slot as f32 * SPARK_W / (WINDOW - 1) as f32).min(SPARK_W - size)
        };
        if !live || n == 0 {
            let dots = (0..n)
                .map(|i| {
                    let size = if i == n - 1 { 3.0 } else { 2.0 };
                    let slot = WINDOW - n + i;
                    Dot { x: positions(i, size), y: baseline(size), size, opacity: 0.3 + 0.7 * slot as f32 / (WINDOW - 1) as f32 }
                })
                .collect();
            return PingSummary {
                value: DASH.into(),
                quality: Quality::Idle,
                stats: format!("MIN {DASH} · AVG {DASH} · MAX {DASH}"),
                jitter_loss: format!("JIT {DASH} · LOSS {DASH}"),
                dots,
                avg_y: SPARK_H - 2.0,
            };
        }
        let (min, max) = ok.iter().fold((f32::MAX, 0.0_f32), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
        let avg = if ok.is_empty() { 0.0 } else { ok.iter().sum::<f32>() / ok.len() as f32 };
        let jitter = if ok.len() > 1 { ok.windows(2).map(|w| (w[1] - w[0]).abs()).sum::<f32>() / (ok.len() - 1) as f32 } else { 0.0 };
        let loss = (n - ok.len()) as f32 / n as f32 * 100.0;
        let scale = max.max(1.0) * 1.15;
        let y_of = |v: f32| SPARK_H - 2.0 - v / scale * (SPARK_H - 5.0);
        let dots = self
            .samples
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let size = if i == n - 1 { 3.0 } else { 2.0 };
                let slot = WINDOW - n + i;
                let y = s.map_or(baseline(size), |v| y_of(v) - size / 2.0);
                Dot { x: positions(i, size), y, size, opacity: 0.3 + 0.7 * slot as f32 / (WINDOW - 1) as f32 }
            })
            .collect();
        let latest = ok.last().copied();
        let int = |v: f32| format!("{}", v.round() as i64);
        if ok.is_empty() {
            return PingSummary {
                value: DASH.into(),
                quality: Quality::Poor,
                stats: format!("MIN {DASH} · AVG {DASH} · MAX {DASH}"),
                jitter_loss: format!("JIT {DASH} · LOSS {loss:.1}%"),
                dots,
                avg_y: SPARK_H - 2.0,
            };
        }
        PingSummary {
            value: latest.map_or_else(|| DASH.into(), int),
            quality: latest.map_or(Quality::Poor, Quality::of),
            stats: format!("MIN {} · AVG {} · MAX {}", int(min), int(avg), int(max)),
            jitter_loss: format!("JIT {jitter:.1} · LOSS {loss:.1}%"),
            dots,
            avg_y: y_of(avg),
        }
    }
}
```

  Note: the newest *sample* is the value shown, which may be older than the newest probe if that probe was a loss. The quality word follows the same sample.

- [ ] **Step 4: Run the tests** → PASS. `cargo clippy --all-targets` → clean.
- [ ] **Step 5: Commit.**
  Message: `feat(applet): 34-sample ping window and its summary`, with the trailer.

---

### Task 5: Sand field simulation and rocker maths

**Files:**
- Create: `src/applet/sand.rs`, `src/applet/rocker.rs`
- Modify: `src/applet/mod.rs` (`pub mod rocker; pub mod sand;`)

**Interfaces:**
- Produces:

```rust
// sand.rs
pub const GRAINS: usize = 1000;
#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum Tint { Downlink, Uplink, Spark }
#[derive(Clone, Copy, Debug, PartialEq)] pub struct Drive { pub live: bool, pub up_kbps: f32, pub down_kbps: f32 }
pub struct Sand;  // Sand::new(w, h, seed), .resize(w, h), .step(dt_s, Drive), .dots(Drive) -> impl Iterator<Item = (f32, f32, f32, Tint, u8)>
pub fn wind_target(d: Drive) -> f32
pub fn up_share(d: Drive) -> f32
// rocker.rs
pub const KNOB_MS: u64 = 120;
pub fn knob_left(on: bool, since_flip: Option<std::time::Duration>) -> f32
```

- [ ] **Step 1: Write the failing tests.**

  `sand.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const LIVE: Drive = Drive { live: true, up_kbps: 30.0, down_kbps: 70.0 };
    const DOWN: Drive = Drive { live: false, up_kbps: 0.0, down_kbps: 0.0 };

    #[test]
    fn wind_follows_throughput_and_dies_with_the_tunnel() {
        assert_eq!(wind_target(DOWN), 2.0);
        assert_eq!(wind_target(Drive { live: true, up_kbps: 0.0, down_kbps: 0.0 }), 10.0);
        assert!((wind_target(LIVE) - (10.0 + 100.0 * 0.85)).abs() < 1e-4);
        assert!((wind_target(Drive { live: true, up_kbps: 900.0, down_kbps: 900.0 }) - (10.0 + 120.0 * 0.85)).abs() < 1e-4, "capped at 120");
        assert_eq!(up_share(LIVE), 0.3);
        assert_eq!(up_share(DOWN), 0.3, "idle: 30 % amber");
        assert_eq!(up_share(Drive { live: true, up_kbps: 0.0, down_kbps: 0.0 }), 0.3);
    }

    #[test]
    fn a_seed_reproduces_the_field() {
        let (mut a, mut b) = (Sand::new(338.0, 104.0, 7), Sand::new(338.0, 104.0, 7));
        for _ in 0..10 {
            a.step(0.016, LIVE);
            b.step(0.016, LIVE);
        }
        assert_eq!(a.dots(LIVE).collect::<Vec<_>>(), b.dots(LIVE).collect::<Vec<_>>());
        assert_eq!(a.dots(LIVE).count(), GRAINS);
    }

    #[test]
    fn grains_blow_right_and_respawn_at_the_left_edge() {
        let mut s = Sand::new(338.0, 104.0, 1);
        for _ in 0..2_000 {
            s.step(0.05, Drive { live: true, up_kbps: 500.0, down_kbps: 500.0 });
        }
        assert!(s.dots(LIVE).all(|(x, _, _, _, _)| (-2.0..=340.0).contains(&x)), "nothing escapes past W + 2");
    }

    #[test]
    fn a_long_frame_is_capped_at_50_ms() {
        let (mut a, mut b) = (Sand::new(338.0, 104.0, 3), Sand::new(338.0, 104.0, 3));
        a.step(5.0, LIVE);
        b.step(0.05, LIVE);
        assert_eq!(a.dots(LIVE).collect::<Vec<_>>(), b.dots(LIVE).collect::<Vec<_>>());
    }

    #[test]
    fn alpha_levels_are_quantised_and_dim_when_down() {
        let mut s = Sand::new(338.0, 104.0, 5);
        s.step(0.016, LIVE);
        assert!(s.dots(LIVE).all(|(.., q)| q <= 10));
        let live_sum: u32 = s.dots(LIVE).map(|(.., q)| u32::from(q)).sum();
        let down_sum: u32 = s.dots(DOWN).map(|(.., q)| u32::from(q)).sum();
        assert!(down_sum < live_sum, "down grains are at 45 %");
    }

    #[test]
    fn tints_split_by_uplink_share_with_rare_sparks() {
        let s = Sand::new(338.0, 104.0, 11);
        let all_up = Drive { live: true, up_kbps: 10.0, down_kbps: 0.0 };
        let all_down = Drive { live: true, up_kbps: 0.0, down_kbps: 10.0 };
        assert!(s.dots(all_down).all(|(.., t, _)| t != Tint::Uplink));
        assert!(s.dots(all_up).all(|(.., t, _)| t != Tint::Downlink));
        let sparks = s.dots(LIVE).filter(|(.., t, _)| *t == Tint::Spark).count();
        assert!(sparks > 5 && sparks < 60, "≈2.5 %: {sparks}");
    }

    #[test]
    fn resize_reseeds_only_on_a_real_change() {
        let mut s = Sand::new(338.0, 104.0, 2);
        s.step(0.016, LIVE);
        let before: Vec<_> = s.dots(LIVE).collect();
        s.resize(338.0, 104.0);
        assert_eq!(before, s.dots(LIVE).collect::<Vec<_>>());
        s.resize(300.0, 104.0);
        assert!(s.dots(LIVE).all(|(x, ..)| x <= 302.0));
    }
}
```

  `rocker.rs` tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn the_knob_rests_at_2_off_and_21_on() {
        assert_eq!(knob_left(false, None), 2.0);
        assert_eq!(knob_left(true, None), 21.0);
    }

    #[test]
    fn the_knob_travels_in_120_ms() {
        assert_eq!(knob_left(true, Some(Duration::ZERO)), 2.0);
        assert_eq!(knob_left(true, Some(Duration::from_millis(60))), 11.5);
        assert_eq!(knob_left(true, Some(Duration::from_millis(120))), 21.0);
        assert_eq!(knob_left(true, Some(Duration::from_secs(9))), 21.0);
        assert_eq!(knob_left(false, Some(Duration::from_millis(60))), 11.5);
    }
}
```

- [ ] **Step 2: Run them** — FAIL.
- [ ] **Step 3: Implement `rocker.rs`.**

```rust
//! The 38×19 square rocker (Nostromo handoff "Rocker"): a 13 px knob at
//! `left 2` (off) or `left 21` (on), sliding between them in 120 ms.

use std::time::Duration;

pub const KNOB_MS: u64 = 120;
const OFF: f32 = 2.0;
const ON: f32 = 21.0;

/// The knob's left edge, `since_flip` after the rocker last changed state
/// (`None`: at rest).
pub fn knob_left(on: bool, since_flip: Option<Duration>) -> f32 {
    let (from, to) = if on { (OFF, ON) } else { (ON, OFF) };
    let Some(d) = since_flip else { return to };
    // Whole milliseconds: exact at the test points, smooth enough at 60 fps.
    let t = (d.as_millis() as f32 / KNOB_MS as f32).min(1.0);
    from + (to - from) * t
}
```

- [ ] **Step 4: Implement `sand.rs`.**
  This is a port of `reference/sand-field.js` (quoted in the handoff "The sand field"). Use a xorshift RNG, so there is no new dependency.

```rust
//! The scope's sand field (Nostromo handoff "The sand field"): 1 000 grains
//! riding a slow dune surface, blown right by a wind that is the tunnel's
//! throughput. The colour mix *is* the up/down ratio, the speed *is* the
//! rate. Pure: the canvas program in the applet binary owns one, steps it
//! on every redraw and paints `dots()`.

pub const GRAINS: usize = 1000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tint {
    Downlink,
    Uplink,
    Spark,
}

/// What drives the field: tunnel up and service running, and the rates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drive {
    pub live: bool,
    pub up_kbps: f32,
    pub down_kbps: f32,
}

pub fn wind_target(d: Drive) -> f32 {
    if d.live { 10.0 + (d.up_kbps + d.down_kbps).min(120.0) * 0.85 } else { 2.0 }
}

/// The uplink's share of the traffic; 30 % when there is none.
pub fn up_share(d: Drive) -> f32 {
    let total = d.up_kbps + d.down_kbps;
    if d.live && total > 0.0 { d.up_kbps / total } else { 0.3 }
}

#[derive(Clone, Copy, Debug)]
struct Grain {
    x: f32,
    y: f32,
    off: f32,
    ph: f32,
    size: f32,
    b: f32,
    k: f32,
}

pub struct Sand {
    w: f32,
    h: f32,
    t: f32,
    wind: Option<f32>,
    grains: Vec<Grain>,
    rng: u64,
}

impl Sand {
    pub fn new(w: f32, h: f32, seed: u64) -> Self {
        let mut s = Sand { w, h, t: 0.0, wind: None, grains: Vec::with_capacity(GRAINS), rng: seed.max(1) };
        s.seed_grains();
        s
    }

    /// Re-seed the grains when the scope's size really changed.
    pub fn resize(&mut self, w: f32, h: f32) {
        if (w - self.w).abs() > 0.5 || (h - self.h).abs() > 0.5 {
            self.w = w;
            self.h = h;
            self.seed_grains();
        }
    }

    fn rand(&mut self) -> f32 {
        // xorshift64*
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 40) as f32 / (1u64 << 24) as f32
    }

    fn grain(&mut self, x: f32) -> Grain {
        let off = (self.rand() - 0.62) * 28.0;
        Grain {
            x,
            y: self.h * 0.64 + off,
            off,
            ph: self.rand() * std::f32::consts::TAU,
            size: if self.rand() < 0.12 { 1.5 } else { 1.0 },
            b: 0.3 + self.rand() * 0.7,
            k: self.rand(),
        }
    }

    fn seed_grains(&mut self) {
        self.grains.clear();
        for _ in 0..GRAINS {
            let x = self.rand() * self.w;
            let g = self.grain(x);
            self.grains.push(g);
        }
    }

    fn surface(&self, x: f32) -> f32 {
        let t = self.t;
        self.h * 0.64 + 11.0 * (x * 0.011 + t * 0.25).sin() + 5.0 * (x * 0.029 - t * 0.17 + 1.3).sin() + 3.0 * (x * 0.07 + t * 0.6).sin()
    }

    /// Advance `dt` seconds (capped at 50 ms).
    pub fn step(&mut self, dt: f32, d: Drive) {
        let dt = dt.clamp(0.0, 0.05);
        self.t += dt;
        let target = wind_target(d);
        let wind = match self.wind {
            None => target,
            Some(w) => w + (target - w) * (dt * 2.5).min(1.0),
        };
        self.wind = Some(wind);
        let t = self.t;
        for i in 0..self.grains.len() {
            let g = self.grains[i];
            let ys = self.surface(g.x);
            let gust = 0.5 + 0.5 * (g.x * 0.02 + t * 1.7 + g.ph).sin();
            let vx = wind * (0.5 + gust) * (0.6 + g.k * 0.8) * if g.off < 0.0 { 1.6 } else { 0.7 };
            let vy = (ys + g.off - g.y) * 2.5 + wind * 0.15 * (g.x * 0.05 + t * 2.3 + g.ph).sin();
            let mut g = Grain { x: g.x + vx * dt, y: g.y + vy * dt, ..g };
            if g.x > self.w + 2.0 {
                g = self.grain(-2.0);
            }
            self.grains[i] = g;
        }
    }

    /// Every grain as `(x, y, size, tint, alpha level 0..=10)`.
    pub fn dots(&self, d: Drive) -> impl Iterator<Item = (f32, f32, f32, Tint, u8)> + '_ {
        let share = up_share(d);
        let dim = if d.live { 1.0 } else { 0.45 };
        let t = self.t;
        self.grains.iter().map(move |g| {
            let a = g.b * (0.5 + 0.5 * (t * 2.2 + g.ph * 3.0).sin()) * dim;
            let tint = if g.k > 0.975 {
                Tint::Spark
            } else if g.k < share {
                Tint::Uplink
            } else {
                Tint::Downlink
            };
            (g.x, g.y, g.size, tint, (a * 10.0).round().clamp(0.0, 10.0) as u8)
        })
    }
}
```

  Note on the `all_up` tint test: with `up_share` = 1.0, `k < 1.0` is always true except for sparks (k > .975 wins first), so no Downlink grains. With `all_down` (share 0), `k < 0` is never true, so no Uplink grains.

- [ ] **Step 5: Run the tests.** `cargo test --lib applet::sand applet::rocker` → PASS. `cargo clippy --all-targets` → clean.
- [ ] **Step 6: Commit.**
  Message: `feat(applet): sand-field simulation and rocker knob maths`, with the trailer.

---

### Task 6: The Nostromo skin

**Files:**
- Create: `src/applet/skin.rs`
- Modify: `src/applet/mod.rs` (`pub mod skin;`)

**Interfaces:**
- Produces:
  - palette consts `BG CARD LINE LINE_2 PHOSPHOR DIM DIMMER WHITE AMBER RED HOVER OVERFLOW_OPEN LAUNCH_GLOW FOCUSED_ROW HEADER_PLATE NOTICE_BORDER QUIT_HOVER SCANLINE`;
  - `Ink` enum with `color()`;
  - `Type` struct + type consts;
  - geometry consts (below);
  - container classes `popover_class() card_class() chip_class(fill, border, text_bg) led_class(Ink, glow) cell_class(Color) plate_class() notice_class() log_class() hairline_class(Color) glow_wrap_class()`;
  - button classes `account_row_class(focused) menu_row_class(danger) primary_class(Primary) overflow_class(open) bare_class()`.

- [ ] **Step 1: Write the failing tests.**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn hex(c: Color) -> (u8, u8, u8, u8) {
        let q = |v: f32| (v * 255.0).round() as u8;
        (q(c.r), q(c.g), q(c.b), q(c.a))
    }

    #[test]
    fn the_palette_is_the_handoffs() {
        assert_eq!(hex(BG), (0x05, 0x09, 0x07, 0xff));
        assert_eq!(hex(CARD), (0x08, 0x11, 0x09, 0xff));
        assert_eq!(hex(LINE), (0x17, 0x3b, 0x22, 0xff));
        assert_eq!(hex(LINE_2), (0x24, 0x52, 0x33, 0xff));
        assert_eq!(hex(PHOSPHOR), (0x7c, 0xe3, 0x8b, 0xff));
        assert_eq!(hex(DIM), (0x4f, 0x9c, 0x5c, 0xff));
        assert_eq!(hex(DIMMER), (0x3c, 0x7a, 0x47, 0xff));
        assert_eq!(hex(WHITE), (0xe9, 0xf5, 0xe6, 0xff));
        assert_eq!(hex(AMBER), (0xff, 0xb0, 0x00, 0xff));
        assert_eq!(hex(RED), (0xff, 0x4a, 0x3d, 0xff));
        assert_eq!(hex(HOVER), (0x7c, 0xe3, 0x8b, 0x12));
        assert_eq!(hex(FOCUSED_ROW), (0xff, 0xb0, 0x00, 0x14));
        assert_eq!(hex(HEADER_PLATE), (0xff, 0xb0, 0x00, 0x1f));
        assert_eq!(hex(NOTICE_BORDER), (0xff, 0xb0, 0x00, 0x80));
        assert_eq!(hex(QUIT_HOVER), (0xff, 0x4a, 0x3d, 0x1a));
        assert_eq!(hex(OVERFLOW_OPEN), (0x7c, 0xe3, 0x8b, 0x1a));
        assert_eq!(hex(LAUNCH_GLOW), (0x7c, 0xe3, 0x8b, 0x40));
    }

    #[test]
    fn geometry_matches_the_handoff() {
        assert_eq!(POPOVER_WIDTH, 360);
        assert_eq!((POPOVER_RADIUS, CARD_RADIUS, CHIP_RADIUS), (4.0, 2.0, 1.0));
        assert_eq!((TOGGLE_W, TOGGLE_H, KNOB_PX), (38.0, 19.0, 13.0));
        assert_eq!((ACCOUNT_ROW_HEIGHT, MENU_ROW_HEIGHT, PRIMARY_HEIGHT, OVERFLOW_PX), (28.0, 28.0, 36.0, 36.0));
        assert_eq!((LED_PX, HEADER_LED_PX, INDEX_CHIP_PX, PLATE_PX, PLATE_MARK_PX), (8.0, 6.0, 17.0, 26.0, 17.0));
        assert_eq!((SCOPE_HEIGHT, GAUGE_CELLS, GAUGE_CELL_H), (104.0, 20, 7.0));
        assert_eq!(HEADER_PAD, pad(12.0, 14.0, 10.0, 14.0));
        assert_eq!(CARD_MARGIN, pad(0.0, 10.0, 10.0, 10.0));
        assert_eq!(SECTION_HEAD_PAD, pad(0.0, 14.0, 6.0, 14.0));
        assert_eq!(ROW_PAD, pad(8.0, 12.0, 8.0, 12.0));
        assert_eq!(MENU_PAD, 5.0);
    }

    /// The terminal's floor is 8 px (ping stats, by design).
    #[test]
    fn nothing_is_set_below_8_px() {
        for t in ALL_TYPES {
            assert!(t.size >= 7.5, "{t:?}");
        }
        assert_eq!(SECTION_INDEX.size, 7.5, "the one exception: Michroma section chips");
        assert_eq!(PING_STATS.size, 8.0);
    }

    #[test]
    fn ink_maps_to_the_palette() {
        assert_eq!(Ink::Phosphor.color(), PHOSPHOR);
        assert_eq!(Ink::Dimmer.color(), DIMMER);
        assert_eq!(Ink::Red.color(), RED);
    }
}
```

- [ ] **Step 2: Run them** — FAIL.
- [ ] **Step 3: Implement `skin.rs`.**

```rust
//! The Nostromo skin (handoff v6 "Design tokens"): one phosphor on black
//! glass, amber when something wants attention, red for Quit and the bad
//! end of a gauge. A fixed palette by design — the popover does not follow
//! the COSMIC accent or light/dark preference. The panel button stays
//! COSMIC (`theme.rs`).

use cosmic::iced::border::Radius;
use cosmic::iced::widget::container;
use cosmic::iced::{Background, Border, Color, Padding, Shadow, Vector};
use cosmic::widget::button;

use crate::applet::fonts::Face;

const fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color {
    Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: a as f32 / 255.0 }
}

// ---- palette ---------------------------------------------------------------

pub const BG: Color = rgba(0x05, 0x09, 0x07, 0xff);
pub const CARD: Color = rgba(0x08, 0x11, 0x09, 0xff);
pub const LINE: Color = rgba(0x17, 0x3b, 0x22, 0xff);
pub const LINE_2: Color = rgba(0x24, 0x52, 0x33, 0xff);
pub const PHOSPHOR: Color = rgba(0x7c, 0xe3, 0x8b, 0xff);
pub const DIM: Color = rgba(0x4f, 0x9c, 0x5c, 0xff);
pub const DIMMER: Color = rgba(0x3c, 0x7a, 0x47, 0xff);
pub const WHITE: Color = rgba(0xe9, 0xf5, 0xe6, 0xff);
pub const AMBER: Color = rgba(0xff, 0xb0, 0x00, 0xff);
pub const RED: Color = rgba(0xff, 0x4a, 0x3d, 0xff);

pub const HOVER: Color = rgba(0x7c, 0xe3, 0x8b, 0x12);
pub const OVERFLOW_OPEN: Color = rgba(0x7c, 0xe3, 0x8b, 0x1a);
pub const LAUNCH_GLOW: Color = rgba(0x7c, 0xe3, 0x8b, 0x40);
pub const FOCUSED_ROW: Color = rgba(0xff, 0xb0, 0x00, 0x14);
pub const HEADER_PLATE: Color = rgba(0xff, 0xb0, 0x00, 0x1f);
pub const NOTICE_BORDER: Color = rgba(0xff, 0xb0, 0x00, 0x80);
pub const QUIT_HOVER: Color = rgba(0xff, 0x4a, 0x3d, 0x1a);
/// Scanline: `#00000059`, one row in three.
pub const SCANLINE: Color = rgba(0, 0, 0, 0x59);
/// The top edge's phosphor line at 70 %.
pub const TOP_LINE: Color = rgba(0x7c, 0xe3, 0x8b, 0xb3);

/// The text colours the view model speaks in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    Phosphor,
    Dim,
    Dimmer,
    White,
    Amber,
    Red,
    Bg,
}

impl Ink {
    pub fn color(self) -> Color {
        match self {
            Ink::Phosphor => PHOSPHOR,
            Ink::Dim => DIM,
            Ink::Dimmer => DIMMER,
            Ink::White => WHITE,
            Ink::Amber => AMBER,
            Ink::Red => RED,
            Ink::Bg => BG,
        }
    }
}

// ---- geometry --------------------------------------------------------------

pub const fn pad(top: f32, right: f32, bottom: f32, left: f32) -> Padding {
    Padding { top, right, bottom, left }
}

/// libcosmic's popup width; the content fills it.
pub const POPOVER_WIDTH: u32 = 360;
pub const POPOVER_RADIUS: f32 = 4.0;
pub const CARD_RADIUS: f32 = 2.0;
pub const CHIP_RADIUS: f32 = 1.0;

pub const HEADER_PAD: Padding = pad(12.0, 14.0, 10.0, 14.0);
pub const HEADER_GAP: f32 = 10.0;
pub const PLATE_PX: f32 = 26.0;
pub const PLATE_MARK_PX: f32 = 17.0;
pub const HEADER_LED_PX: f32 = 6.0;

pub const SECTION_HEAD_PAD: Padding = pad(0.0, 14.0, 6.0, 14.0);
pub const SECTION_HEAD_GAP: f32 = 8.0;
/// Index chip `3 5 2`.
pub const SECTION_CHIP_PAD: Padding = pad(3.0, 5.0, 2.0, 5.0);
/// Every card's margin: `0 10 10`.
pub const CARD_MARGIN: Padding = pad(0.0, 10.0, 10.0, 10.0);
pub const BRACKET_PX: f32 = 8.0;

pub const ROW_PAD: Padding = pad(8.0, 12.0, 8.0, 12.0);
pub const ROW_GAP: f32 = 10.0;
pub const HAIRLINE_INSET: f32 = 12.0;
pub const LED_PX: f32 = 8.0;
pub const TOGGLE_W: f32 = 38.0;
pub const TOGGLE_H: f32 = 19.0;
pub const KNOB_PX: f32 = 13.0;

pub const ACCOUNT_LIST_PAD: f32 = 5.0;
pub const ACCOUNT_ROW_HEIGHT: f32 = 28.0;
pub const ACCOUNT_ROW_PAD: Padding = pad(0.0, 8.0, 0.0, 8.0);
pub const INDEX_CHIP_PX: f32 = 17.0;
pub const EMPTY_PAD: Padding = pad(10.0, 12.0, 11.0, 12.0);

pub const SCOPE_HEIGHT: f32 = 104.0;
pub const SCOPE_INSET: f32 = 12.0;
pub const CORNER_MARK_PX: f32 = 7.0;
pub const PING_LEFT_W: f32 = 64.0;
pub const PING_RIGHT_W: f32 = 74.0;
pub const FACTS_PAD: Padding = pad(7.0, 12.0, 8.0, 12.0);

pub const HOST_PAD: Padding = pad(8.0, 12.0, 9.0, 12.0);
pub const HOST_ROW_GAP: f32 = 6.0;
pub const GAUGE_CELLS: usize = 20;
pub const GAUGE_CELL_H: f32 = 7.0;
pub const GAUGE_GAP: f32 = 2.0;
pub const GAUGE_LABEL_W: f32 = 26.0;
pub const GAUGE_VALUE_W: f32 = 32.0;
pub const GAUGE_DETAIL_W: f32 = 54.0;

pub const NOTICE_STRIPE_H: f32 = 4.0;
pub const NOTICE_PAD: Padding = pad(7.0, 11.0, 8.0, 11.0);

pub const ACTION_GAP: f32 = 8.0;
pub const PRIMARY_HEIGHT: f32 = 36.0;
pub const PRIMARY_PAD_X: f32 = 14.0;
pub const OVERFLOW_PX: f32 = 36.0;

pub const MENU_PAD: f32 = 5.0;
pub const MENU_ROW_HEIGHT: f32 = 28.0;
pub const MENU_ROW_PAD: Padding = pad(0.0, 10.0, 0.0, 10.0);

pub const FOOTER_PAD: Padding = pad(7.0, 14.0, 8.0, 14.0);
pub const CURSOR_W: f32 = 6.0;
pub const CURSOR_H: f32 = 9.0;

// ---- type --------------------------------------------------------------------

/// A text style: size in px, face, tracking in em.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Type {
    pub size: f32,
    pub face: Face,
    pub track: f32,
}

const fn ty(size: f32, face: Face, track: f32) -> Type {
    Type { size, face, track }
}

pub const WORDMARK: Type = ty(12.0, Face::Display, 0.24);
pub const SUBLINE: Type = ty(8.5, Face::Mono, 0.14);
pub const SUBLINE_JP: Type = ty(8.5, Face::Jp, 0.14);
pub const STATE_WORD: Type = ty(10.0, Face::MonoBold, 0.14);
pub const SECTION_INDEX: Type = ty(7.5, Face::Display, 0.12);
pub const SECTION_LABEL: Type = ty(10.0, Face::MonoBold, 0.16);
pub const SECTION_META: Type = ty(8.5, Face::Mono, 0.10);
pub const COUNT: Type = ty(10.5, Face::MonoBold, 0.0);
pub const ROW_TITLE: Type = ty(11.5, Face::MonoBold, 0.06);
pub const ROW_SUB: Type = ty(8.5, Face::Mono, 0.06);
pub const ACCOUNT_NAME: Type = ty(11.5, Face::Mono, 0.04);
pub const INDEX_CHIP: Type = ty(9.5, Face::MonoBold, 0.0);
pub const FOCUSED_TAG: Type = ty(8.5, Face::MonoBold, 0.16);
pub const EMPTY_TITLE: Type = ty(10.5, Face::MonoBold, 0.08);
pub const EMPTY_BODY: Type = ty(9.5, Face::Mono, 0.04);
pub const READOUT_LABEL: Type = ty(8.5, Face::MonoBold, 0.16);
pub const READOUT: Type = ty(16.0, Face::MonoBold, 0.0);
pub const PING_VALUE: Type = ty(20.0, Face::MonoBold, 0.0);
pub const UNIT: Type = ty(8.5, Face::Mono, 0.08);
pub const SCOPE_FOOT: Type = ty(8.5, Face::Mono, 0.08);
pub const PING_STATS: Type = ty(8.0, Face::Mono, 0.0);
pub const QUALITY_WORD: Type = ty(8.5, Face::MonoBold, 0.0);
pub const FACTS: Type = ty(8.5, Face::Mono, 0.04);
pub const FACTS_KEY: Type = ty(8.5, Face::MonoBold, 0.04);
pub const GAUGE_LABEL: Type = ty(9.0, Face::MonoBold, 0.12);
pub const GAUGE_VALUE: Type = ty(11.0, Face::MonoBold, 0.0);
pub const GAUGE_DETAIL: Type = ty(8.5, Face::Mono, 0.0);
pub const NOTICE: Type = ty(9.5, Face::Mono, 0.04);
pub const PRIMARY: Type = ty(11.5, Face::MonoBold, 0.14);
pub const PRIMARY_SUB: Type = ty(8.5, Face::Mono, 0.12);
pub const LOG: Type = ty(9.5, Face::Mono, 0.04);
pub const MENU: Type = ty(10.5, Face::Mono, 0.08);
pub const FOOTER: Type = ty(8.5, Face::Mono, 0.16);
pub const OVERFLOW: Type = ty(15.0, Face::Mono, 0.0);

pub const ALL_TYPES: [Type; 34] = [
    WORDMARK, SUBLINE, SUBLINE_JP, STATE_WORD, SECTION_INDEX, SECTION_LABEL, SECTION_META, COUNT, ROW_TITLE, ROW_SUB,
    ACCOUNT_NAME, INDEX_CHIP, FOCUSED_TAG, EMPTY_TITLE, EMPTY_BODY, READOUT_LABEL, READOUT, PING_VALUE, UNIT, SCOPE_FOOT,
    PING_STATS, QUALITY_WORD, FACTS, FACTS_KEY, GAUGE_LABEL, GAUGE_VALUE, GAUGE_DETAIL, NOTICE, PRIMARY, PRIMARY_SUB, LOG,
    MENU, FOOTER, OVERFLOW,
];

// ---- container classes ---------------------------------------------------------

fn boxed(bg: Option<Color>, edge: Option<Color>, radius: f32) -> container::Style {
    container::Style {
        background: bg.map(Background::Color),
        border: Border { radius: Radius::from(radius), width: if edge.is_some() { 1.0 } else { 0.0 }, color: edge.unwrap_or(Color::TRANSPARENT) },
        ..Default::default()
    }
}

fn class(style: container::Style) -> cosmic::theme::Container<'static> {
    cosmic::theme::Container::custom(move |_| style)
}

/// The popover's glass: `BG`, 1 px `LINE` edge, radius 4, deep shadow.
pub fn popover_class() -> cosmic::theme::Container<'static> {
    class(container::Style {
        shadow: Shadow { color: rgba(0, 0, 0, 0xff), offset: Vector::new(0.0, 30.0), blur_radius: 70.0 },
        ..boxed(Some(BG), Some(LINE), POPOVER_RADIUS)
    })
}

pub fn card_class() -> cosmic::theme::Container<'static> {
    class(boxed(Some(CARD), Some(LINE), CARD_RADIUS))
}

/// An outline card (the launch log): no fill.
pub fn log_class() -> cosmic::theme::Container<'static> {
    class(boxed(None, Some(LINE), CARD_RADIUS))
}

/// A small chip: optional fill, optional 1 px border, radius 1.
pub fn chip_class(fill: Option<Color>, edge: Option<Color>) -> cosmic::theme::Container<'static> {
    class(boxed(fill, edge, CHIP_RADIUS))
}

/// A square LED; `glow` adds the handoff's blur-8 shadow at 67 %.
pub fn led_class(color: Color, glow: bool) -> cosmic::theme::Container<'static> {
    class(container::Style {
        shadow: if glow { Shadow { color: Color { a: 0.67, ..color }, offset: Vector::ZERO, blur_radius: 8.0 } } else { Shadow::default() },
        ..boxed(Some(color), None, CHIP_RADIUS)
    })
}

/// One gauge cell.
pub fn cell_class(color: Color) -> cosmic::theme::Container<'static> {
    class(boxed(Some(color), None, CHIP_RADIUS))
}

/// A solid 1 px line.
pub fn hairline_class(color: Color) -> cosmic::theme::Container<'static> {
    class(boxed(Some(color), None, 0.0))
}

/// The header plate: amber 12 % fill, 1 px amber, radius 2.
pub fn plate_class() -> cosmic::theme::Container<'static> {
    class(boxed(Some(HEADER_PLATE), Some(AMBER), CARD_RADIUS))
}

/// The Steam notice: amber 50 % border, radius 2, no fill.
pub fn notice_class() -> cosmic::theme::Container<'static> {
    class(boxed(None, Some(NOTICE_BORDER), CARD_RADIUS))
}

/// The ready launch button's glow, worn by a wrapper container
/// (cosmic buttons have no shadow).
pub fn launch_glow_class() -> cosmic::theme::Container<'static> {
    class(container::Style { shadow: Shadow { color: LAUNCH_GLOW, offset: Vector::ZERO, blur_radius: 14.0 }, ..boxed(None, None, CARD_RADIUS) })
}

// ---- button classes --------------------------------------------------------------

fn button_style(text: Color, fill: Option<Color>, edge: Option<Color>, radius: f32) -> button::Style {
    button::Style {
        background: fill.map(Background::Color),
        border_radius: Radius::from(radius),
        border_width: if edge.is_some() { 1.0 } else { 0.0 },
        border_color: edge.unwrap_or(Color::TRANSPARENT),
        text_color: Some(text),
        icon_color: Some(text),
        ..Default::default()
    }
}

fn states(rest: button::Style, hover: button::Style, disabled: button::Style) -> cosmic::theme::Button {
    let (r, h, d) = (rest, hover, disabled);
    cosmic::theme::Button::Custom {
        active: Box::new(move |_, _| r),
        disabled: Box::new(move |_| d),
        hovered: Box::new(move |_, _| h),
        pressed: Box::new(move |_, _| h),
    }
}

/// An account row: amber 8 % when focused, phosphor 7 % under the pointer.
pub fn account_row_class(focused: bool) -> cosmic::theme::Button {
    let rest = focused.then_some(FOCUSED_ROW);
    states(
        button_style(PHOSPHOR, rest, None, CHIP_RADIUS),
        button_style(PHOSPHOR, Some(rest.unwrap_or(HOVER)), None, CHIP_RADIUS),
        button_style(DIMMER, rest, None, CHIP_RADIUS),
    )
}

/// A menu row; Quit is red with a red 10 % hover.
pub fn menu_row_class(danger: bool) -> cosmic::theme::Button {
    let (ink, hover) = if danger { (RED, QUIT_HOVER) } else { (PHOSPHOR, HOVER) };
    states(button_style(ink, None, None, CHIP_RADIUS), button_style(ink, Some(hover), None, CHIP_RADIUS), button_style(DIMMER, None, None, CHIP_RADIUS))
}

/// The primary button's three looks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrimaryLook {
    /// Phosphor fill, `BG` text.
    Ready,
    /// Phosphor outline and text.
    Busy,
    /// `LINE` outline, dimmer text, no press.
    Inert,
}

pub fn primary_class(look: PrimaryLook) -> cosmic::theme::Button {
    let s = match look {
        PrimaryLook::Ready => button_style(BG, Some(PHOSPHOR), Some(PHOSPHOR), CARD_RADIUS),
        PrimaryLook::Busy => button_style(PHOSPHOR, None, Some(PHOSPHOR), CARD_RADIUS),
        PrimaryLook::Inert => button_style(DIMMER, None, Some(LINE), CARD_RADIUS),
    };
    states(s, s, s)
}

/// The `⋯` button: `LINE_2` border at rest; open = phosphor border + 10 % fill.
pub fn overflow_class(open: bool) -> cosmic::theme::Button {
    let rest = if open { button_style(PHOSPHOR, Some(OVERFLOW_OPEN), Some(PHOSPHOR), CARD_RADIUS) } else { button_style(PHOSPHOR, None, Some(LINE_2), CARD_RADIUS) };
    let hover = button_style(PHOSPHOR, Some(if open { OVERFLOW_OPEN } else { HOVER }), Some(if open { PHOSPHOR } else { LINE_2 }), CARD_RADIUS);
    states(rest, hover, rest)
}

/// No chrome at all: a hit area around a drawn control (the rockers).
pub fn bare_class() -> cosmic::theme::Button {
    let s = button_style(PHOSPHOR, None, None, 0.0);
    states(s, s, s)
}
```

- [ ] **Step 4: Run the tests** → PASS. `cargo clippy --all-targets` → clean.
- [ ] **Step 5: Commit.**
  Message: `feat(applet): the Nostromo skin — palette, geometry, type, classes`, with the trailer.

---

### Task 7: The `Console` view model

**Files:**
- Create: `src/applet/console.rs`
- Modify: `src/applet/mod.rs` (`pub mod console;`)

**Interfaces:**
- Consumes:
  - `skin::Ink`;
  - `ping::{PingWindow, PingSummary}`;
  - `host::HostReading`;
  - `rate::Rates`;
  - `format::*`;
  - `icon::HANDSHAKE_STALE_S`;
  - `menu::rows`;
  - `crate::steam::first_message`;
  - `crate::tunnel::status::Status`.
- Produces:

```rust
pub struct Inputs<'a> { pub rates: Rates, pub totals: (u64, u64), pub host: &'a HostReading, pub ping: &'a PingWindow, pub tunnel_pending: Option<bool>, pub service_pending: bool, pub available: Option<i32>, pub menu_open: bool }
pub fn console(status: Option<&Status>, i: &Inputs) -> Console
pub fn console_height(c: &Console, menu_open: bool) -> i32
pub enum RockerState { Off, On, Pending, Disabled }
pub struct ControlRow { pub title: &'static str, pub sub: String, pub sub_ink: Ink, pub led: Ink, pub glow: bool, pub rocker: RockerState, pub press: Option<Action> }
pub enum Accounts { Stopped, Empty, Rows(Vec<AccountRow>) }
pub struct AccountRow { pub index: usize, pub name: String, pub focused: bool }
pub struct Network { pub live: bool, pub up: String, pub down: String, pub tx_total: String, pub rx_total: String, pub up_kbps: f32, pub down_kbps: f32, pub scope: bool, pub ping: PingSummary, pub endpoint: String, pub peer: String, pub uptime: String, pub uptime_ink: Ink }
pub enum Level { Normal, Warn, Crit }
pub struct Gauge { pub label: &'static str, pub lit: usize, pub level: Level, pub value: String, pub detail: String }
pub enum LaunchButton { Hidden, Ready, Launching { step: u8 }, Inert(&'static str) }
pub struct Console { pub running: bool, pub state_word: &'static str, pub control: [ControlRow; 2], pub accounts_label: &'static str, pub count: Option<String>, pub accounts: Accounts, pub network: Network, pub host_meta: String, pub host: [Gauge; 3], pub notice: Option<String>, pub launch: LaunchButton, pub footer: String }
pub fn gauge_cells(pct: Option<f32>) -> (usize, Level)
```

- [ ] **Step 1: Write the failing tests.** Port the fixtures from `display.rs`'s tests.

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tunnel::status::{ClientStatus, TunnelStatus};

    fn status(connected: bool, handshake_age_s: Option<u64>, clients: usize) -> Status {
        Status {
            clients: (0..clients).map(|i| ClientStatus { name: format!("Pilot {i}"), active: i == 0 }).collect(),
            hidden: false,
            tunnel: TunnelStatus {
                installed: true,
                connected,
                iface: "yutani0".into(),
                location: "London".into(),
                address: None,
                endpoint: connected.then(|| "203.0.113.42:51820".into()),
                handshake_age_s,
                up_for_s: connected.then_some(5_880),
                failed: false,
                exit_address: None,
                rx_bytes: 693_600_000,
                tx_bytes: 105_800_000,
            },
            shortcuts: None,
            outputs: Vec::new(),
            steam: Vec::new(),
        }
    }

    fn inputs<'a>(host: &'a HostReading, ping: &'a PingWindow) -> Inputs<'a> {
        Inputs { rates: Rates { tx: 1_000.0, rx: 8_000.0 }, totals: (693_600_000, 105_800_000), host, ping, tunnel_pending: None, service_pending: false, available: None, menu_open: false }
    }

    #[test]
    fn running_and_connected_fills_every_section() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let c = console(Some(&status(true, Some(4), 2)), &inputs(&h, &p));
        assert!(c.running);
        assert_eq!(c.state_word, "RUNNING");
        assert_eq!(c.control[0].sub, "MULTIBOX · HOTKEYS · ROUTING ACTIVE");
        assert_eq!(c.control[0].rocker, RockerState::On);
        assert_eq!(c.control[0].press, Some(Action::Quit));
        assert_eq!((c.control[1].sub.as_str(), c.control[1].sub_ink), ("CONNECTED · LONDON", Ink::Dim));
        assert_eq!((c.control[1].rocker, c.control[1].press), (RockerState::On, Some(Action::Disconnect)));
        assert_eq!((c.accounts_label, c.count.as_deref()), ("ACCOUNTS", Some("02")));
        let Accounts::Rows(rows) = &c.accounts else { panic!() };
        assert_eq!(rows[0], AccountRow { index: 1, name: "PILOT 0".into(), focused: true });
        let n = &c.network;
        assert_eq!((n.up.as_str(), n.down.as_str()), ("1", "8"));
        assert_eq!((n.tx_total.as_str(), n.rx_total.as_str()), ("105.8 MB TX", "693.6 MB RX"));
        assert_eq!((n.endpoint.as_str(), n.peer.as_str()), ("203.0.113.42", "yutani0"));
        assert_eq!((n.uptime.as_str(), n.uptime_ink), ("T+ 1H 38M", Ink::Phosphor));
        assert!(n.live && n.scope);
        assert_eq!(c.launch, LaunchButton::Hidden, "Phase 1");
        assert!(c.footer.starts_with("YUTANI OS · BUILD "));
    }

    #[test]
    fn a_stopped_service_reads_stopped_everywhere() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let c = console(None, &inputs(&h, &p));
        assert_eq!((c.running, c.state_word), (false, "STOPPED"));
        assert_eq!(c.control[0].sub, "STOPPED · MULTIBOX, HOTKEYS, ROUTING OFF");
        assert_eq!((c.control[0].rocker, c.control[0].press), (RockerState::Off, Some(Action::StartDaemon)));
        assert_eq!((c.control[1].sub.as_str(), c.control[1].rocker, c.control[1].press), ("START YUTANI TO ROUTE TRAFFIC", RockerState::Disabled, None));
        assert_eq!(c.accounts, Accounts::Stopped);
        assert_eq!(c.count, None);
        assert_eq!((c.network.up.as_str(), c.network.down.as_str()), ("0", "0"));
        assert_eq!((c.network.endpoint.as_str(), c.network.uptime.as_str(), c.network.uptime_ink), ("—", "IDLE", Ink::Dimmer));
        assert_eq!(c.network.tx_total, "105.8 MB TX", "totals freeze, they do not reset");
        assert!(!c.network.live);
    }

    #[test]
    fn every_tunnel_state_has_its_copy() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let i = inputs(&h, &p);
        let sub = |s: &Status, i: &Inputs| {
            let c = console(Some(s), i);
            (c.control[1].sub.clone(), c.control[1].sub_ink, c.control[1].rocker)
        };
        let mut not_installed = status(false, None, 0);
        not_installed.tunnel.installed = false;
        assert_eq!(sub(&not_installed, &i), ("NO TUNNEL INSTALLED · PREFERENCES → TUNNEL".into(), Ink::Dimmer, RockerState::Disabled));
        assert_eq!(sub(&status(false, None, 0), &i), ("IDLE · LONDON".into(), Ink::Dimmer, RockerState::Off));
        assert_eq!(sub(&status(true, Some(200), 0), &i), ("LONDON · HANDSHAKE STALE".into(), Ink::Amber, RockerState::On));
        let mut just_up = status(true, None, 0);
        just_up.tunnel.up_for_s = Some(5);
        assert_eq!(sub(&just_up, &i), ("CONNECTING · LONDON".into(), Ink::Amber, RockerState::Pending), "up, never handshaked, just up");
        let mut never = status(true, None, 0);
        never.tunnel.up_for_s = Some(HANDSHAKE_STALE_S);
        assert_eq!(sub(&never, &i).1, Ink::Amber, "up for 3 min without a handshake is stale");
        let pending = Inputs { tunnel_pending: Some(true), ..inputs(&h, &p) };
        assert_eq!(sub(&status(false, None, 0), &pending), ("CONNECTING · LONDON".into(), Ink::Amber, RockerState::Pending));
    }

    #[test]
    fn accounts_label_count_and_empty_state() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let one = console(Some(&status(true, Some(4), 1)), &inputs(&h, &p));
        assert_eq!((one.accounts_label, one.count.clone()), ("ACCOUNT", None));
        let none = console(Some(&status(true, Some(4), 0)), &inputs(&h, &p));
        assert_eq!(none.accounts, Accounts::Empty);
    }

    #[test]
    fn gauges_light_twenty_cells_and_turn_amber_then_red() {
        assert_eq!(gauge_cells(None), (0, Level::Normal));
        assert_eq!(gauge_cells(Some(23.0)), (5, Level::Normal));
        assert_eq!(gauge_cells(Some(69.9)), (14, Level::Normal));
        assert_eq!(gauge_cells(Some(70.0)), (14, Level::Warn));
        assert_eq!(gauge_cells(Some(90.0)), (18, Level::Crit));
        assert_eq!(gauge_cells(Some(140.0)), (20, Level::Crit));
        let h = HostReading { cpu: Some(23.4), gpu: Some(40.0), ram_used: Some(10_952_166_604), ram_total: Some(33_438_498_816), cpu_temp: Some(46.1), gpu_temp: None };
        let p = PingWindow::default();
        let c = console(None, &inputs(&h, &p));
        assert_eq!((c.host[0].value.as_str(), c.host[0].detail.as_str()), ("23%", "46°C"));
        assert_eq!((c.host[1].value.as_str(), c.host[1].detail.as_str()), ("40%", "—"));
        assert_eq!((c.host[2].value.as_str(), c.host[2].detail.as_str()), ("33%", "10.2 GB"));
        assert_eq!(c.host_meta, "32 GB · 1 HZ");
    }

    #[test]
    fn the_steam_notice_is_shouted_with_the_settings_pointer() {
        use crate::steam::{Finding, Verdict};
        let (h, p) = (HostReading::default(), PingWindow::default());
        let mut s = status(true, Some(4), 0);
        s.steam = vec![Finding { account: "1".into(), verdict: Verdict::NoWrapper }];
        let c = console(Some(&s), &inputs(&h, &p));
        let n = c.notice.expect("notice");
        assert!(n.ends_with("OPEN SETTINGS → STEAM."), "{n}");
        assert_eq!(n, n.to_uppercase());
    }

    /// The scope band is the first (and only) thing to drop on a short screen.
    #[test]
    fn the_scope_drops_when_the_popover_would_not_fit() {
        let (h, p) = (HostReading::default(), PingWindow::default());
        let s = status(true, Some(4), 2);
        let tall = console(Some(&s), &Inputs { available: Some(2_000), ..inputs(&h, &p) });
        let full = console_height(&tall, false);
        assert!(tall.network.scope);
        let short = console(Some(&s), &Inputs { available: Some(full - 1), ..inputs(&h, &p) });
        assert!(!short.network.scope);
        assert_eq!(full - console_height(&short, false), SCOPE_BAND_PX);
        let unknown = console(Some(&s), &Inputs { available: None, ..inputs(&h, &p) });
        assert!(unknown.network.scope);
    }
}
```

  > Check `crate::steam::Finding`'s actual field names before relying on them: `grep -n "pub struct Finding" -A6 src/steam.rs`. Adjust the literal to match.

- [ ] **Step 2: Run them** — FAIL.
- [ ] **Step 3: Implement `console.rs`.**

```rust
//! Everything the Nostromo popover shows (handoff v6 "Screen"), as
//! finished strings and small enums. The view renders a `Console` and
//! decides nothing; the rules live here, where they are tested.

use crate::applet::Action;
use crate::applet::format;
use crate::applet::host::HostReading;
use crate::applet::icon::HANDSHAKE_STALE_S;
use crate::applet::ping::{PingSummary, PingWindow};
use crate::applet::rate::Rates;
use crate::applet::skin::Ink;
use crate::tunnel::status::Status;

pub const DASH: &str = "—";

pub struct Inputs<'a> {
    pub rates: Rates,
    /// Session totals `(rx, tx)`: the counters while connected, frozen at
    /// their last value otherwise (the interface is per session, so its
    /// counters *are* the session's).
    pub totals: (u64, u64),
    pub host: &'a HostReading,
    pub ping: &'a PingWindow,
    /// A connect (`true`) or disconnect (`false`) is still settling.
    pub tunnel_pending: Option<bool>,
    /// A start or quit of the service is still settling.
    pub service_pending: bool,
    /// The height the popover may take (`None`: unknown).
    pub available: Option<i32>,
    pub menu_open: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RockerState {
    Off,
    On,
    Pending,
    Disabled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ControlRow {
    pub title: &'static str,
    pub sub: String,
    pub sub_ink: Ink,
    pub led: Ink,
    pub glow: bool,
    pub rocker: RockerState,
    pub press: Option<Action>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountRow {
    pub index: usize,
    pub name: String,
    pub focused: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Accounts {
    Stopped,
    Empty,
    Rows(Vec<AccountRow>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Network {
    /// Tunnel up and service running: wind blows, readouts are live.
    pub live: bool,
    pub up: String,
    pub down: String,
    pub tx_total: String,
    pub rx_total: String,
    pub up_kbps: f32,
    pub down_kbps: f32,
    /// The scope band fits (short-screen rule).
    pub scope: bool,
    pub ping: PingSummary,
    pub endpoint: String,
    pub peer: String,
    pub uptime: String,
    pub uptime_ink: Ink,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Normal,
    Warn,
    Crit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Gauge {
    pub label: &'static str,
    pub lit: usize,
    pub level: Level,
    pub value: String,
    pub detail: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchButton {
    /// Phase 1: no Launch EVE yet.
    Hidden,
    Ready,
    Launching { step: u8 },
    Inert(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Console {
    pub running: bool,
    pub state_word: &'static str,
    pub control: [ControlRow; 2],
    pub accounts_label: &'static str,
    /// `02`, only when running with more than one client.
    pub count: Option<String>,
    pub accounts: Accounts,
    pub network: Network,
    pub host_meta: String,
    pub host: [Gauge; 3],
    pub notice: Option<String>,
    pub launch: LaunchButton,
    pub footer: String,
}

/// Lit cells out of 20 and the colour band: amber from 70 %, red from 90 %.
pub fn gauge_cells(pct: Option<f32>) -> (usize, Level) {
    let Some(p) = pct else { return (0, Level::Normal) };
    let p = p.clamp(0.0, 100.0);
    let lit = (p / 100.0 * crate::applet::skin::GAUGE_CELLS as f32).round() as usize;
    let level = if p >= 90.0 {
        Level::Crit
    } else if p >= 70.0 {
        Level::Warn
    } else {
        Level::Normal
    };
    (lit, level)
}

fn gauge(label: &'static str, pct: Option<f32>, detail: String) -> Gauge {
    let (lit, level) = gauge_cells(pct);
    Gauge { label, lit, level, value: pct.map_or_else(|| DASH.to_string(), |p| format!("{}%", p.round() as i64)), detail }
}

fn host_gauges(h: &HostReading) -> [Gauge; 3] {
    let temp = |t: Option<f32>| t.map_or_else(|| DASH.to_string(), |t| format!("{}°C", t.round() as i64));
    let ram_pct = match (h.ram_used, h.ram_total) {
        (Some(u), Some(t)) if t > 0 => Some(u as f32 / t as f32 * 100.0),
        _ => None,
    };
    [
        gauge("CPU", h.cpu, temp(h.cpu_temp)),
        gauge("GPU", h.gpu, temp(h.gpu_temp)),
        gauge("RAM", ram_pct, h.ram_used.map_or_else(|| DASH.to_string(), format::gb1)),
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TunnelState {
    Stopped,
    NotInstalled,
    Connecting,
    Stale,
    Connected,
    Idle,
}

fn tunnel_state(status: Option<&Status>, pending: Option<bool>) -> TunnelState {
    let Some(s) = status else { return TunnelState::Stopped };
    let t = &s.tunnel;
    if !t.installed {
        return TunnelState::NotInstalled;
    }
    if pending == Some(true) && !t.connected {
        return TunnelState::Connecting;
    }
    match (t.connected, t.handshake_age_s) {
        (false, _) => TunnelState::Idle,
        (true, None) if t.up_for_s.is_none_or(|up| up < HANDSHAKE_STALE_S) => TunnelState::Connecting,
        (true, None) => TunnelState::Stale,
        (true, Some(age)) if age >= HANDSHAKE_STALE_S => TunnelState::Stale,
        (true, Some(_)) => TunnelState::Connected,
    }
}

fn tunnel_row(state: TunnelState, location: &str) -> ControlRow {
    let loc = location.to_uppercase();
    let (sub, ink, rocker, press) = match state {
        TunnelState::Stopped => ("START YUTANI TO ROUTE TRAFFIC".to_string(), Ink::Dimmer, RockerState::Disabled, None),
        TunnelState::NotInstalled => ("NO TUNNEL INSTALLED · PREFERENCES → TUNNEL".to_string(), Ink::Dimmer, RockerState::Disabled, None),
        TunnelState::Connecting => (format!("CONNECTING · {loc}"), Ink::Amber, RockerState::Pending, Some(Action::Disconnect)),
        TunnelState::Stale => (format!("{loc} · HANDSHAKE STALE"), Ink::Amber, RockerState::On, Some(Action::Disconnect)),
        TunnelState::Connected => (format!("CONNECTED · {loc}"), Ink::Dim, RockerState::On, Some(Action::Disconnect)),
        TunnelState::Idle => (format!("IDLE · {loc}"), Ink::Dimmer, RockerState::Off, Some(Action::Connect)),
    };
    let led = match state {
        TunnelState::Connected => Ink::Phosphor,
        TunnelState::Connecting | TunnelState::Stale => Ink::Amber,
        _ => Ink::Dimmer,
    };
    ControlRow { title: "WIREGUARD TUNNEL", sub, sub_ink: ink, led, glow: state == TunnelState::Connected, rocker, press }
}

fn service_row(running: bool, pending: bool) -> ControlRow {
    let rocker = if pending { RockerState::Pending } else if running { RockerState::On } else { RockerState::Off };
    if running {
        ControlRow {
            title: "YUTANI SERVICE",
            sub: "MULTIBOX · HOTKEYS · ROUTING ACTIVE".into(),
            sub_ink: Ink::Dim,
            led: Ink::Phosphor,
            glow: true,
            rocker,
            press: Some(Action::Quit),
        }
    } else {
        ControlRow {
            title: "YUTANI SERVICE",
            sub: "STOPPED · MULTIBOX, HOTKEYS, ROUTING OFF".into(),
            sub_ink: Ink::Dimmer,
            led: Ink::Dimmer,
            glow: false,
            rocker,
            press: Some(Action::StartDaemon),
        }
    }
}

/// The scope band's share of the height: 104 + its hairline.
pub const SCOPE_BAND_PX: i32 = 105;

/// The popover's height in logical px, estimated from the handoff's
/// geometry (iced does not lay a popup out before it opens). Verified
/// against a screenshot in Task 11; adjust the section constants there.
pub fn console_height(c: &Console, menu_open: bool) -> i32 {
    const HEADER: i32 = 48;
    const SECTION_HEAD: i32 = 23;
    const CONTROL: i32 = 2 * 46 + 1 + 2;
    const PING_AND_FACTS: i32 = 50 + 1 + 27 + 2;
    const HOST: i32 = 9 + 3 * 14 + 2 * 6 + 9 + 2;
    const ACTION: i32 = 36;
    const MENU: i32 = 1 + 2 * 5 + 3 * 28 + 2;
    const FOOTER: i32 = 1 + 7 + 12 + 8;
    const GAP: i32 = 10;
    let accounts = match &c.accounts {
        Accounts::Rows(rows) => 2 * 5 + 28 * rows.len() as i32 + (rows.len() as i32 - 1).max(0) + 2,
        _ => 10 + 14 + 3 + 26 + 11 + 2,
    };
    let scope = if c.network.scope { SCOPE_BAND_PX } else { 0 };
    let notice = c.notice.as_ref().map_or(0, |n| 4 + 7 + 8 + 13 * n.chars().count().div_ceil(52).max(1) as i32 + 2 + GAP);
    let menu = if menu_open { MENU } else { 0 };
    HEADER
        + 4 * SECTION_HEAD
        + CONTROL + GAP
        + accounts + GAP
        + scope + PING_AND_FACTS + GAP
        + HOST + GAP
        + notice
        + ACTION + GAP
        + menu
        + FOOTER
}

pub fn console(status: Option<&Status>, i: &Inputs) -> Console {
    let running = status.is_some();
    let tstate = tunnel_state(status, i.tunnel_pending);
    let location = status.map_or("", |s| s.tunnel.location.as_str());
    let live = running && status.is_some_and(|s| s.tunnel.connected);
    let rate = |r: f64| if live { r } else { 0.0 };
    let (rx_total, tx_total) = i.totals;
    let (accounts, count, accounts_label) = match status {
        None => (Accounts::Stopped, None, "ACCOUNTS"),
        Some(s) if s.clients.is_empty() => (Accounts::Empty, None, "ACCOUNTS"),
        Some(s) => {
            let rows: Vec<AccountRow> = s
                .clients
                .iter()
                .enumerate()
                .map(|(n, c)| AccountRow { index: n + 1, name: c.name.to_uppercase(), focused: c.active })
                .collect();
            let n = rows.len();
            (Accounts::Rows(rows), (n > 1).then(|| format!("{n:02}")), if n == 1 { "ACCOUNT" } else { "ACCOUNTS" })
        }
    };
    let t = status.map(|s| &s.tunnel);
    let endpoint = t
        .filter(|t| t.connected)
        .and_then(|t| t.endpoint.as_ref().map(|e| e.rsplit_once(':').map_or(e.as_str(), |(h, _)| h).to_string()))
        .unwrap_or_else(|| DASH.to_string());
    let (uptime, uptime_ink) = match t.filter(|t| t.connected).and_then(|t| t.up_for_s) {
        Some(secs) if running => (format::mission_clock(secs), Ink::Phosphor),
        _ => ("IDLE".to_string(), Ink::Dimmer),
    };
    let ram_total = i.host.ram_total.map_or_else(|| DASH.to_string(), format::gb_ceil);
    let notice = status
        .and_then(|s| crate::steam::first_message(&s.steam))
        .map(|m| format!("{} OPEN SETTINGS → STEAM.", m.to_uppercase()));
    let mut c = Console {
        running,
        state_word: if running { "RUNNING" } else { "STOPPED" },
        control: [service_row(running, i.service_pending), tunnel_row(tstate, location)],
        accounts_label,
        count,
        accounts,
        network: Network {
            live,
            up: format::kb_int(rate(i.rates.tx)),
            down: format::kb_int(rate(i.rates.rx)),
            tx_total: format!("{} TX", format::mb1(tx_total)),
            rx_total: format!("{} RX", format::mb1(rx_total)),
            up_kbps: (rate(i.rates.tx) / 1_000.0) as f32,
            down_kbps: (rate(i.rates.rx) / 1_000.0) as f32,
            scope: true,
            ping: i.ping.summary(live),
            endpoint,
            peer: t.map_or_else(|| crate::tunnel::IFACE.to_string(), |t| t.iface.clone()),
            uptime,
            uptime_ink,
        },
        host_meta: format!("{ram_total} · 1 HZ"),
        host: host_gauges(i.host),
        notice,
        launch: LaunchButton::Hidden,
        footer: format!("YUTANI OS · BUILD {}", env!("YUTANI_BUILD")),
    };
    c.network.scope = i.available.is_none_or(|h| console_height(&c, i.menu_open) <= h);
    c
}
```

- [ ] **Step 4: Run the tests.** `cargo test --lib applet::console` → PASS. `cargo clippy --all-targets` → clean.
- [ ] **Step 5: Commit.**
  Message: `feat(applet): the Console view model for the Nostromo popover`, with the trailer.

---

### Task 8: Render the Console and swap the popup

**Files:**
- Create: `src/bin/yutani-applet/widgets.rs`, `src/bin/yutani-applet/console_view.rs`
- Modify: `src/bin/yutani-applet/main.rs` (`mod widgets; mod console_view;`), `src/bin/yutani-applet/app.rs`

**Interfaces:**
- Consumes: everything from Tasks 1–7.
- Produces: `console_view::popup(&Applet) -> Element<Msg>`. `app.rs` gains the fields `host: HostSampler`, `host_reading: HostReading`, `ping: PingWindow`, `totals: (u64, u64)`, `starting: Option<Instant>`, and the message `Msg::FontsLoaded(bool)`.

The view code below is complete. It is checked visually in Task 11, so its correctness gate is: it builds, clippy is clean, the existing app tests still pass, plus the two new app tests here.

- [ ] **Step 1: Write the failing app tests.** Add to `app.rs` tests:

```rust
    /// Totals are the counters while connected and freeze — not reset —
    /// when the tunnel or the service goes away.
    #[test]
    fn session_totals_freeze_when_the_tunnel_goes_down() {
        let mut applet = applet();
        let mut s = connected();
        s.tunnel.rx_bytes = 5_000_000;
        s.tunnel.tx_bytes = 1_000_000;
        let _ = applet.update(Msg::Status(Ok(s.clone())));
        assert_eq!(applet.totals, (5_000_000, 1_000_000));
        s.tunnel.connected = false;
        s.tunnel.rx_bytes = 0;
        s.tunnel.tx_bytes = 0;
        let _ = applet.update(Msg::Status(Ok(s)));
        assert_eq!(applet.totals, (5_000_000, 1_000_000));
        let _ = applet.update(Msg::Status(Err(IpcError::Offline)));
        assert_eq!(applet.totals, (5_000_000, 1_000_000));
    }

    /// Starting the service shows the pending rocker until a status reply
    /// proves the daemon is up.
    #[test]
    fn the_service_rocker_is_pending_until_the_daemon_answers() {
        let mut applet = applet();
        let _ = applet.update(Msg::Status(Err(IpcError::Offline)));
        applet.starting = Some(Instant::now() + Duration::from_secs(PENDING_S));
        assert!(applet.console().control[0].rocker == yutani::applet::console::RockerState::Pending);
        let _ = applet.update(Msg::Status(Ok(connected())));
        assert!(applet.starting.is_none());
    }
```

  Run `cargo test --bin yutani-applet` → FAIL (no `totals`, `starting`, `console`).

- [ ] **Step 2: Wire `app.rs`.**
  1. Imports: replace `use yutani::applet::display::{Popover, degrade, popover};` with `use yutani::applet::display::degrade;`. `degrade` stays until Task 10 moves it. Add:

```rust
use yutani::applet::console::{Console, Inputs, console};
use yutani::applet::host::{HostReading, HostSampler, Sources};
use yutani::applet::ping::PingWindow;
```

  2. New fields in `Applet` (after `note`):

```rust
    /// The HOST card's sampler, read at 1 Hz while the popover is open.
    pub host: HostSampler,
    pub host_reading: HostReading,
    /// Ping samples (Phase 2 feeds it; empty until then).
    pub ping: PingWindow,
    /// Session totals `(rx, tx)`: the counters while connected, frozen
    /// otherwise.
    pub totals: (u64, u64),
    /// "Start" was pressed: the service rocker shows pending until a
    /// status reply arrives or this deadline passes.
    pub starting: Option<Instant>,
```

  Initialise them in `init`:

```rust
host: HostSampler::new(Sources::discover()),
host_reading: HostReading::default(),
ping: PingWindow::default(),
totals: (0, 0),
starting: None,
```

  3. `Msg` gains:

```rust
    /// The bundled fonts finished loading (`false`: at least one failed).
    FontsLoaded(bool),
```

  In `init`, return `Task::batch([first, yutani::applet::fonts::load_all().map(|ok| cosmic::Action::App(Msg::FontsLoaded(ok)))])`. If `Task<bool>.map` to `cosmic::Action` does not type-check, use `cosmic::task::future`-free mapping: `yutani::applet::fonts::load_all().map(Msg::FontsLoaded).map(cosmic::Action::App)`. The app's `Task` alias is `cosmic::app::Task<Msg>` = `iced::Task<cosmic::Action<Msg>>`.
  4. Handle `Msg::FontsLoaded(ok)`: `if !ok { yutani::applet::fonts::set_missing(); tracing::warn!("a bundled font failed to load; using the COSMIC monospace"); } Task::none()`.
  5. In `Msg::Tick`, before polling: `if self.popup.is_some() { self.host_reading = self.host.sample(); }`.
  6. In `Msg::Status(Ok(..))`, after `let live = …`: `if live { self.totals = (status.tunnel.rx_bytes, status.tunnel.tx_bytes); }` and `self.starting = None;`.
  7. In `Msg::Press(Action::StartDaemon)` `Ok` arm: set `self.starting = Some(Instant::now() + Duration::from_secs(PENDING_S));` before returning. In `Msg::Done(Action::StartDaemon, Err(_))` (the generic `Done(action, Err)` arm): add `if action == Action::StartDaemon { self.starting = None; }`.
  8. Replace `pub fn popover(&self) -> Popover` with:

```rust
    pub fn console(&self) -> Console {
        let available = self.status.as_ref().and_then(|s| {
            let mine = &self.core.applet.output_name;
            s.outputs
                .iter()
                .find(|o| &o.name == mine)
                .map(|o| o.height)
                .or_else(|| s.outputs.iter().map(|o| o.height).filter(|h| *h > 0).min())
                .map(|h| h - PANEL_RESERVE)
        });
        let starting = self.starting.is_some_and(|until| Instant::now() < until);
        let inputs = Inputs {
            rates: self.rates,
            totals: self.totals,
            host: &self.host_reading,
            ping: &self.ping,
            tunnel_pending: self.pending.filter(|_| self.pending()).map(|(want, _)| want),
            service_pending: starting || self.quitting.is_some(),
            available,
            menu_open: self.menu_open,
        };
        console(self.status.as_ref(), &inputs)
    }
```

  9. In `open_popup_message`'s view closure, replace `view::popup(state)` with `crate::console_view::popup(state)`.
  10. The `history` field stays until Task 10, still pushed. The old `view::popup` is now unused: add `#[allow(dead_code)]` on it with the comment `// v5 popover, deleted in the cleanup task`.

- [ ] **Step 3: Write `widgets.rs`.** These are the canvas programs. Each animates itself with `request_redraw`, so `view()` never runs per frame.

```rust
//! The popover's drawn pieces (Nostromo handoff): the sand-field scope,
//! the ping sparkline, the rockers, dotted section rules, corner brackets,
//! the glass overlay and the footer's blinking cursor. Anything that moves
//! animates inside its own canvas — `RedrawRequested` steps it and asks for
//! the next frame — so the popover's `view()` is not rebuilt per frame.

use std::time::{Duration, Instant};

use cosmic::iced::mouse;
use cosmic::iced::widget::canvas::{self, Event, Frame, Geometry, Path, Stroke};
use cosmic::iced::{Color, Point, Rectangle, Renderer, Size, window};
use cosmic::widget::canvas::Action;

use yutani::applet::console::RockerState;
use yutani::applet::ping::{Dot, SPARK_H, SPARK_W};
use yutani::applet::rocker::knob_left;
use yutani::applet::sand::{Drive, Sand, Tint};
use yutani::applet::skin::{self, AMBER, BG, CORNER_MARK_PX, DIM, DIMMER, LINE, LINE_2, PHOSPHOR, WHITE};

fn alpha(c: Color, a: f32) -> Color {
    Color { a, ..c }
}

// ---- scope ---------------------------------------------------------------------

/// The sand field behind the network readouts.
pub struct Scope {
    pub drive: Drive,
}

#[derive(Default)]
pub struct ScopeState {
    sand: Option<Sand>,
    last: Option<Instant>,
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Scope {
    type State = ScopeState;

    fn update(&self, state: &mut ScopeState, event: &Event, bounds: Rectangle, _: mouse::Cursor) -> Option<Action<M>> {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return None };
        let dt = state.last.map_or(0.0, |l| now.saturating_duration_since(l).as_secs_f32());
        state.last = Some(*now);
        let sand = state.sand.get_or_insert_with(|| Sand::new(bounds.width, bounds.height, 0x5EED));
        sand.resize(bounds.width, bounds.height);
        sand.step(dt, self.drive);
        Some(Action::request_redraw())
    }

    fn draw(&self, state: &ScopeState, renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        if let Some(sand) = &state.sand {
            for (x, y, size, tint, level) in sand.dots(self.drive) {
                if level == 0 {
                    continue;
                }
                let base = match tint {
                    Tint::Downlink => PHOSPHOR,
                    Tint::Uplink => AMBER,
                    Tint::Spark => WHITE,
                };
                frame.fill_rectangle(Point::new(x, y), Size::new(size, size), alpha(base, f32::from(level) / 10.0));
            }
        }
        // 7×7 phosphor corner marks inside each corner.
        let (w, h, m) = (bounds.width, bounds.height, CORNER_MARK_PX);
        for (x, y, dx, dy) in [(0.0, 0.0, 1.0, 1.0), (w, 0.0, -1.0, 1.0), (0.0, h, 1.0, -1.0), (w, h, -1.0, -1.0)] {
            let path = Path::new(|p| {
                p.move_to(Point::new(x + dx * m, y + dy * 0.5));
                p.line_to(Point::new(x + dx * 0.5, y + dy * 0.5));
                p.line_to(Point::new(x + dx * 0.5, y + dy * m));
            });
            frame.stroke(&path, Stroke::default().with_color(PHOSPHOR).with_width(1.0));
        }
        vec![frame.into_geometry()]
    }
}

// ---- ping sparkline -------------------------------------------------------------

pub struct Sparkline {
    pub dots: Vec<Dot>,
    pub avg_y: f32,
    pub color: Color,
    pub live: bool,
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Sparkline {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let base = SPARK_H - 0.5;
        frame.stroke(&Path::line(Point::new(0.0, base), Point::new(SPARK_W, base)), Stroke::default().with_color(LINE).with_width(1.0));
        if self.live {
            // The average: white 30 %, dashed 1 4.
            let mut x = 0.0;
            while x < SPARK_W {
                frame.fill_rectangle(Point::new(x, self.avg_y - 0.5), Size::new(1.0, 1.0), alpha(WHITE, 0.3));
                x += 5.0;
            }
        }
        for d in &self.dots {
            frame.fill_rectangle(Point::new(d.x, d.y), Size::new(d.size, d.size), alpha(self.color, d.opacity));
        }
        vec![frame.into_geometry()]
    }
}

// ---- rocker ---------------------------------------------------------------------

/// The 38×19 rocker. The knob slides for 120 ms whenever `on` flips.
pub struct Rocker {
    pub state: RockerState,
}

#[derive(Default)]
pub struct RockerAnim {
    shown: Option<bool>,
    flipped: Option<Instant>,
}

impl Rocker {
    fn on(&self) -> bool {
        matches!(self.state, RockerState::On | RockerState::Pending)
    }
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Rocker {
    type State = RockerAnim;

    fn update(&self, s: &mut RockerAnim, event: &Event, _: Rectangle, _: mouse::Cursor) -> Option<Action<M>> {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return None };
        let on = self.on();
        if s.shown.is_some_and(|shown| shown != on) {
            s.flipped = Some(*now);
        }
        s.shown = Some(on);
        let moving = s.flipped.is_some_and(|f| now.saturating_duration_since(f) < Duration::from_millis(yutani::applet::rocker::KNOB_MS));
        moving.then(Action::request_redraw)
    }

    fn draw(&self, s: &RockerAnim, renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (fill, edge, knob, opacity) = match self.state {
            RockerState::Off => (None, LINE_2, DIM, 1.0),
            RockerState::On => (Some(PHOSPHOR), PHOSPHOR, BG, 1.0),
            RockerState::Pending => (Some(AMBER), AMBER, BG, 1.0),
            RockerState::Disabled => (None, LINE, DIMMER, 0.45),
        };
        let rect = Path::rounded_rectangle(Point::new(0.5, 0.5), Size::new(skin::TOGGLE_W - 1.0, skin::TOGGLE_H - 1.0), 2.0.into());
        if let Some(f) = fill {
            frame.fill(&rect, alpha(f, opacity));
        }
        frame.stroke(&rect, Stroke::default().with_color(alpha(edge, opacity)).with_width(1.0));
        let since = s.flipped.map(|f| Instant::now().saturating_duration_since(f));
        let left = knob_left(self.on(), since);
        let knob = Path::rounded_rectangle(Point::new(left, 3.0), Size::new(skin::KNOB_PX, skin::KNOB_PX), 1.0.into());
        frame.fill(&knob, alpha(knob, opacity));
        vec![frame.into_geometry()]
    }
}

// ---- dotted rule ------------------------------------------------------------------

/// The section header's dotted rule: 2 px `LINE_2`, 3 px gap, 1 px high.
pub struct DottedRule;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for DottedRule {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let y = (bounds.height / 2.0).floor();
        let mut x = 0.0;
        while x < bounds.width {
            frame.fill_rectangle(Point::new(x, y), Size::new(2.0, 1.0), LINE_2);
            x += 5.0;
        }
        vec![frame.into_geometry()]
    }
}

// ---- corner brackets ----------------------------------------------------------------

/// 8×8 phosphor brackets, top-left and bottom-right, on the two
/// interactive cards.
pub struct Brackets;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Brackets {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let (w, h, b) = (bounds.width, bounds.height, skin::BRACKET_PX);
        frame.fill_rectangle(Point::ORIGIN, Size::new(b, 1.0), PHOSPHOR);
        frame.fill_rectangle(Point::ORIGIN, Size::new(1.0, b), PHOSPHOR);
        frame.fill_rectangle(Point::new(w - b, h - 1.0), Size::new(b, 1.0), PHOSPHOR);
        frame.fill_rectangle(Point::new(w - 1.0, h - b), Size::new(1.0, b), PHOSPHOR);
        vec![frame.into_geometry()]
    }
}

// ---- glass -----------------------------------------------------------------------------

/// Scanlines (one dark row in three) and the 70 % phosphor top line, over
/// the whole popover. Cached: redrawn only when the size changes. Never
/// handles an event, so the controls under it get every click.
#[derive(Default)]
pub struct Glass {
    cache: canvas::Cache,
}

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Glass {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        vec![self.cache.draw(renderer, bounds.size(), |frame| {
            let mut y = 2.0;
            while y < bounds.height {
                frame.fill_rectangle(Point::new(0.0, y), Size::new(bounds.width, 1.0), skin::SCANLINE);
                y += 3.0;
            }
            frame.fill_rectangle(Point::ORIGIN, Size::new(bounds.width, 1.0), skin::TOP_LINE);
        })]
    }
}

// ---- cursor ----------------------------------------------------------------------------

/// The footer's 6×9 block cursor, blinking at 1 Hz, stepped.
pub struct Cursor;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Cursor {
    type State = Option<Instant>;

    fn update(&self, start: &mut Option<Instant>, event: &Event, _: Rectangle, _: mouse::Cursor) -> Option<Action<M>> {
        let Event::Window(window::Event::RedrawRequested(now)) = event else { return None };
        let s = *start.get_or_insert(*now);
        let ms = now.saturating_duration_since(s).as_millis() as u64;
        Some(Action::request_redraw_at(*now + Duration::from_millis(500 - ms % 500)))
    }

    fn draw(&self, start: &Option<Instant>, renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let on = start.is_none_or(|s| (Instant::now().saturating_duration_since(s).as_millis() / 500) % 2 == 0);
        if on {
            frame.fill_rectangle(Point::ORIGIN, bounds.size(), PHOSPHOR);
        }
        vec![frame.into_geometry()]
    }
}
```

  > The `knob` variable shadows the colour in `Rocker::draw`. Rename the colour binding to `knob_ink` when writing it; it is shown here as one name only to keep the match readable. Also check the imports compile: `cosmic::widget::canvas::Action` may live at `cosmic::iced::widget::canvas::Action` in this fork (`grep -n "pub use" ~/.cargo/git/checkouts/libcosmic-*/a401af8/iced/widget/src/canvas.rs`). `Renderer` is `cosmic::Renderer`.

- [ ] **Step 4: Write `console_view.rs`.**

```rust
//! The Nostromo popover (handoff v6 "Screen"): renders `Console` top to
//! bottom. Every size and colour comes from `yutani::applet::skin`, every
//! string from `yutani::applet::console::Console`.

use cosmic::iced::alignment::{Horizontal, Vertical};
use cosmic::iced::{Alignment, Color, Length};
use cosmic::widget::{self, Column, Row};
use cosmic::Element;

use yutani::applet::Action;
use yutani::applet::console::{AccountRow, Accounts, Console, ControlRow, Gauge, LaunchButton, Level, Network};
use yutani::applet::fonts::{self, advance_em};
use yutani::applet::menu::MenuRow;
use yutani::applet::ping::{Quality, SPARK_H, SPARK_W};
use yutani::applet::sand::Drive;
use yutani::applet::skin::{self, Ink, PrimaryLook, Type};

use crate::app::{Applet, Msg, Note};
use crate::widgets;

// ---- text ----------------------------------------------------------------------

/// Tracked text: iced has no letter-spacing, so each glyph sits in a cell
/// of its own advance plus the tracking — exact for every face, since the
/// advances come from the bundled font files.
fn t<'a>(text: impl AsRef<str>, ty: Type, color: Color) -> Element<'a, Msg> {
    let font = fonts::font(ty.face);
    if ty.track == 0.0 {
        return widget::text(text.as_ref().to_string()).size(ty.size).font(font).class(cosmic::theme::Text::Color(color)).into();
    }
    let cells = text.as_ref().chars().map(|ch| {
        let w = (advance_em(ty.face, ch) + ty.track) * ty.size;
        widget::container(widget::text(ch.to_string()).size(ty.size).font(font).class(cosmic::theme::Text::Color(color)))
            .width(Length::Fixed(w))
            .into()
    });
    Row::with_children(cells.collect::<Vec<Element<'a, Msg>>>()).align_y(Alignment::Center).into()
}

fn ink(i: Ink) -> Color {
    i.color()
}

fn fill_x<'a>() -> Element<'a, Msg> {
    widget::space().width(Length::Fill).into()
}

fn sized(w: f32, h: f32) -> widget::Space {
    widget::space().width(Length::Fixed(w)).height(Length::Fixed(h))
}

fn hairline<'a>(inset: f32) -> Element<'a, Msg> {
    widget::container(widget::container(widget::space().width(Length::Fill).height(Length::Fixed(1.0))).class(skin::hairline_class(skin::LINE)))
        .width(Length::Fill)
        .padding([0.0, inset])
        .into()
}

fn led<'a>(color: Color, glow: bool, px: f32) -> Element<'a, Msg> {
    widget::container(sized(px, px)).class(skin::led_class(color, glow)).into()
}

fn centered<'a>(content: impl Into<Element<'a, Msg>>, x: Horizontal) -> Element<'a, Msg> {
    widget::container(content).width(Length::Fill).height(Length::Fill).align_x(x).align_y(Vertical::Center).into()
}

fn margin<'a>(content: impl Into<Element<'a, Msg>>) -> Element<'a, Msg> {
    widget::container(content).width(Length::Fill).padding(skin::CARD_MARGIN).into()
}

/// A card; `brackets` adds the interactive cards' corner marks.
fn card<'a>(content: impl Into<Element<'a, Msg>>, brackets: bool) -> Element<'a, Msg> {
    let body: Element<'a, Msg> = widget::container(content).width(Length::Fill).class(skin::card_class()).into();
    let body = if brackets {
        cosmic::iced::widget::stack([body, widget::canvas(widgets::Brackets).width(Length::Fill).height(Length::Fill).into()]).into()
    } else {
        body
    };
    margin(body)
}

// ---- header ----------------------------------------------------------------------

fn header<'a>(c: &Console) -> Element<'a, Msg> {
    let mark = widget::icon(widget::icon::from_svg_bytes(yutani::assets::YUTANI_SYMBOLIC).symbolic(true))
        .class(cosmic::theme::Svg::custom(|_| cosmic::iced::widget::svg::Style { color: Some(skin::AMBER) }))
        .width(Length::Fixed(skin::PLATE_MARK_PX))
        .height(Length::Fixed(skin::PLATE_MARK_PX));
    let plate = widget::container(mark)
        .width(Length::Fixed(skin::PLATE_PX))
        .height(Length::Fixed(skin::PLATE_PX))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(skin::plate_class());
    let subline = Row::new()
        .align_y(Alignment::Center)
        .push(t("ユタニ重工", skin::SUBLINE_JP, skin::DIM))
        .push(t(" · MULTIBOX SYSTEMS", skin::SUBLINE, skin::DIM));
    let words = Column::new().spacing(2).push(t("YUTANI", skin::WORDMARK, skin::WHITE)).push(subline);
    let state = if c.running { skin::PHOSPHOR } else { skin::DIMMER };
    Row::new()
        .width(Length::Fill)
        .padding(skin::HEADER_PAD)
        .spacing(skin::HEADER_GAP)
        .align_y(Alignment::Center)
        .push(plate)
        .push(words)
        .push(fill_x())
        .push(led(state, c.running, skin::HEADER_LED_PX))
        .push(t(c.state_word, skin::STATE_WORD, state))
        .into()
}

// ---- section header ---------------------------------------------------------------

fn section<'a>(index: &'static str, label: &str, meta: Option<Element<'a, Msg>>) -> Element<'a, Msg> {
    let chip = widget::container(t(index, skin::SECTION_INDEX, skin::BG)).padding(skin::SECTION_CHIP_PAD).class(skin::chip_class(Some(skin::PHOSPHOR), None));
    let mut row = Row::new()
        .width(Length::Fill)
        .padding(skin::SECTION_HEAD_PAD)
        .spacing(skin::SECTION_HEAD_GAP)
        .align_y(Alignment::Center)
        .push(chip)
        .push(t(label.to_string(), skin::SECTION_LABEL, skin::PHOSPHOR))
        .push(widget::canvas(widgets::DottedRule).width(Length::Fill).height(Length::Fixed(3.0)));
    if let Some(meta) = meta {
        row = row.push(meta);
    }
    row.into()
}

// ---- 01 control ---------------------------------------------------------------------

fn control_row<'a>(r: &ControlRow) -> Element<'a, Msg> {
    let text = Column::new().spacing(3).push(t(r.title, skin::ROW_TITLE, skin::PHOSPHOR)).push(t(r.sub.clone(), skin::ROW_SUB, ink(r.sub_ink)));
    let rocker = widget::button::custom(widget::canvas(widgets::Rocker { state: r.rocker }).width(Length::Fixed(skin::TOGGLE_W)).height(Length::Fixed(skin::TOGGLE_H)))
        .padding(0)
        .class(skin::bare_class())
        .on_press_maybe(r.press.map(Msg::Press));
    Row::new()
        .width(Length::Fill)
        .padding(skin::ROW_PAD)
        .spacing(skin::ROW_GAP)
        .align_y(Alignment::Center)
        .push(led(ink(r.led), r.glow, skin::LED_PX))
        .push(widget::container(text).width(Length::Fill).clip(true))
        .push(rocker)
        .into()
}

fn control<'a>(c: &Console) -> Element<'a, Msg> {
    card(Column::new().width(Length::Fill).push(control_row(&c.control[0])).push(hairline(skin::HAIRLINE_INSET)).push(control_row(&c.control[1])), true)
}

// ---- 02 accounts ----------------------------------------------------------------------

fn account_row<'a>(r: &AccountRow) -> Element<'a, Msg> {
    let (chip_fill, chip_edge, chip_ink) = if r.focused { (Some(skin::AMBER), None, skin::BG) } else { (None, Some(skin::LINE_2), skin::DIM) };
    let chip = widget::container(t(r.index.to_string(), skin::INDEX_CHIP, chip_ink))
        .width(Length::Fixed(skin::INDEX_CHIP_PX))
        .height(Length::Fixed(skin::INDEX_CHIP_PX))
        .align_x(Horizontal::Center)
        .align_y(Vertical::Center)
        .class(skin::chip_class(chip_fill, chip_edge));
    let name_ink = if r.focused { skin::WHITE } else { skin::PHOSPHOR };
    let mut row = Row::new()
        .width(Length::Fill)
        .spacing(skin::ROW_GAP)
        .align_y(Alignment::Center)
        .push(chip)
        .push(widget::container(t(r.name.clone(), skin::ACCOUNT_NAME, name_ink)).width(Length::Fill).clip(true));
    if r.focused {
        row = row.push(t("◄ FOCUSED", skin::FOCUSED_TAG, skin::AMBER));
    }
    widget::button::custom(centered(widget::container(row).padding(skin::ACCOUNT_ROW_PAD), Horizontal::Left))
        .width(Length::Fill)
        .height(Length::Fixed(skin::ACCOUNT_ROW_HEIGHT))
        .padding(0)
        .class(skin::account_row_class(r.focused))
        .on_press(Msg::Press(Action::Focus(r.index)))
        .into()
}

fn accounts<'a>(c: &Console) -> Element<'a, Msg> {
    let empty = |title: &'static str, title_ink: Color, body: &'static str, body_ink: Color| -> Element<'a, Msg> {
        Column::new()
            .width(Length::Fill)
            .spacing(3)
            .padding(skin::EMPTY_PAD)
            .push(t(title, skin::EMPTY_TITLE, title_ink))
            .push(widget::text(body).size(skin::EMPTY_BODY.size).font(fonts::font(skin::EMPTY_BODY.face)).class(cosmic::theme::Text::Color(body_ink)))
            .into()
    };
    let body: Element<'a, Msg> = match &c.accounts {
        Accounts::Stopped => empty("SERVICE STOPPED", skin::DIM, "THUMBNAILS, HOTKEYS AND TUNNEL ROUTING ARE INACTIVE.", skin::DIMMER),
        Accounts::Empty => empty("NO EVE CLIENTS RUNNING", skin::PHOSPHOR, "LAUNCH EVE BELOW — IT APPEARS HERE WITH THE NEXT FREE HOTKEY.", skin::DIM),
        Accounts::Rows(rows) => Column::with_children(rows.iter().map(account_row).collect::<Vec<_>>()).width(Length::Fill).spacing(1).padding(skin::ACCOUNT_LIST_PAD).into(),
    };
    card(body, true)
}

// ---- 03 network -------------------------------------------------------------------------

fn readout<'a>(arrow_label: &'static str, value: String, label_ink: Color, label_first: bool) -> Element<'a, Msg> {
    let label = t(arrow_label, skin::READOUT_LABEL, label_ink);
    let value = t(value, skin::READOUT, skin::WHITE);
    let unit = t("KB/S", skin::UNIT, skin::DIM);
    let row = Row::new().spacing(6).align_y(Alignment::Center);
    if label_first { row.push(label).push(value).push(unit) } else { row.push(value).push(unit).push(label) }.into()
}

fn scope<'a>(n: &Network) -> Element<'a, Msg> {
    let drive = Drive { live: n.live, up_kbps: n.up_kbps, down_kbps: n.down_kbps };
    let top = Row::new()
        .width(Length::Fill)
        .push(readout("▲ UPLINK", n.up.clone(), skin::AMBER, true))
        .push(fill_x())
        .push(readout("DOWNLINK ▼", n.down.clone(), skin::PHOSPHOR, false));
    let bottom = Row::new()
        .width(Length::Fill)
        .push(t(n.tx_total.clone(), skin::SCOPE_FOOT, skin::DIM))
        .push(fill_x())
        .push(t("SESSION", skin::SCOPE_FOOT, skin::DIM))
        .push(fill_x())
        .push(t(n.rx_total.clone(), skin::SCOPE_FOOT, skin::DIM));
    let overlay = Column::new().width(Length::Fill).height(Length::Fill).padding([10.0, skin::SCOPE_INSET]).push(top).push(widget::space().height(Length::Fill)).push(bottom);
    cosmic::iced::widget::stack([
        widget::canvas(widgets::Scope { drive }).width(Length::Fill).height(Length::Fixed(skin::SCOPE_HEIGHT)).into(),
        overlay.into(),
    ])
    .width(Length::Fill)
    .height(Length::Fixed(skin::SCOPE_HEIGHT))
    .into()
}

fn quality_ink(q: Quality) -> Color {
    match q {
        Quality::Nominal => skin::PHOSPHOR,
        Quality::Degraded => skin::AMBER,
        Quality::Poor => skin::RED,
        Quality::Idle => skin::DIMMER,
    }
}

fn ping_row<'a>(n: &Network) -> Element<'a, Msg> {
    let p = &n.ping;
    let q = quality_ink(p.quality);
    let left = Column::new()
        .width(Length::Fixed(skin::PING_LEFT_W))
        .spacing(2)
        .push(t("PING · TQ", skin::READOUT_LABEL, skin::PHOSPHOR))
        .push(Row::new().spacing(4).align_y(Alignment::End).push(t(p.value.clone(), skin::PING_VALUE, skin::WHITE)).push(t("MS", skin::UNIT, skin::DIM)));
    let spark = widgets::Sparkline { dots: p.dots.clone(), avg_y: p.avg_y, color: q, live: n.live };
    let middle = Column::new()
        .width(Length::Fill)
        .spacing(4)
        .push(widget::canvas(spark).width(Length::Fixed(SPARK_W)).height(Length::Fixed(SPARK_H)))
        .push(t(p.stats.clone(), skin::PING_STATS, skin::DIM));
    let right = Column::new()
        .width(Length::Fixed(skin::PING_RIGHT_W))
        .spacing(4)
        .align_x(Alignment::End)
        .push(Row::new().spacing(6).align_y(Alignment::Center).push(t(p.quality.word(), skin::QUALITY_WORD, q)).push(led(q, false, 6.0)))
        .push(t(p.jitter_loss.clone(), skin::PING_STATS, skin::DIM));
    Row::new().width(Length::Fill).padding(skin::ROW_PAD).spacing(skin::ROW_GAP).align_y(Alignment::Center).push(left).push(middle).push(right).into()
}

fn facts<'a>(n: &Network) -> Element<'a, Msg> {
    Row::new()
        .width(Length::Fill)
        .padding(skin::FACTS_PAD)
        .spacing(6)
        .align_y(Alignment::Center)
        .push(t("ENDPOINT", skin::FACTS_KEY, skin::PHOSPHOR))
        .push(t(n.endpoint.clone(), skin::FACTS, skin::WHITE))
        .push(t("·", skin::FACTS, skin::LINE_2))
        .push(t("PEER", skin::FACTS_KEY, skin::PHOSPHOR))
        .push(t(n.peer.clone(), skin::FACTS, skin::WHITE))
        .push(fill_x())
        .push(t(n.uptime.clone(), skin::FACTS, ink(n.uptime_ink)))
        .into()
}

fn network<'a>(n: &Network) -> Element<'a, Msg> {
    let mut col = Column::new().width(Length::Fill);
    if n.scope {
        col = col.push(scope(n)).push(hairline(0.0));
    }
    card(col.push(ping_row(n)).push(hairline(skin::HAIRLINE_INSET)).push(facts(n)), false)
}

// ---- 04 host --------------------------------------------------------------------------------

fn gauge_row<'a>(g: &Gauge) -> Element<'a, Msg> {
    let lit_color = match g.level {
        Level::Normal => skin::PHOSPHOR,
        Level::Warn => skin::AMBER,
        Level::Crit => skin::RED,
    };
    let cells = (0..skin::GAUGE_CELLS).map(|i| {
        let color = if i >= g.lit {
            skin::LINE
        } else if g.level == Level::Normal && i + 1 == g.lit {
            skin::WHITE
        } else {
            lit_color
        };
        widget::container(widget::space().width(Length::Fill).height(Length::Fixed(skin::GAUGE_CELL_H))).width(Length::Fill).class(skin::cell_class(color)).into()
    });
    let value_ink = if g.level == Level::Normal { skin::WHITE } else { lit_color };
    Row::new()
        .width(Length::Fill)
        .spacing(8)
        .align_y(Alignment::Center)
        .push(widget::container(t(g.label, skin::GAUGE_LABEL, skin::PHOSPHOR)).width(Length::Fixed(skin::GAUGE_LABEL_W)))
        .push(Row::with_children(cells.collect::<Vec<Element<'a, Msg>>>()).width(Length::Fill).spacing(skin::GAUGE_GAP))
        .push(widget::container(t(g.value.clone(), skin::GAUGE_VALUE, value_ink)).width(Length::Fixed(skin::GAUGE_VALUE_W)).align_x(Horizontal::Right))
        .push(widget::container(t(g.detail.clone(), skin::GAUGE_DETAIL, skin::DIM)).width(Length::Fixed(skin::GAUGE_DETAIL_W)).align_x(Horizontal::Right))
        .into()
}

fn host<'a>(c: &Console) -> Element<'a, Msg> {
    card(Column::with_children(c.host.iter().map(gauge_row).collect::<Vec<_>>()).width(Length::Fill).spacing(skin::HOST_ROW_GAP).padding(skin::HOST_PAD), false)
}

// ---- notice, action row, menu, footer ---------------------------------------------------------

fn notice<'a>(text: &str) -> Element<'a, Msg> {
    let stripe = widget::canvas(widgets::Hazard).width(Length::Fill).height(Length::Fixed(skin::NOTICE_STRIPE_H));
    let body = widget::container(widget::text(text.to_string()).size(skin::NOTICE.size).font(fonts::font(skin::NOTICE.face)).class(cosmic::theme::Text::Color(skin::AMBER)))
        .width(Length::Fill)
        .padding(skin::NOTICE_PAD);
    margin(widget::container(Column::new().width(Length::Fill).push(stripe).push(body)).width(Length::Fill).class(skin::notice_class()))
}

fn action_row<'a>(c: &Console, menu_open: bool) -> Element<'a, Msg> {
    let overflow = widget::button::custom(centered(t("⋯", skin::OVERFLOW, skin::PHOSPHOR), Horizontal::Center))
        .width(Length::Fixed(skin::OVERFLOW_PX))
        .height(Length::Fixed(skin::OVERFLOW_PX))
        .padding(0)
        .class(skin::overflow_class(menu_open))
        .on_press(Msg::ToggleMenu);
    let mut row = Row::new().width(Length::Fill).spacing(skin::ACTION_GAP);
    row = match c.launch {
        // Phase 1: the ⋯ alone, at the right.
        LaunchButton::Hidden => row.push(fill_x()),
        LaunchButton::Ready | LaunchButton::Launching { .. } | LaunchButton::Inert(_) => row.push(primary(c.launch)),
    };
    margin(row.push(overflow))
}

/// The primary button. Phase 1 never produces anything but `Hidden`; the
/// other looks are Phase 3's, drawn here so the skin is complete.
fn primary<'a>(l: LaunchButton) -> Element<'a, Msg> {
    let (label, sub, look) = match l {
        LaunchButton::Ready => ("▶ LAUNCH EVE".to_string(), None, PrimaryLook::Ready),
        LaunchButton::Launching { step } => ("LAUNCHING…".to_string(), Some(format!("STEP {step} / 4")), PrimaryLook::Busy),
        LaunchButton::Inert(why) => (why.to_string(), None, PrimaryLook::Inert),
        LaunchButton::Hidden => unreachable!("not drawn"),
    };
    let ink = match look {
        PrimaryLook::Ready => skin::BG,
        PrimaryLook::Busy => skin::PHOSPHOR,
        PrimaryLook::Inert => skin::DIMMER,
    };
    let mut content = Row::new().width(Length::Fill).align_y(Alignment::Center).push(t(label, skin::PRIMARY, ink)).push(fill_x());
    if let Some(sub) = sub {
        content = content.push(t(sub, skin::PRIMARY_SUB, ink));
    }
    let button = widget::button::custom(centered(widget::container(content).padding([0.0, skin::PRIMARY_PAD_X]), Horizontal::Left))
        .width(Length::Fill)
        .height(Length::Fixed(skin::PRIMARY_HEIGHT))
        .padding(0)
        .class(skin::primary_class(look))
        .on_press_maybe((look == PrimaryLook::Ready).then_some(Msg::Launch));
    if look == PrimaryLook::Ready {
        widget::container(button).width(Length::Fill).class(skin::launch_glow_class()).into()
    } else {
        button.into()
    }
}

fn menu_row<'a>(r: &MenuRow) -> Element<'a, Msg> {
    let color = if r.danger { skin::RED } else if r.action.is_some() { skin::PHOSPHOR } else { skin::DIMMER };
    widget::button::custom(centered(widget::container(t(r.label.to_uppercase(), skin::MENU, color)).padding(skin::MENU_ROW_PAD), Horizontal::Left))
        .width(Length::Fill)
        .height(Length::Fixed(skin::MENU_ROW_HEIGHT))
        .padding(0)
        .class(skin::menu_row_class(r.danger))
        .on_press_maybe(r.action.map(Msg::Press))
        .into()
}

fn menu<'a>(running: bool) -> Element<'a, Msg> {
    let rows = yutani::applet::menu::rows(running);
    Column::new()
        .width(Length::Fill)
        .push(hairline(0.0))
        .push(Column::with_children(rows.iter().map(menu_row).collect::<Vec<_>>()).width(Length::Fill).spacing(1).padding(skin::MENU_PAD))
        .into()
}

fn note_line<'a>(note: &Note) -> Element<'a, Msg> {
    let color = if note.progress { skin::DIM } else { skin::RED };
    widget::container(widget::text(note.text.to_uppercase()).size(skin::NOTICE.size).font(fonts::font(skin::NOTICE.face)).class(cosmic::theme::Text::Color(color)))
        .width(Length::Fill)
        .padding(skin::pad(0.0, 14.0, 10.0, 14.0))
        .into()
}

fn footer<'a>(c: &Console) -> Element<'a, Msg> {
    let ready = Row::new()
        .spacing(4)
        .align_y(Alignment::Center)
        .push(t("READY FOR INQUIRY", skin::FOOTER, skin::DIM))
        .push(widget::canvas(widgets::Cursor).width(Length::Fixed(skin::CURSOR_W)).height(Length::Fixed(skin::CURSOR_H)));
    Column::new()
        .width(Length::Fill)
        .push(hairline(0.0))
        .push(Row::new().width(Length::Fill).padding(skin::FOOTER_PAD).align_y(Alignment::Center).push(t(c.footer.clone(), skin::FOOTER, skin::DIM)).push(fill_x()).push(ready))
        .into()
}

// ---- the popover ----------------------------------------------------------------------------------

pub fn popup(state: &Applet) -> Element<'_, Msg> {
    let c = state.console();
    let count = c.count.clone().map(|n| t(n, skin::COUNT, skin::WHITE));
    let mut col = Column::new()
        .width(Length::Fill)
        .push(header(&c))
        .push(section("01", "CONTROL", None))
        .push(control(&c))
        .push(section("02", c.accounts_label, count))
        .push(accounts(&c))
        .push(section("03", "NETWORK", Some(t("EVE TRAFFIC ONLY", skin::SECTION_META, skin::DIM))))
        .push(network(&c.network))
        .push(section("04", "HOST", Some(t(c.host_meta.clone(), skin::SECTION_META, skin::DIM))))
        .push(host(&c));
    if let Some(n) = &c.notice {
        col = col.push(notice(n));
    }
    col = col.push(action_row(&c, state.menu_open));
    if let Some(note) = state.visible_note() {
        col = col.push(note_line(note));
    }
    if state.menu_open {
        col = col.push(menu(c.running));
    }
    col = col.push(footer(&c));
    let glass = widget::canvas(&state.glass).width(Length::Fill).height(Length::Fill);
    widget::container(cosmic::iced::widget::stack([col.into(), glass.into()]))
        .width(Length::Fill)
        .class(skin::popover_class())
        .into()
}
```

  Supporting additions this view needs (make them in this step):
  - `widgets.rs`: add a `Hazard` program, the notice's 135° stripe (amber 6 px, `BG` 6 px):

```rust
/// The Steam notice's hazard stripe: 135° amber/black, 6 px each.
pub struct Hazard;

impl<M> canvas::Program<M, cosmic::Theme, Renderer> for Hazard {
    type State = ();

    fn draw(&self, _: &(), renderer: &Renderer, _: &cosmic::Theme, bounds: Rectangle, _: mouse::Cursor) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(Point::ORIGIN, bounds.size(), BG);
        let h = bounds.height;
        let mut x = -h;
        while x < bounds.width + h {
            let band = Path::new(|p| {
                p.move_to(Point::new(x, 0.0));
                p.line_to(Point::new(x + 6.0, 0.0));
                p.line_to(Point::new(x + 6.0 - h, h));
                p.line_to(Point::new(x - h, h));
                p.close();
            });
            frame.fill(&band, AMBER);
            x += 12.0;
        }
        vec![frame.into_geometry()]
    }
}
```

  - `app.rs`: a field `pub glass: crate::widgets::Glass` (init `Default::default()`), and `Msg::Launch` (Phase 3 handles it; Phase 1: `Msg::Launch => Task::none()`).

- [ ] **Step 5: Build and fix compile errors.** `cargo build --bin yutani-applet 2>&1 | head -80`. Expected fix-ups, all mechanical:
  - import paths for `canvas::Action`;
  - `Renderer` vs `cosmic::Renderer`;
  - `padding([f32; 2])` vs `[u16; 2]` (convert where the fork wants `u16`);
  - `widget::space()` sizing helpers;
  - `Text::Color` vs `.class(color)`.

  Keep the structure; fix types only.
- [ ] **Step 6: Run the tests.** `cargo test --bin yutani-applet && cargo test --lib` → PASS. Then `cargo clippy --all-targets` → clean.
- [ ] **Step 7: Smoke-run the applet binary outside the panel.** It cannot open a popup there; this only checks the fonts load and nothing panics at init: `timeout 3 target/debug/yutani-applet; echo $?`. Expected: exit 124 or a libcosmic "not running in a panel" error, and no panic.
- [ ] **Step 8: Commit.**
  Message: `feat(applet): render the Nostromo popover`, with the trailer.

---

### Task 9: Square tray badge

**Files:**
- Modify: `src/applet/theme.rs` (`badge_px`, `badge_class`, tests), `src/bin/yutani-applet/view.rs` (`panel_button` only)

**Interfaces:**
- Produces:
  - `theme::BADGE_SQUARE_PX: f32 = 7.0`;
  - `theme::badge_class(badge: Badge) -> Container` (the diameter parameter is dropped).
- Colours:
  - dark panel: Connected `skin::PHOSPHOR` with glow, Attention `skin::AMBER`, Sync a hollow `skin::AMBER` outline;
  - light panel: the theme's success/warning colours.
- Ring: 1.5 px in the panel background. Radius 1.

- [ ] **Step 1: Write the failing test.** Replace `the_badge_is_a_third_of_the_icon_within_bounds`:

```rust
    /// Nostromo: a 7×7 square badge whatever the panel size.
    #[test]
    fn the_badge_is_a_seven_pixel_square() {
        assert_eq!(BADGE_SQUARE_PX, 7.0);
        assert_eq!(BADGE_RING_PX, 1.5);
        assert_eq!(DIM_OPACITY, 0.40);
        assert_eq!(badge_fill(true, Badge::Connected), (crate::applet::skin::PHOSPHOR, true));
        assert_eq!(badge_fill(true, Badge::Attention), (crate::applet::skin::AMBER, false));
    }
```

  `badge_fill(is_dark: bool, badge: Badge) -> (Color, bool /*glow*/)`, defined for dark panels; light panels resolve through the theme inside `badge_class`.
- [ ] **Step 2: Run it** — FAIL.
- [ ] **Step 3: Implement.** Replace `badge_px` and `badge_class` in `theme.rs`:

```rust
/// The status badge (Nostromo): a 7×7 square, radius 1.
pub const BADGE_SQUARE_PX: f32 = 7.0;

/// The badge's fill on a dark panel, and whether it glows.
pub fn badge_fill(_is_dark: bool, badge: super::icon::Badge) -> (Color, bool) {
    use super::icon::Badge;
    match badge {
        Badge::Connected => (crate::applet::skin::PHOSPHOR, true),
        Badge::Attention | Badge::Sync => (crate::applet::skin::AMBER, false),
    }
}

/// The status badge: phosphor (glowing) or amber on a dark panel, the
/// theme's success/warning on a light one (phosphor on a light grey is
/// under 2:1); a 1.5 px ring in the panel's own colour; Sync is a hollow
/// outline.
pub fn badge_class(badge: super::icon::Badge) -> cosmic::theme::Container<'static> {
    use super::icon::Badge;
    cosmic::theme::Container::custom(move |theme| {
        let c = theme.cosmic();
        let panel = panel_bg(c);
        let (fill, glow) = if c.is_dark {
            badge_fill(true, badge)
        } else {
            (if badge == Badge::Connected { success(c) } else { warning(c) }, false)
        };
        let (bg, ring) = if badge == Badge::Sync { (panel, fill) } else { (fill, panel) };
        container::Style {
            background: Some(Background::Color(bg)),
            border: Border { radius: Radius::from(1.0), width: BADGE_RING_PX, color: ring },
            shadow: if glow { cosmic::iced::Shadow { color: Color { a: 0.67, ..fill }, offset: cosmic::iced::Vector::ZERO, blur_radius: 6.0 } } else { Default::default() },
            ..Default::default()
        }
    })
}
```

  In `view.rs` `panel_button`: `let d = theme::BADGE_SQUARE_PX + 2.0 * theme::BADGE_RING_PX;` (the ring is drawn inside the border box, so the box grows by the ring to keep a 7 px fill), and `.class(theme::badge_class(badge))`.
- [ ] **Step 4: Run the tests** → PASS. `cargo clippy --all-targets` → clean.
- [ ] **Step 5: Commit.**
  Message: `feat(applet): square Nostromo tray badge`, with the trailer.

---

### Task 10: Remove the v5 popover

**Files:**
- Delete: `src/applet/display.rs`, `src/applet/history.rs`
- Modify:
  - `src/applet/mod.rs`;
  - `src/applet/theme.rs` (keep only what `panel_button` and the badge use: `DIM_OPACITY`, `BADGE_RING_PX`, `BADGE_SQUARE_PX`, `mark_class`, `MARK_ON_DARK`, `badge_fill`, `badge_class`, the role fns those need: `ink`, `success`, `warning`, `panel_bg`);
  - `src/applet/format.rs` (drop `bytes`, `rate`, `rate_parts`, `uptime`, `handshake`, `accounts_label` if nothing uses them — check with `grep -rn "format::" src`);
  - `src/bin/yutani-applet/view.rs` → keep only `panel_button`, moving it into `app.rs`'s neighbour `panel.rs` if that reads better;
  - `src/bin/yutani-applet/app.rs` (drop `history`, `degrade` import → move `degrade` into `console.rs` with its doc comment and test);
  - `src/applet/icon.rs` (`IconState::opacity` uses `theme::DIM_OPACITY`, which is kept).

- [ ] **Step 1: Move `degrade`.** Move `display::degrade` and its doc comment verbatim to `console.rs`, and add this test there:

```rust
    #[test]
    fn degrade_takes_the_link_down_but_keeps_the_counters() {
        let mut s = status(true, Some(4), 0);
        degrade(&mut s);
        assert!(!s.tunnel.connected && s.tunnel.handshake_age_s.is_none() && s.tunnel.up_for_s.is_none());
        assert_eq!(s.tunnel.rx_bytes, 693_600_000);
    }
```

- [ ] **Step 2: Delete and trim.** Delete `display.rs` and `history.rs`, remove their `pub mod` lines, and remove the `history` field and its pushes from `app.rs`. Trim `theme.rs` and `format.rs` as listed above, deleting their now-unused tests (`sizes_match_the_handoff`, `the_violet_is_…`, the old format tests). Remove `#[allow(dead_code)]` items in `view.rs` (everything but `panel_button`).
- [ ] **Step 3: Verify nothing references the removed items.**
  `grep -rn "display::\|history::\|VIOLET\|rate_parts\|hotkey_hint\|ThumbsButton" src` → no matches.
- [ ] **Step 4: Run the full check.** `cargo test --lib && cargo test --bin yutani-applet && cargo clippy --all-targets` → all PASS and clean, and `cargo build --release` succeeds.
- [ ] **Step 5: Commit.**
  Message: `refactor(applet): remove the v5 popover`, with the trailer.

---

### Task 11: Install and visual QA (controller, not a subagent)

- [ ] **Step 1: Build and install the package.**
  `cd packaging && makepkg -si`. Use `pkexec pacman -U` on the built package if `-si` cannot prompt.
- [ ] **Step 2: Restart the applet and the daemon.**
  `pkill -x cosmic-panel`. Then `systemctl --user restart yutani`; the daemon is unchanged in Phase 1, so this is only for parity.
- [ ] **Step 3: Screenshot the states and compare each with `previews/01–05`.**
  Capture with `cosmic-screenshot --interactive=false --notify=false -s <scratch dir>` after opening the popover. States:
  - running + connected;
  - tunnel idle;
  - menu open;
  - service stopped (rocker off).

  For each, check:
  - spacing, tracking and the fonts (B612 Mono, Michroma, the kana);
  - the sand field moves, and the knob slides;
  - scanlines don't eat clicks: account rows and rockers still work;
  - the cursor blinks.
- [ ] **Step 4: Fix the height estimate.** Measure the real popover height in the screenshot against `console_height`, and correct the section constants in `console.rs` if they are off by more than 10 px. Commit any fixes with their own messages.
- [ ] **Step 5: Hand back to Daniel.**
  Show Daniel the screenshots. Phase 2 (ping) gets its own plan once he has looked.
