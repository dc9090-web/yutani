# Yutani — Thumbnail opacity slider and "Centre vertically"

**Status:** requested by Daniel 2026-09-13 ("add a transparency slider to
the Thumbnails section … under Display" and "a button in settings to center
the thumbnail vertically"); design approved the same day. §1 built
2026-09-14 (ca7ac2c), §2 built 2026-09-14 as an *Arrange* section on the
redesigned Layouts pane (the button, its blocker as the row's help line,
the note at the header's end).

## 1. Thumbnail opacity

### 1.1 Config

A new field in `config.ron`:

```ron
thumb_opacity: 100,   // whole-number percent, 20..=100
```

- Type `u8`, default `100`, so an existing install looks exactly as before.
- `Config::validate` replaces a value outside `20..=100` with the default and
  logs why, like every other checked field.
- Live-reloaded through the existing config watcher: a hand edit of the file
  takes effect on the next redraw, no restart.
- The floor of 20 % is deliberate: a thumbnail can never be made invisible
  and lost on screen.

### 1.2 Settings window

Display page, **Thumbnails** section, a slider between "Hover zoom" and
"Show character names":

- Label `Thumbnail opacity: 80%`; range `20..=100`, integer steps.
- Same write-once behaviour as the width and zoom sliders: `Msg::Opacity(u8)`
  is live-only (`is_live_only`), applies to the in-memory config on every
  tick of the slider, and the file is written on `on_release` → `Msg::Commit`.
- `apply_config_field` clamps to `20..=100`; a value equal to the current one
  reports no change.

### 1.3 Rendering

The effective opacity of one thumbnail is

```
effective = if hovered { 100 } else { config.thumb_opacity }   // percent
```

as a pure function `thumbnail::opacity(config, hovered) -> f32` (0.2..=1.0).
"Hovered" is the same state that drives the hover zoom: the pointer is over
that thumbnail. A hovered thumbnail is fully opaque so it can be read; it
fades back when the pointer leaves. The active client's thumbnail is treated
like any other.

Applied in `thumbnail::view`:

- The captured image is a libcosmic `Subsurface`; `Subsurface::alpha(f32)`
  sets it, and libcosmic drives the compositor's `wp_alpha_modifier_v1` from
  it (COSMIC advertises the protocol). The compositor multiplies the alpha at
  composition time: no shader change, no re-render, no per-frame cost, and a
  hover change is instant rather than waiting for the next captured frame.
- Everything iced draws on the parent layer surface follows the same factor
  by multiplying its colour alpha: the frame border, the character-name
  label (background and text), the pin badge, and the two placeholder boxes
  ("waiting for frame…", "capture unavailable"). So the whole thumbnail fades
  as one object.
- If a compositor lacks the alpha-modifier protocol the frame would fade and
  the image would not. Yutani targets COSMIC, which has it; this is noted,
  not handled.

### 1.4 Not affected

Position, size, hover zoom, drag, snap, click-to-focus, dock arrangement and
the applet are untouched.

### 1.5 Tests

- `Config`: default is 100; 0, 19, 101 and 255 validate back to 100; 20 and
  100 survive.
- `apply_config_field`: `Opacity(80)` → 80 and `Ok(true)`; `Opacity(80)`
  again → `Ok(false)`; `Opacity(5)` → 20; `Opacity(200)` → 100.
- `is_live_only(Opacity(_))` is true.
- `thumbnail::opacity`: not hovered → `thumb_opacity / 100`; hovered → 1.0
  regardless.
- A colour helper `with_opacity(color, factor)` multiplies only the alpha.

## 2. Centre vertically

### 2.1 Where

Layouts page, a new **Arrange** section above "Saved layouts", holding one
button, **Centre vertically**. It is disabled, with a caption saying why,
when:

- the mode is Dock — "The dock is already centred along its edge."
- no floating thumbnail is shown — "No thumbnails are showing."

### 2.2 What it does

For each output separately, the floating thumbnails currently shown on it
(those with a surface) form one group. The group's top is the smallest `y`,
its bottom the largest `y + height` (the current surface size, zoom
excluded). Every thumbnail on that output is shifted by the same `dy` so the
group is centred between the output's top and bottom, at the output's
logical height. `x` is untouched, so the arrangement keeps its shape and
nothing overlaps.

The arithmetic is one pure function in `model::layout`:

```rust
/// The vertical shift that centres a group spanning `top..bottom` on a
/// screen `height` tall. An odd leftover pixel goes to the top. A group
/// taller than the screen is pinned to the top (shift so `top` is 0),
/// never pushed off the bottom.
pub fn centre_shift(top: i32, bottom: i32, height: i32) -> i32
```

The daemon applies it in a `centre_vertically()` method: group by
`output_name_of`, compute `dy`, then for each thumbnail set
`client.position.1 += dy`, `set_margin` the surface (unless it is a drag
canvas, like `reposition_to_layout`), and `persist_position` it. A
thumbnail whose character name is not known yet moves on screen but is not
saved, exactly as a drag of it is not.

### 2.3 Feedback

The note line says `centred 3 thumbnails on DP-1; centred 2 thumbnails on
DP-2` (one clause per output, in output order) or `nothing to centre`. A
group already centred (`dy == 0`) still counts as centred.

### 2.4 Message flow

Settings window `Msg::CentreVertically` → daemon `App::centre_vertically()`
→ note. No IPC command and no keyboard shortcut (not asked for).

### 2.5 Tests

- `centre_shift`: already centred → 0; group at top of a 1440 screen, 200
  tall → 620; odd leftover → top gets the extra pixel; group taller than the
  screen → `-top`; empty/inverted span (`bottom <= top`) → 0.
- Grouping helper: thumbnails on two outputs get independent shifts; a
  thumbnail without a surface is ignored.
- The Layouts page shows the button, disabled in Dock mode and when no
  thumbnail is shown (view-level assertion on the blocker caption).

## 3. Out of scope

- Horizontal centring, or centring per thumbnail (rejected in the design
  conversation: a column would collapse).
- Per-thumbnail or per-character opacity.
- A shader fallback for compositors without `wp_alpha_modifier_v1`.
- Any change to the applet or the CLI.
