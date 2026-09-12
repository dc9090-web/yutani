# GPU-masked Thumbnails Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rounded thumbnail corners that actually work, by drawing each captured frame on the GPU into a thumbnail-sized dmabuf whose alpha carries a rounded-rect mask; remove the `opacity` option; put the subsurface below the parent so border/label/pin draw over the image.

**Architecture:** A GLES2 context (EGL on the gbm render node we already open, `glow` bindings) lives on the backend thread. On each `ready` frame the capture dmabuf is imported as an EGLImage texture (cached per capture buffer) and rendered with a full-screen triangle + rounded-rect SDF fragment shader into the back buffer of a per-client pool of two thumbnail-sized ABGR8888 gbm BOs, which are shipped to the UI as today's `SubsurfaceBuffer`. Any GL failure falls back to shipping the raw capture buffer. The UI tells the backend the physical thumbnail size per client and the mask radius.

**Tech Stack:** Rust 2024, libcosmic (pinned rev `a401af8`), `khronos-egl` 6 (`dynamic`), `glow` 0.16, `gbm` 0.18, cosmic-comp 1.8.

**Spec:** `docs/superpowers/specs/2026-09-12-yutani-gpu-thumbnails-design.md`

## Global Constraints

- libcosmic pinned to rev `a401af8b1c54a8abd393b8c5b7c8809402f83850`; do not bump.
- New direct deps only: `khronos-egl = { version = "6", features = ["dynamic"] }` and `glow = "0.16"` (both already in `Cargo.lock` via wgpu-hal — `cargo build` must not add new lock entries beyond those two lines).
- Output buffer format is **ABGR8888** (`u32::from(wl_shm::Format::Abgr8888)` — same fourcc the capture buffers use), not ARGB as the spec first said; the spec is corrected in Task 5.
- Modifier choice for output BOs: `gbm.create_buffer_object_with_modifiers2` over the compositor's dmabuf-feedback modifiers for ABGR8888 (gbm/Mesa picks one the GPU can render to); if the feedback has no explicit modifiers, a plain `create_buffer_object` with `RENDERING | LINEAR`. No `eglQueryDmaBufModifiersEXT` (the spec's "intersection" is what gbm already does).
- Everything GL runs on the backend thread only. Types stored in `ScreencopySession` must stay `Send` (the session is behind `Arc<Capture>` shared with thread-pool tasks): hold raw EGL pointers as `usize` and free them through the trash queue described in Task 3, never via `Drop` calling into EGL/GL.
- Every commit: `cargo build -q` warning-free for new code and `cargo test -q` green.
- Commit trailer on every commit:
  ```
  Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH
  ```

---

## File Structure

| File | Responsibility |
|---|---|
| `src/backend/gl.rs` (new) | EGL/GLES context, shader program, dmabuf import, thumbnail targets, render, self-test; pure helpers (`buffer_coords`, `uv_matrix`, `mask_radius`, `modifiers_for`, `dmabuf_image_attribs`) with tests. |
| `src/backend/mod.rs` | `mod gl;` `GlState` on `AppData`, lazy init, `Cmd::SetThumbSize` / `Cmd::SetCornerRadius`, `thumb_sizes`, `corner_radius_px`. |
| `src/backend/capture.rs` | `ThumbPool` per session; `ready()` renders through GL when possible, else ships raw. |
| `src/backend/buffer.rs` | `Buffer` gains `source: Option<gl::SourceTexture>` (lazy EGL import cache). |
| `src/model/config.rs` | Remove `opacity`. |
| `src/ui/thumbnail.rs` | Subsurface `z(-1)`, no alpha, `Transform::Normal`. |
| `src/ui/mod.rs` | Remove compositor corner-radius request machinery; `Output.scale`; send `SetThumbSize`/`SetCornerRadius`. |
| `src/doctor.rs` | `gl:` line. |
| `docs/superpowers/specs/2026-09-11-yutani-design.md` | §5/§6/§9 amendments. |
| `docs/superpowers/specs/2026-09-12-yutani-gpu-thumbnails-design.md` | ABGR/modifier corrections. |

---

### Task 1: Remove `opacity` and the compositor corner-radius request; subsurface below the parent

**Files:**
- Modify: `src/model/config.rs`
- Modify: `src/ui/thumbnail.rs:44-75`
- Modify: `src/ui/mod.rs` (`Client.rounded`, `Msg::Redrawn`, `round_surface`, `awaiting_radius`, `apply_config`, `subscription`, `Output`)

**Interfaces:**
- Produces: `Config` without `opacity`; `ui::Output { handle, name, logical_size, scale: i32 }`.

- [ ] **Step 1: Update the config tests (they fail to compile until the field is gone)**

In `src/model/config.rs` tests:
- `defaults_match_spec`: delete the line `assert_eq!(c.opacity, 1.0);`.
- `validate_replaces_bad_values_with_defaults`: delete `opacity: 7.0,` and `assert_eq!(c.opacity, d.opacity);`.
- `validate_keeps_good_values`: change the constructor to `Config { thumb_width: 480, fps: 60, zoom_factor: 2.0, border_px: 0, ..Config::default() }`.
- Add, next to `plan3_defaults_and_validation`:

```rust
    #[test]
    fn stale_opacity_field_is_ignored() {
        // Removed in the GPU-thumbnails spec; old config files still parse.
        let c: Config = ron::from_str("(opacity: 0.5, thumb_width: 300)").unwrap();
        assert_eq!(c.thumb_width, 300);
    }
```

- [ ] **Step 2: Remove the field**

In `src/model/config.rs`: delete `pub opacity: f32,` (line 38), `opacity: 1.0,` in `Default` (line 67), and the `check!(opacity, ...)` line in `validate` (line 142). Also delete the `/// Thumbnail opacity ...` doc comment if one sits above the field.

- [ ] **Step 3: Thumbnail widget: below the parent, opaque, untransformed**

In `src/ui/thumbnail.rs` replace the `Some(img) =>` arm (lines 62-72) with:

```rust
            Some(img) => Subsurface::new(img.buffer.clone())
                .width(Length::Fill)
                .height(Length::Fill)
                .content_fit(ContentFit::Contain)
                // Below the parent (z < 0) so the iced border, name label and
                // pin glyph paint over the image. The backend ships frames
                // already upright and corner-masked; when it can't (no GL)
                // the raw frame carries its own transform.
                .z(-1)
                .transform(img.transform)
                .into(),
```

- [ ] **Step 4: Remove the corner-radius request machinery in `src/ui/mod.rs`**

- Delete imports on lines 11-12 (`corner_radius`, `CornerRadius`).
- Delete `pub rounded: bool,` and its doc comment from `Client` (lines 82-84); delete `rounded: false,` in the `Client` initialiser in `on_backend` (line 624); delete `c.rounded = false;` in `forget_surface` (line 348).
- Delete the `Redrawn(SurfaceId)` variant and its doc comment from `Msg` (lines 112-114) and the `Msg::Redrawn(id) => self.round_surface(id),` arm in `update` (line 795).
- Delete `awaiting_radius` and `round_surface` (lines 298-323) and the comment `// Corner rounding is requested later, ... see round_surface.` above `create` in `create_surface` (lines 293-294) — keep `create` as the return value.
- In `apply_config` delete the block
  ```rust
        if new.corner_radius != self.config.corner_radius {
            // Re-request on every surface at its next frame (see `round_surface`).
            self.clients.values_mut().for_each(|c| c.rounded = false);
        }
  ```
  (Task 5 adds the replacement that tells the backend.)
- In `subscription` delete the whole `if self.awaiting_radius() { ... }` block (lines 814-823).
- Fix the doc comment on `apply_config`: `/// Apply a freshly re-read (and validated) config. Border colours and names apply on the next redraw automatically because ...`.

- [ ] **Step 5: `Output.scale`**

In `src/ui/mod.rs` add `pub scale: i32,` to `Output` (after `logical_size`). In `on_output`, in the `Created(Some(info)) | InfoUpdate(info)` arm:

```rust
                let scale = info.scale_factor.max(1);
                if let Some(existing) = self.outputs.iter_mut().find(|o| o.handle == output) {
                    existing.name = name;
                    existing.logical_size = logical_size;
                    existing.scale = scale;
                } else {
                    tracing::info!(%name, ?logical_size, scale, "output");
                    self.outputs.push(Output { handle: output, name, logical_size, scale });
                }
```

- [ ] **Step 6: Build and test**

Run: `cargo build -q 2>&1 | grep -E '^(warning|error)' ; cargo test -q 2>&1 | tail -3`
Expected: no errors; the only pre-existing warning is `save_to` dead code; `test result: ok. 50 passed`.

- [ ] **Step 7: Commit**

```bash
git add -A src/model/config.rs src/ui/thumbnail.rs src/ui/mod.rs
git commit -m "refactor: drop opacity and the compositor corner-radius hint; subsurface below the parent

cosmic-comp never clips layer surfaces to cosmic_corner_radius_layer_v1
(should_clip is false for Stage::LayerSurface), so the request did
nothing for the image. z(-1) makes the iced border, name label and pin
glyph visible over the image again.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 2: `backend/gl.rs` pure helpers (TDD)

**Files:**
- Create: `src/backend/gl.rs`
- Modify: `src/backend/mod.rs` (add `pub mod gl;` after `mod gbm_devices;`), `Cargo.toml`

**Interfaces (produced, used by Tasks 3-4):**
- `pub fn buffer_coords(transform: wl_output::Transform, x: f32, y: f32) -> (f32, f32)`
- `pub fn uv_matrix(transform: wl_output::Transform) -> [f32; 9]` (column-major mat3)
- `pub fn mask_radius(radius_px: u32, size: (u32, u32)) -> u32`
- `pub fn modifiers_for(table: &[(u32, u64)], tranches: &[&[u16]], format: u32) -> Vec<u64>`
- `pub fn dmabuf_image_attribs(dma: &Dmabuf) -> Vec<khronos_egl::Attrib>`
- consts `DRM_FORMAT_MOD_INVALID`, `DRM_FORMAT_MOD_LINEAR`, `ABGR8888`

- [ ] **Step 1: Dependencies**

In `Cargo.toml` `[dependencies]`, after `gbm = "0.18.0"`:

```toml
khronos-egl = { version = "6", features = ["dynamic"] }
glow = "0.16"
```

Run `cargo build -q 2>&1 | tail -2` — must succeed; `git diff --stat Cargo.lock` must show only the `yutani` package's dependency list changing (no new `[[package]]` entries).

- [ ] **Step 2: Write the failing tests**

Create `src/backend/gl.rs`:

```rust
//! GPU post-processing of captured frames: each capture dmabuf is drawn into
//! a thumbnail-sized dmabuf with rounded corners baked into its alpha.
//! cosmic-comp never clips layer surfaces to a corner radius (the
//! cosmic_corner_radius_layer_v1 hint only shapes blur), so this is the only
//! way to get rounded thumbnails. See the 2026-09-12 spec.

use cosmic::cctk::wayland_client::protocol::wl_output::Transform;
use cosmic::iced::platform_specific::shell::subsurface_widget::Dmabuf;
use khronos_egl as egl;
use std::os::fd::AsRawFd;

pub const DRM_FORMAT_MOD_INVALID: u64 = 0x00ff_ffff_ffff_ffff;
pub const DRM_FORMAT_MOD_LINEAR: u64 = 0;
/// DRM fourcc 'AB24': little-endian R, G, B, A bytes — GL RGBA order.
pub const ABGR8888: u32 = 0x3432_4241;

// EGL_EXT_image_dma_buf_import(_modifiers)
const EGL_LINUX_DMA_BUF_EXT: egl::Enum = 0x3270;
const EGL_LINUX_DRM_FOURCC_EXT: egl::Attrib = 0x3271;
/// Per plane: (FD, OFFSET, PITCH, MODIFIER_LO, MODIFIER_HI).
const PLANE_ATTRIBS: [[egl::Attrib; 5]; 4] = [
    [0x3272, 0x3273, 0x3274, 0x3443, 0x3444],
    [0x3275, 0x3276, 0x3277, 0x3445, 0x3446],
    [0x3278, 0x3279, 0x327A, 0x3447, 0x3448],
    [0x3440, 0x3441, 0x3442, 0x3449, 0x344A],
];

/// Normalised *output* coordinates (x right, y down, 0..1) → normalised
/// *buffer* coordinates for a buffer carrying `transform`
/// (`wl_surface.set_buffer_transform` semantics: the content was rendered
/// already transformed, the compositor applies the inverse to display it,
/// and 90 means a quarter turn counter-clockwise going buffer → output).
pub fn buffer_coords(transform: Transform, x: f32, y: f32) -> (f32, f32) {
    match transform {
        Transform::Normal => (x, y),
        Transform::Flipped => (1.0 - x, y),
        Transform::_180 => (1.0 - x, 1.0 - y),
        Transform::Flipped180 => (x, 1.0 - y),
        Transform::_90 => (1.0 - y, x),
        Transform::_270 => (y, 1.0 - x),
        Transform::Flipped90 => (y, x),
        Transform::Flipped270 => (1.0 - y, 1.0 - x),
        _ => (x, y),
    }
}

/// True when the displayed width is the buffer's height.
pub fn swaps_axes(transform: Transform) -> bool {
    matches!(transform, Transform::_90 | Transform::_270 | Transform::Flipped90 | Transform::Flipped270)
}

/// Column-major 3×3 matrix with `uv = M * (x, y, 1)` equal to
/// `buffer_coords(transform, x, y)`.
pub fn uv_matrix(transform: Transform) -> [f32; 9] {
    let (cx, cy) = buffer_coords(transform, 0.0, 0.0);
    let (xx, xy) = buffer_coords(transform, 1.0, 0.0);
    let (yx, yy) = buffer_coords(transform, 0.0, 1.0);
    [xx - cx, xy - cy, 0.0, yx - cx, yy - cy, 0.0, cx, cy, 1.0]
}

/// Mask radius actually used for a target: never more than half the short
/// side, so opposite corners can't overlap.
pub fn mask_radius(radius_px: u32, (w, h): (u32, u32)) -> u32 {
    radius_px.min(w.min(h) / 2)
}

/// Explicit modifiers the compositor accepts for `format`, in tranche
/// preference order, deduplicated, without `DRM_FORMAT_MOD_INVALID`.
/// `table` is the dmabuf-feedback format table as `(fourcc, modifier)`;
/// `tranches` are each tranche's indices into it.
pub fn modifiers_for(table: &[(u32, u64)], tranches: &[&[u16]], format: u32) -> Vec<u64> {
    let mut out: Vec<u64> = Vec::new();
    for tranche in tranches {
        for &i in *tranche {
            let Some(&(f, m)) = table.get(i as usize) else { continue };
            if f == format && m != DRM_FORMAT_MOD_INVALID && !out.contains(&m) {
                out.push(m);
            }
        }
    }
    out
}

/// Attribute list for `eglCreateImage(EGL_LINUX_DMA_BUF_EXT)` describing
/// `dma`. Modifier attributes are only emitted for explicit modifiers.
pub fn dmabuf_image_attribs(dma: &Dmabuf) -> Vec<egl::Attrib> {
    let mut a: Vec<egl::Attrib> = vec![
        egl::WIDTH as egl::Attrib,
        dma.width as egl::Attrib,
        egl::HEIGHT as egl::Attrib,
        dma.height as egl::Attrib,
        EGL_LINUX_DRM_FOURCC_EXT,
        dma.format as egl::Attrib,
    ];
    for plane in dma.planes.iter().take(4) {
        let [fd, offset, pitch, lo, hi] = PLANE_ATTRIBS[plane.plane_idx as usize];
        a.extend([fd, plane.fd.as_raw_fd() as egl::Attrib, offset, plane.offset as egl::Attrib, pitch, plane.stride as egl::Attrib]);
        if dma.modifier != DRM_FORMAT_MOD_INVALID {
            a.extend([lo, (dma.modifier & 0xffff_ffff) as egl::Attrib, hi, (dma.modifier >> 32) as egl::Attrib]);
        }
    }
    a.push(egl::ATTRIB_NONE);
    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmic::iced::platform_specific::shell::subsurface_widget::Plane;
    use std::os::fd::OwnedFd;

    fn corners(t: Transform) -> [(f32, f32); 4] {
        [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)].map(|(x, y)| buffer_coords(t, x, y))
    }

    #[test]
    fn buffer_coords_axis_aligned_transforms() {
        assert_eq!(corners(Transform::Normal), [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]);
        assert_eq!(corners(Transform::Flipped), [(1.0, 0.0), (0.0, 0.0), (0.0, 1.0), (1.0, 1.0)]);
        assert_eq!(corners(Transform::_180), [(1.0, 1.0), (0.0, 1.0), (0.0, 0.0), (1.0, 0.0)]);
        assert_eq!(corners(Transform::Flipped180), [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)]);
    }

    #[test]
    fn buffer_coords_quarter_turns_are_bijections_and_inverses() {
        // Output top-left comes from the buffer's top-right for a 90° CCW turn.
        assert_eq!(buffer_coords(Transform::_90, 0.0, 0.0), (1.0, 0.0));
        assert_eq!(buffer_coords(Transform::_270, 0.0, 0.0), (0.0, 1.0));
        for t in [Transform::_90, Transform::_270, Transform::Flipped90, Transform::Flipped270] {
            let mut c = corners(t).to_vec();
            c.sort_by(|a, b| a.partial_cmp(b).unwrap());
            assert_eq!(c, vec![(0.0, 0.0), (0.0, 1.0), (1.0, 0.0), (1.0, 1.0)], "{t:?} must permute the corners");
            assert!(swaps_axes(t));
        }
        // 90 then 270 is the identity.
        let (x, y) = buffer_coords(Transform::_90, 0.25, 0.75);
        assert_eq!(buffer_coords(Transform::_270, x, y), (0.25, 0.75));
        assert!(!swaps_axes(Transform::Flipped180));
    }

    #[test]
    fn uv_matrix_matches_buffer_coords() {
        for t in [Transform::Normal, Transform::_90, Transform::Flipped270] {
            let m = uv_matrix(t);
            let (x, y) = (0.3f32, 0.8f32);
            let u = m[0] * x + m[3] * y + m[6];
            let v = m[1] * x + m[4] * y + m[7];
            let (bu, bv) = buffer_coords(t, x, y);
            assert!((u - bu).abs() < 1e-6 && (v - bv).abs() < 1e-6, "{t:?}");
        }
    }

    #[test]
    fn mask_radius_clamps_to_half_short_side() {
        assert_eq!(mask_radius(8, (484, 274)), 8);
        assert_eq!(mask_radius(200, (484, 274)), 137);
        assert_eq!(mask_radius(0, (10, 10)), 0);
    }

    #[test]
    fn modifiers_for_filters_dedups_and_keeps_tranche_order() {
        let table = [(ABGR8888, 7), (0x1234, 7), (ABGR8888, DRM_FORMAT_MOD_INVALID), (ABGR8888, 0), (ABGR8888, 7)];
        let t0: &[u16] = &[0, 1, 2];
        let t1: &[u16] = &[3, 4, 99];
        assert_eq!(modifiers_for(&table, &[t0, t1], ABGR8888), vec![7, 0]);
        assert!(modifiers_for(&table, &[t0], 0x9999).is_empty());
    }

    fn fake_dma(modifier: u64, planes: usize) -> Dmabuf {
        let planes = (0..planes)
            .map(|i| {
                let fd: OwnedFd = std::fs::File::open("/dev/null").unwrap().into();
                Plane { fd, plane_idx: i as u32, offset: 16 * i as u32, stride: 1920 }
            })
            .collect();
        Dmabuf { width: 480, height: 270, planes, format: ABGR8888, modifier }
    }

    #[test]
    fn dmabuf_attribs_linear_single_plane() {
        let dma = fake_dma(DRM_FORMAT_MOD_INVALID, 1);
        let fd = dma.planes[0].fd.as_raw_fd() as egl::Attrib;
        assert_eq!(
            dmabuf_image_attribs(&dma),
            vec![0x3057, 480, 0x3056, 270, 0x3271, ABGR8888 as egl::Attrib, 0x3272, fd, 0x3273, 0, 0x3274, 1920, egl::ATTRIB_NONE]
        );
    }

    #[test]
    fn dmabuf_attribs_explicit_modifier_two_planes() {
        let dma = fake_dma(0x0100_0000_0000_0002, 2);
        let a = dmabuf_image_attribs(&dma);
        // plane 1 block starts after the 6-entry header + plane 0's 10 entries
        assert_eq!(&a[16..26], &[0x3275, dma.planes[1].fd.as_raw_fd() as egl::Attrib, 0x3276, 16, 0x3277, 1920, 0x3445, 2, 0x3446, 0x0100_0000]);
        assert_eq!(*a.last().unwrap(), egl::ATTRIB_NONE);
    }
}
```

Add `pub mod gl;` to `src/backend/mod.rs` after `mod gbm_devices;`.

- [ ] **Step 3: Run the tests**

Run: `cargo test -q gl:: 2>&1 | tail -3`
Expected: `test result: ok. 7 passed` (the implementation is in the same file; the point of this task is the pinned-down behaviour). If a `Transform` variant name differs (`_90` vs `N90`), fix to whatever `wl_output::Transform` in this wayland-client exports — check with `grep -n 'Transform' ~/.cargo/registry/src/*/wayland-client-0.31*/src/protocol.rs | head`.

- [ ] **Step 4: Commit**

```bash
git add Cargo.toml Cargo.lock src/backend/gl.rs src/backend/mod.rs
git commit -m "feat(gl): pure helpers for the GPU thumbnail pass (uv mapping, mask radius, modifiers, dmabuf attribs)

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 3: EGL/GLES context, targets, render, self-test; `yutani doctor` `gl:` line

**Files:**
- Modify: `src/backend/gl.rs` (append)
- Modify: `src/doctor.rs`

**Interfaces (produced):**
```rust
pub struct Gl { .. }                       // !Send; lives in AppData only
impl Gl {
    pub fn new(gbm: &gbm::Device<std::fs::File>) -> anyhow::Result<Gl>;
    pub fn import_source(&self, backing: &Arc<BufferSource>) -> anyhow::Result<SourceTexture>;
    pub fn create_target(&self, gbm: &gbm::Device<std::fs::File>, modifiers: &[u64], size: (u32, u32)) -> anyhow::Result<Target>;
    pub fn render(&self, src: &SourceTexture, dst: &Target, transform: Transform, radius_px: u32) -> anyhow::Result<()>;
    pub fn self_test(&self, gbm: &gbm::Device<std::fs::File>) -> anyhow::Result<()>;
}
pub struct SourceTexture { .. }            // Send; drop → trash queue
pub struct Target { pub backing: Arc<BufferSource>, pub size: (u32, u32), .. } // Send; drop → trash queue
pub fn open_render_node() -> anyhow::Result<gbm::Device<std::fs::File>>;   // first /dev/dri/renderD*
```

- [ ] **Step 1: Append the context, resources and render path to `src/backend/gl.rs`**

Add these imports at the top of the file (merge with the existing `use` block):

```rust
use anyhow::{Context as _, anyhow, ensure};
use cosmic::iced::platform_specific::shell::subsurface_widget::{BufferSource, Plane};
use gbm::AsRaw;
use glow::HasContext;
use std::ffi::c_void;
use std::fs::File;
use std::num::NonZeroU32;
use std::sync::{Arc, Mutex};
```

Then append after `dmabuf_image_attribs`:

```rust
const EGL_PLATFORM_GBM_KHR: egl::Enum = 0x31D7;

const VERT: &str = r#"
attribute vec2 a_pos;
uniform mat3 u_uv;
uniform vec2 u_size;
varying vec2 v_uv;
varying vec2 v_px;
void main() {
    // Clip space → output space in *Wayland* orientation (y down): GL row 0
    // is memory row 0, which the compositor treats as the top row.
    vec2 o = vec2(a_pos.x * 0.5 + 0.5, 0.5 - a_pos.y * 0.5);
    v_uv = (u_uv * vec3(o, 1.0)).xy;
    v_px = o * u_size;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

const FRAG: &str = r#"
precision mediump float;
uniform sampler2D u_tex;
uniform vec2 u_size;
uniform float u_radius;
varying vec2 v_uv;
varying vec2 v_px;
void main() {
    vec2 half = u_size * 0.5;
    vec2 d = abs(v_px - half) - (half - vec2(u_radius));
    // Rounded-box signed distance: negative inside, 0 on the edge.
    float dist = length(max(d, 0.0)) + min(max(d.x, d.y), 0.0) - u_radius;
    float a = clamp(0.5 - dist, 0.0, 1.0);
    vec3 c = texture2D(u_tex, v_uv).rgb;
    gl_FragColor = vec4(c, 1.0) * a; // premultiplied; source treated as opaque
}
"#;

type ImageTargetFn = unsafe extern "system" fn(u32, *const c_void);

/// GL/EGL objects owned by `SourceTexture`/`Target`s that have been dropped
/// (possibly from a `Send` context with no GL access). `Gl` frees them on
/// its own thread at the next `render`.
#[derive(Default)]
struct Trash {
    images: Vec<usize>,
    textures: Vec<u32>,
    renderbuffers: Vec<u32>,
    framebuffers: Vec<u32>,
}

type TrashQueue = Arc<Mutex<Trash>>;

/// A capture buffer imported for sampling.
pub struct SourceTexture {
    texture: u32,
    /// `EGLImage` pointer, 0 for a plain uploaded texture (self-test).
    image: usize,
    trash: TrashQueue,
}

impl Drop for SourceTexture {
    fn drop(&mut self) {
        let mut t = self.trash.lock().unwrap();
        t.textures.push(self.texture);
        if self.image != 0 {
            t.images.push(self.image);
        }
    }
}

/// A thumbnail-sized dmabuf we render into and ship to the UI.
pub struct Target {
    pub backing: Arc<BufferSource>,
    pub size: (u32, u32),
    framebuffer: u32,
    renderbuffer: u32,
    image: usize,
    trash: TrashQueue,
}

impl Drop for Target {
    fn drop(&mut self) {
        let mut t = self.trash.lock().unwrap();
        t.framebuffers.push(self.framebuffer);
        t.renderbuffers.push(self.renderbuffer);
        t.images.push(self.image);
    }
}

pub struct Gl {
    egl: egl::DynamicInstance<egl::EGL1_5>,
    display: egl::Display,
    _context: egl::Context,
    gl: glow::Context,
    image_target_texture: ImageTargetFn,
    image_target_renderbuffer: ImageTargetFn,
    program: glow::Program,
    vbo: glow::Buffer,
    a_pos: u32,
    u_uv: glow::UniformLocation,
    u_size: glow::UniformLocation,
    u_radius: glow::UniformLocation,
    u_tex: glow::UniformLocation,
    trash: TrashQueue,
}

fn tex(id: u32) -> glow::Texture {
    glow::NativeTexture(NonZeroU32::new(id).expect("GL names are non-zero"))
}
fn fbo(id: u32) -> glow::Framebuffer {
    glow::NativeFramebuffer(NonZeroU32::new(id).expect("GL names are non-zero"))
}
fn rbo(id: u32) -> glow::Renderbuffer {
    glow::NativeRenderbuffer(NonZeroU32::new(id).expect("GL names are non-zero"))
}

impl Gl {
    /// EGL display on `gbm`'s render node, GLES2 context (surfaceless, or a
    /// 1×1 pbuffer if the driver lacks EGL_KHR_surfaceless_context), and
    /// the mask program. `gbm` must outlive the returned `Gl`.
    pub fn new(gbm: &gbm::Device<File>) -> anyhow::Result<Gl> {
        // SAFETY: loads libEGL; no preconditions beyond the library being sane.
        let egl = unsafe { egl::DynamicInstance::<egl::EGL1_5>::load_required() }
            .map_err(|e| anyhow!("libEGL.so.1: {e}"))?;
        // SAFETY: a live gbm_device pointer is what EGL_PLATFORM_GBM_KHR expects.
        let display = unsafe {
            egl.get_platform_display(EGL_PLATFORM_GBM_KHR, gbm.as_raw() as *mut c_void, &[egl::ATTRIB_NONE])
        }
        .map_err(|e| anyhow!("eglGetPlatformDisplay(GBM): {e}"))?;
        egl.initialize(display).map_err(|e| anyhow!("eglInitialize: {e}"))?;
        let exts = egl.query_string(Some(display), egl::EXTENSIONS).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        ensure!(exts.contains("EGL_EXT_image_dma_buf_import"), "EGL lacks EGL_EXT_image_dma_buf_import");
        egl.bind_api(egl::OPENGL_ES_API).map_err(|e| anyhow!("eglBindAPI: {e}"))?;
        let config = egl
            .choose_first_config(
                display,
                &[
                    egl::SURFACE_TYPE, egl::PBUFFER_BIT,
                    egl::RENDERABLE_TYPE, egl::OPENGL_ES2_BIT,
                    egl::RED_SIZE, 8, egl::GREEN_SIZE, 8, egl::BLUE_SIZE, 8, egl::ALPHA_SIZE, 8,
                    egl::NONE,
                ],
            )
            .map_err(|e| anyhow!("eglChooseConfig: {e}"))?
            .context("no EGL config with GLES2 + RGBA8")?;
        let context = egl
            .create_context(display, config, None, &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE])
            .map_err(|e| anyhow!("eglCreateContext: {e}"))?;
        if exts.contains("EGL_KHR_surfaceless_context") {
            egl.make_current(display, None, None, Some(context)).map_err(|e| anyhow!("eglMakeCurrent: {e}"))?;
        } else {
            let pbuffer = egl
                .create_pbuffer_surface(display, config, &[egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE])
                .map_err(|e| anyhow!("eglCreatePbufferSurface: {e}"))?;
            egl.make_current(display, Some(pbuffer), Some(pbuffer), Some(context))
                .map_err(|e| anyhow!("eglMakeCurrent(pbuffer): {e}"))?;
        }

        let proc_addr = |name: &str| egl.get_proc_address(name).map_or(std::ptr::null(), |f| f as *const c_void);
        // SAFETY: the context is current on this thread; glow only stores pointers.
        let gl = unsafe { glow::Context::from_loader_function(|s| proc_addr(s)) };
        let load_image_fn = |name: &str| -> anyhow::Result<ImageTargetFn> {
            let p = proc_addr(name);
            ensure!(!p.is_null(), "{name} unavailable");
            // SAFETY: both OES entry points have exactly this signature.
            Ok(unsafe { std::mem::transmute::<*const c_void, ImageTargetFn>(p) })
        };
        let image_target_texture = load_image_fn("glEGLImageTargetTexture2DOES")?;
        let image_target_renderbuffer = load_image_fn("glEGLImageTargetRenderbufferStorageOES")?;

        // SAFETY: plain GL calls on the current context; every handle used
        // below was created here.
        let (program, vbo, a_pos, u_uv, u_size, u_radius, u_tex) = unsafe {
            let compile = |kind: u32, src: &str| -> anyhow::Result<glow::Shader> {
                let s = gl.create_shader(kind).map_err(|e| anyhow!(e))?;
                gl.shader_source(s, src);
                gl.compile_shader(s);
                ensure!(gl.get_shader_compile_status(s), "shader compile: {}", gl.get_shader_info_log(s));
                Ok(s)
            };
            let vs = compile(glow::VERTEX_SHADER, VERT)?;
            let fs = compile(glow::FRAGMENT_SHADER, FRAG)?;
            let program = gl.create_program().map_err(|e| anyhow!(e))?;
            gl.attach_shader(program, vs);
            gl.attach_shader(program, fs);
            gl.link_program(program);
            ensure!(gl.get_program_link_status(program), "program link: {}", gl.get_program_info_log(program));
            gl.delete_shader(vs);
            gl.delete_shader(fs);
            let uniform = |n: &str| gl.get_uniform_location(program, n).with_context(|| format!("uniform {n}"));
            let a_pos = gl.get_attrib_location(program, "a_pos").context("attribute a_pos")?;
            let (u_uv, u_size, u_radius, u_tex) = (uniform("u_uv")?, uniform("u_size")?, uniform("u_radius")?, uniform("u_tex")?);

            // One triangle covering clip space.
            let verts: [f32; 6] = [-1.0, -1.0, 3.0, -1.0, -1.0, 3.0];
            let bytes: Vec<u8> = verts.iter().flat_map(|f| f.to_ne_bytes()).collect();
            let vbo = gl.create_buffer().map_err(|e| anyhow!(e))?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(glow::ARRAY_BUFFER, &bytes, glow::STATIC_DRAW);
            (program, vbo, a_pos, u_uv, u_size, u_radius, u_tex)
        };

        tracing::info!("GL thumbnail pass ready ({})", egl.query_string(Some(display), egl::VENDOR).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
        Ok(Gl {
            egl,
            display,
            _context: context,
            gl,
            image_target_texture,
            image_target_renderbuffer,
            program,
            vbo,
            a_pos,
            u_uv,
            u_size,
            u_radius,
            u_tex,
            trash: Arc::default(),
        })
    }

    fn create_image(&self, dma: &Dmabuf) -> anyhow::Result<egl::Image> {
        let attribs = dmabuf_image_attribs(dma);
        // SAFETY: a null client buffer is what EGL_LINUX_DMA_BUF_EXT requires.
        let buffer = unsafe { egl::ClientBuffer::from_ptr(std::ptr::null_mut()) };
        // SAFETY: EGL_NO_CONTEXT is required for this target.
        let no_context = unsafe { egl::Context::from_ptr(egl::NO_CONTEXT) };
        self.egl
            .create_image(self.display, no_context, EGL_LINUX_DMA_BUF_EXT, buffer, &attribs)
            .map_err(|e| anyhow!("eglCreateImage(dmabuf {}x{} fmt {:#x} mod {:#x}): {e}", dma.width, dma.height, dma.format, dma.modifier))
    }

    /// Import a capture buffer as a sampleable texture. Only dmabuf
    /// backings can be imported; shm captures take the raw path.
    pub fn import_source(&self, backing: &Arc<BufferSource>) -> anyhow::Result<SourceTexture> {
        let BufferSource::Dma(dma) = &**backing else { anyhow::bail!("shm capture buffer; GL pass needs a dmabuf") };
        let image = self.create_image(dma)?;
        // SAFETY: GL calls on the current context.
        let texture = unsafe {
            let t = self.gl.create_texture().map_err(|e| anyhow!(e))?;
            self.gl.bind_texture(glow::TEXTURE_2D, Some(t));
            (self.image_target_texture)(glow::TEXTURE_2D, image.as_ptr());
            self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
            self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
            let err = self.gl.get_error();
            ensure!(err == glow::NO_ERROR, "glEGLImageTargetTexture2DOES: GL error {err:#x}");
            t.0.get()
        };
        Ok(SourceTexture { texture, image: image.as_ptr() as usize, trash: self.trash.clone() })
    }

    /// Allocate a thumbnail-sized ABGR8888 dmabuf on `gbm` (an explicit
    /// modifier from `modifiers` if any, else implicit + linear), import it
    /// as a renderbuffer-backed framebuffer, and wrap it as a `BufferSource`
    /// the UI can attach to a subsurface.
    pub fn create_target(&self, gbm: &gbm::Device<File>, modifiers: &[u64], (width, height): (u32, u32)) -> anyhow::Result<Target> {
        ensure!(width > 0 && height > 0, "zero-sized target");
        let format = gbm::Format::try_from(ABGR8888)?;
        let bo = if modifiers.is_empty() {
            gbm.create_buffer_object::<()>(width, height, format, gbm::BufferObjectFlags::RENDERING | gbm::BufferObjectFlags::LINEAR)
        } else {
            gbm.create_buffer_object_with_modifiers2::<()>(
                width,
                height,
                format,
                modifiers.iter().map(|m| gbm::Modifier::from(*m)),
                gbm::BufferObjectFlags::RENDERING,
            )
        }
        .context("gbm_bo_create for thumbnail target")?;
        let modifier: u64 = bo.modifier().into();
        let mut planes = Vec::new();
        for i in 0..bo.plane_count() as i32 {
            planes.push(Plane { fd: bo.fd_for_plane(i)?, plane_idx: i as u32, offset: bo.offset(i), stride: bo.stride_for_plane(i) });
        }
        // The fds keep the memory alive; the bo handle itself is not needed.
        drop(bo);
        let dma = Dmabuf { width: width as i32, height: height as i32, planes, format: ABGR8888, modifier };
        let image = self.create_image(&dma)?;
        // SAFETY: GL calls on the current context.
        let (framebuffer, renderbuffer) = unsafe {
            let rb = self.gl.create_renderbuffer().map_err(|e| anyhow!(e))?;
            self.gl.bind_renderbuffer(glow::RENDERBUFFER, Some(rb));
            (self.image_target_renderbuffer)(glow::RENDERBUFFER, image.as_ptr());
            let fb = self.gl.create_framebuffer().map_err(|e| anyhow!(e))?;
            self.gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fb));
            self.gl.framebuffer_renderbuffer(glow::FRAMEBUFFER, glow::COLOR_ATTACHMENT0, glow::RENDERBUFFER, Some(rb));
            let status = self.gl.check_framebuffer_status(glow::FRAMEBUFFER);
            let err = self.gl.get_error();
            if status != glow::FRAMEBUFFER_COMPLETE || err != glow::NO_ERROR {
                self.gl.delete_framebuffer(fb);
                self.gl.delete_renderbuffer(rb);
                let _ = self.egl.destroy_image(self.display, image);
                anyhow::bail!("target framebuffer incomplete (status {status:#x}, GL error {err:#x}, modifier {modifier:#x})");
            }
            (fb.0.get(), rb.0.get())
        };
        Ok(Target {
            backing: Arc::new(dma.into()),
            size: (width, height),
            framebuffer,
            renderbuffer,
            image: image.as_ptr() as usize,
            trash: self.trash.clone(),
        })
    }

    /// Draw `src` (upright per `transform`) into `dst` with corners of
    /// `radius_px` masked out; blocks until the GPU is done so the buffer
    /// can be handed to the compositor.
    pub fn render(&self, src: &SourceTexture, dst: &Target, transform: Transform, radius_px: u32) -> anyhow::Result<()> {
        self.collect_trash();
        let (w, h) = dst.size;
        // SAFETY: GL calls on the current context with handles we created.
        unsafe {
            let gl = &self.gl;
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo(dst.framebuffer)));
            gl.viewport(0, 0, w as i32, h as i32);
            gl.disable(glow::BLEND);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::DEPTH_TEST);
            gl.use_program(Some(self.program));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(tex(src.texture)));
            gl.uniform_1_i32(Some(&self.u_tex), 0);
            gl.uniform_matrix_3_f32_slice(Some(&self.u_uv), false, &uv_matrix(transform));
            gl.uniform_2_f32(Some(&self.u_size), w as f32, h as f32);
            gl.uniform_1_f32(Some(&self.u_radius), mask_radius(radius_px, dst.size) as f32);
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(self.vbo));
            gl.enable_vertex_attrib_array(self.a_pos);
            gl.vertex_attrib_pointer_f32(self.a_pos, 2, glow::FLOAT, false, 0, 0);
            gl.draw_arrays(glow::TRIANGLES, 0, 3);
            gl.finish();
            let err = gl.get_error();
            ensure!(err == glow::NO_ERROR, "render: GL error {err:#x}");
        }
        Ok(())
    }

    fn collect_trash(&self) {
        let trash = std::mem::take(&mut *self.trash.lock().unwrap());
        // SAFETY: freeing handles we created, on the context's thread.
        unsafe {
            for f in trash.framebuffers {
                self.gl.delete_framebuffer(fbo(f));
            }
            for r in trash.renderbuffers {
                self.gl.delete_renderbuffer(rbo(r));
            }
            for t in trash.textures {
                self.gl.delete_texture(tex(t));
            }
            for i in trash.images {
                let _ = self.egl.destroy_image(self.display, egl::Image::from_ptr(i as *mut c_void));
            }
        }
    }

    /// Render a white 2×2 texture into a fresh 64×64 target with a 16 px
    /// radius and read it back: the corner must be transparent, the centre
    /// opaque white. Used by `yutani doctor`.
    pub fn self_test(&self, gbm: &gbm::Device<File>) -> anyhow::Result<()> {
        let target = self.create_target(gbm, &[], (64, 64))?;
        // SAFETY: GL calls on the current context.
        let texture = unsafe {
            let t = self.gl.create_texture().map_err(|e| anyhow!(e))?;
            self.gl.bind_texture(glow::TEXTURE_2D, Some(t));
            self.gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::RGBA as i32, 2, 2, 0, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelUnpackData::Slice(Some(&[255u8; 16])));
            self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
            self.gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
            t.0.get()
        };
        let src = SourceTexture { texture, image: 0, trash: self.trash.clone() };
        self.render(&src, &target, Transform::Normal, 16)?;
        let mut px = vec![0u8; 64 * 64 * 4];
        // SAFETY: the target framebuffer is still bound after `render`.
        unsafe {
            self.gl.read_pixels(0, 0, 64, 64, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
        }
        let at = |x: usize, y: usize| &px[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4];
        ensure!(at(0, 0)[3] == 0, "corner pixel not transparent: {:?}", at(0, 0));
        ensure!(at(32, 32) == [255, 255, 255, 255], "centre pixel not opaque white: {:?}", at(32, 32));
        ensure!(at(32, 0)[3] == 255, "top-edge pixel not opaque: {:?}", at(32, 0));
        Ok(())
    }
}

/// First render node under /dev/dri, for `yutani doctor` (the app uses the
/// device the compositor names in its dmabuf feedback).
pub fn open_render_node() -> anyhow::Result<gbm::Device<File>> {
    let mut nodes: Vec<_> = std::fs::read_dir("/dev/dri")?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("renderD")))
        .collect();
    nodes.sort();
    let path = nodes.first().context("no /dev/dri/renderD* node")?;
    let file = File::options().read(true).write(true).open(path).with_context(|| format!("open {}", path.display()))?;
    Ok(gbm::Device::new(file)?)
}
```

- [ ] **Step 2: Build**

Run: `cargo build -q 2>&1 | grep -E 'error|warning: unused' -A5 | head -60`
Expected: clean. Likely nits to fix on the spot: `egl::Image::as_ptr` visibility (if private, use `std::mem::transmute::<egl::Image, *mut c_void>(image)` — `Image` is `#[repr(transparent)]` over a pointer; check with `grep -n 'pub fn as_ptr' -B3 ~/.cargo/registry/src/*/khronos-egl-6.0.0/src/lib.rs`); `khronos_egl::Error: std::error::Error` (if not, keep the `map_err(|e| anyhow!("…{e}"))` form everywhere as written).

- [ ] **Step 3: Doctor line**

In `src/doctor.rs`, in `run()` after the protocol table loop and before the `if all_required_present` block:

```rust
    // GPU thumbnail pass: EGL on the first render node + a tiny offscreen render.
    let gl_status = crate::backend::gl::open_render_node()
        .and_then(|gbm| {
            let gl = crate::backend::gl::Gl::new(&gbm)?;
            gl.self_test(&gbm)
        })
        .map(|()| "ok".to_string())
        .unwrap_or_else(|err| format!("unavailable: {err:#} (thumbnails will have square corners)"));
    println!("\n{:<56} {}", "gl (rounded corners)", gl_status);
```

- [ ] **Step 4: Run the doctor**

Run: `cargo build -q 2>&1 | grep -E '^error' -A5; ./target/debug/yutani doctor | tail -4`
Expected:
```
gl (rounded corners)                                     ok

OK: cosmic-comp advertises everything Yutani needs.
```
If it prints `unavailable: …`, the reason is the bug to fix before continuing (typical: `glEGLImageTargetRenderbufferStorageOES` needs `GL_OES_EGL_image` — present on Mesa; `target framebuffer incomplete` → try `modifiers = &[DRM_FORMAT_MOD_LINEAR]` in `self_test` and, if that works, make `create_target` fall back to an explicit LINEAR modifier when the implicit allocation's framebuffer is incomplete).

- [ ] **Step 5: Tests still green, commit**

Run: `cargo test -q 2>&1 | tail -2` → `ok. 57 passed` (50 + 7).

```bash
git add src/backend/gl.rs src/doctor.rs
git commit -m "feat(gl): EGL/GLES2 thumbnail pass — dmabuf import, masked render into thumbnail-sized targets, doctor self-test

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 4: Backend integration — GL state, commands, per-session thumbnail pool

**Files:**
- Modify: `src/backend/mod.rs` (`Cmd`, `AppData`, `handle_cmd`, `start`)
- Modify: `src/backend/buffer.rs` (`Buffer.source`)
- Modify: `src/backend/capture.rs` (`ScreencopySession.thumb`, `ready`)

**Interfaces:**
- Consumes: `gl::{Gl, SourceTexture, Target, modifiers_for, ABGR8888}` from Task 3.
- Produces: `Cmd::SetThumbSize(Handle, (u32, u32))`, `Cmd::SetCornerRadius(u32)` for Task 5.

- [ ] **Step 1: `Buffer.source`**

In `src/backend/buffer.rs` add to `Buffer`:

```rust
    /// EGL import of `backing`, created on first use by the GL pass and
    /// kept for the buffer's lifetime (imports are per buffer, not per frame).
    pub source: Option<super::gl::SourceTexture>,
```

and `source: None,` in both constructors (`create_gbm_buffer`, `create_shm_buffer`).

- [ ] **Step 2: Commands and state in `src/backend/mod.rs`**

Extend `Cmd`:

```rust
    /// Physical-pixel size of this client's thumbnail surface; the GL pass
    /// renders into buffers of exactly this size.
    SetThumbSize(Handle, (u32, u32)),
    /// Corner mask radius in physical pixels (0 = square).
    SetCornerRadius(u32),
```

Add to `AppData`:

```rust
    pub gl: GlState,
    pub thumb_sizes: HashMap<Handle, (u32, u32)>,
    pub corner_radius_px: u32,
```

and above `pub struct AppData`:

```rust
/// The GL thumbnail pass is created lazily on the first dmabuf frame (it
/// needs the compositor's main device) and disabled for good after
/// repeated failures.
pub enum GlState {
    Untried,
    Ready { gl: gl::Gl, consecutive_failures: u32 },
    Disabled,
}

/// Consecutive per-frame GL failures before the pass is switched off.
pub const GL_MAX_FAILURES: u32 = 3;
```

Initialise in `start`: `gl: GlState::Untried, thumb_sizes: HashMap::new(), corner_radius_px: 8,`.

Add to `handle_cmd`:

```rust
            Cmd::SetThumbSize(handle, size) => {
                self.thumb_sizes.insert(handle, size);
            }
            Cmd::SetCornerRadius(px) => {
                self.corner_radius_px = px;
            }
```

Add these methods to `impl AppData` (after `handle_cmd`):

```rust
    /// The GL pass, initialising it on first call. `None` when unavailable.
    fn gl_init(&mut self) -> bool {
        if !matches!(self.gl, GlState::Untried) {
            return matches!(self.gl, GlState::Ready { .. });
        }
        let Some(dev) = self.dmabuf_feedback.as_ref().map(|f| f.main_device()) else { return false };
        let gbm = match self.gbm_devices.gbm_device(dev) {
            Ok(Some((_, gbm))) => gbm,
            Ok(None) => {
                tracing::warn!("no gbm device for the compositor's main device; thumbnails will have square corners");
                self.gl = GlState::Disabled;
                return false;
            }
            Err(err) => {
                tracing::warn!("cannot open gbm device: {err}; thumbnails will have square corners");
                self.gl = GlState::Disabled;
                return false;
            }
        };
        match gl::Gl::new(gbm) {
            Ok(gl) => {
                self.gl = GlState::Ready { gl, consecutive_failures: 0 };
                true
            }
            Err(err) => {
                tracing::warn!("GL thumbnail pass unavailable: {err:#}; thumbnails will have square corners");
                self.gl = GlState::Disabled;
                false
            }
        }
    }

    /// Record a per-frame GL failure; after `GL_MAX_FAILURES` in a row the
    /// pass is disabled for the rest of the session.
    fn gl_failed(&mut self, err: anyhow::Error) {
        if let GlState::Ready { consecutive_failures, .. } = &mut self.gl {
            *consecutive_failures += 1;
            if *consecutive_failures >= GL_MAX_FAILURES {
                tracing::warn!("GL thumbnail pass failed {GL_MAX_FAILURES} times ({err:#}); disabling, thumbnails will have square corners");
                self.gl = GlState::Disabled;
            } else {
                tracing::debug!("GL thumbnail pass failed: {err:#}; raw frame this time");
            }
        }
    }

    /// Modifiers the compositor accepts for ABGR8888, from its feedback.
    fn thumb_modifiers(&self) -> Vec<u64> {
        let Some(fb) = self.dmabuf_feedback.as_ref() else { return Vec::new() };
        let table: Vec<(u32, u64)> = fb.format_table().iter().map(|f| (f.format, f.modifier)).collect();
        let tranches: Vec<&[u16]> = fb.tranches().iter().map(|t| t.formats.as_slice()).collect();
        gl::modifiers_for(&table, &tranches, gl::ABGR8888)
    }

    /// Run the GL pass for `front` into `thumb` (allocating or re-allocating
    /// the pool for `size`), returning the buffer to ship. `Err` means the
    /// caller ships the raw frame.
    fn gl_process(
        &mut self,
        front: &mut Buffer,
        thumb: &mut Option<capture::ThumbPool>,
        size: (u32, u32),
        transform: wl_output::Transform,
    ) -> anyhow::Result<Arc<cosmic::iced::platform_specific::shell::subsurface_widget::BufferSource>> {
        if !self.gl_init() {
            anyhow::bail!("GL pass unavailable");
        }
        let modifiers = self.thumb_modifiers();
        let dev = self.dmabuf_feedback.as_ref().map(|f| f.main_device()).context("no dmabuf feedback")?;
        let radius = self.corner_radius_px;
        let AppData { gl, gbm_devices, .. } = self;
        let GlState::Ready { gl, .. } = gl else { anyhow::bail!("GL pass unavailable") };
        let (_, gbm) = gbm_devices.gbm_device(dev)?.context("gbm device vanished")?;

        if thumb.as_ref().is_none_or(|t| t.size != size) {
            *thumb = Some(capture::ThumbPool {
                size,
                targets: [gl.create_target(gbm, &modifiers, size)?, gl.create_target(gbm, &modifiers, size)?],
                release: None,
            });
        }
        let pool = thumb.as_mut().unwrap();
        if front.source.is_none() {
            front.source = Some(gl.import_source(&front.backing)?);
        }
        // Render into the back target, then make it the front.
        gl.render(front.source.as_ref().unwrap(), &pool.targets[1], transform, radius)?;
        pool.targets.rotate_left(1);
        Ok(pool.targets[0].backing.clone())
    }
```

Add `use anyhow::Context as _;` and `use buffer::Buffer;` to the imports of `mod.rs` (check `Buffer` is `pub struct` — it is).

- [ ] **Step 3: `ThumbPool` and the `ready()` path in `src/backend/capture.rs`**

Add after `BUFFER_COUNT`:

```rust
/// Thumbnail-sized GL targets for one session: [front, back], rotated on
/// every processed frame. `release` is the compositor's release of the
/// front target (the next render into it must wait for that).
pub struct ThumbPool {
    pub size: (u32, u32),
    pub targets: [super::gl::Target; BUFFER_COUNT],
    pub release: Option<SubsurfaceBufferRelease>,
}
```

Add `pub thumb: Option<ThumbPool>,` to `ScreencopySession` (after `release`) and `thumb: None,` in `ScreencopySession::new`. Make `buffers`, `release`, `last_submit`, `session`, `in_flight`, `consecutive_failures` stay as they are.

Replace the body of `ready()` from `// Back buffer now holds the newest frame` through `self.send_event(Event::Frame(...))` with:

```rust
        // Back buffer now holds the newest frame: make it the front.
        buffers.rotate_left(1);
        buffers[0].damage.clear();
        for buffer in &mut buffers[1..] {
            buffer.damage.extend_from_slice(&frame.damage);
        }
        let transform = match frame.transform {
            WEnum::Value(t) => t,
            WEnum::Unknown(_) => cctk::wayland_client::protocol::wl_output::Transform::Normal,
        };

        // GL pass: the front capture buffer → a thumbnail-sized, corner-masked
        // target. Falls back to the raw frame if the pass is unavailable, the
        // UI hasn't told us a size yet, or this frame's render failed.
        let thumb_size = self.thumb_sizes.get(&capture.handle).copied();
        let front_size = buffers[0].size;
        let mut processed: Option<(Arc<BufferSource>, (u32, u32))> = None;
        let mut gl_error = None;
        if let Some(size) = thumb_size {
            let ScreencopySession { buffers: bufs, thumb, .. } = state;
            let front = &mut bufs.as_mut().unwrap()[0];
            match self.gl_process(front, thumb, size, transform) {
                Ok(backing) => processed = Some((backing, size)),
                Err(err) => gl_error = Some(err),
            }
        }
        let Some(buffers) = state.buffers.as_mut() else { return };

        let (subsurface_buffer, release, image) = match processed {
            Some((backing, (w, h))) => {
                let (sb, release) = SubsurfaceBuffer::new(backing);
                let image = CaptureImage {
                    buffer: sb.clone(),
                    width: w,
                    height: h,
                    transform: cctk::wayland_client::protocol::wl_output::Transform::Normal,
                };
                (sb, release, image)
            }
            None => {
                let (sb, release) = SubsurfaceBuffer::new(buffers[0].backing.clone());
                let image = CaptureImage { buffer: sb.clone(), width: front_size.0, height: front_size.1, transform };
                (sb, release, image)
            }
        };
        drop(subsurface_buffer);
        // What the next submit must wait for: the buffer the compositor is
        // now holding — the GL target if we rendered, else the raw front.
        let previous_release = if processed.is_some() {
            let pool = state.thumb.as_mut().unwrap();
            pool.release.replace(release)
        } else {
            state.release.replace(release)
        };
        let last_submit = state.last_submit;
        let session_id = state.session.clone();

        // Next capture: after the previous front buffer is released by the
        // compositor and at least one frame interval since the last submit.
        // The wait is computed *after* the release resolves, since waiting
        // for release can itself take longer than the frame interval.
        let capture_for_task = capture.clone();
        let conn = conn.clone();
        let qh = qh.clone();
        let fps = self.fps.clone();
        self.thread_pool.spawn_ok(async move {
            if let Some(release) = previous_release {
                release.await;
            }
            let wait = frame_interval(&fps).saturating_sub(last_submit.elapsed());
            if !wait.is_zero() {
                futures_timer::Delay::new(wait).await;
            }
            let mut guard = capture_for_task.session.lock().unwrap();
            if let Some(state) = guard.as_mut() {
                // Belt and braces: `in_flight` is the primary guard against a
                // stale task racing a restarted session on the same `Arc`,
                // but also bail if this task no longer targets the session
                // it was spawned for.
                if state.session != session_id {
                    return;
                }
                state.submit(&capture_for_task, &conn, &qh);
            }
        });

        drop(guard);
        if let Some(err) = gl_error {
            self.gl_failed(err);
        }
        self.send_event(Event::Frame(capture.handle.clone(), image));
```

Notes for the implementer:
- `state` is the `&mut ScreencopySession` obtained at the top of `ready()`; the destructuring `let ScreencopySession { buffers: bufs, thumb, .. } = state;` reborrows two fields disjointly so `gl_process` (which takes `&mut self` = `AppData`) can run while the session guard is held. `self.gl_process` does not touch `self.captures`, so no deadlock.
- Add `use cosmic::iced::platform_specific::shell::subsurface_widget::BufferSource;` to the imports.
- `SubsurfaceBuffer` must be `Clone` (it is: `Arc` newtype). If `clone()` is not derived on this pinned iced, construct `image` first and pass `image.buffer.clone()` nowhere — i.e. build the tuple as `(release, CaptureImage { buffer: sb, .. })` and drop the extra binding.
- The raw-path `release` and the pool `release` are kept separately on purpose: when the pass flips between raw and processed (e.g. a size change fails to allocate), each buffer family waits on its own last release.

- [ ] **Step 4: Build, test, smoke**

Run: `cargo build -q 2>&1 | grep -E '^(error|warning)' -A6 | head -80; cargo test -q 2>&1 | tail -2`
Expected: clean build (borrow-checker adjustments are expected here; keep the structure), `ok. 57 passed`.

Smoke (EVE running, no `SetThumbSize` sent yet by the UI, so frames should still flow raw):
`RUST_LOG=yutani=debug timeout 6 ./target/debug/yutani 2>&1 | sed 's/\x1b\[[0-9;]*m//g' | grep -E 'create_surface|GL|gl |panic|error' | cut -c1-140`
Expected: `create_surface …`, no panics, no `GL` lines (pass not yet triggered).

- [ ] **Step 5: Commit**

```bash
git add src/backend/mod.rs src/backend/buffer.rs src/backend/capture.rs
git commit -m "feat(backend): route captured frames through the GL mask pass into thumbnail-sized targets; SetThumbSize/SetCornerRadius commands

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

### Task 5: UI sends sizes and radius; hands-on verification; spec amendments

**Files:**
- Modify: `src/ui/mod.rs` (`create_surface`, `leave_canvas`, `resize_if_needed`, `apply_config`, `on_backend` `CmdSender`, `forget_surface`)
- Modify: `docs/superpowers/specs/2026-09-11-yutani-design.md`, `docs/superpowers/specs/2026-09-12-yutani-gpu-thumbnails-design.md`

**Interfaces:**
- Consumes: `Cmd::SetThumbSize`, `Cmd::SetCornerRadius` (Task 4); `Output.scale` (Task 1).

- [ ] **Step 1: Helpers in `impl App` (`src/ui/mod.rs`, next to `send`)**

```rust
    /// Integer scale of the output `client` is (or would be) shown on.
    fn scale_for(&self, client: &Client) -> i32 {
        self.output_for(&client.info)
            .and_then(|handle| self.outputs.iter().find(|o| o.handle == handle))
            .map_or(1, |o| o.scale)
    }

    /// Tell the backend the physical size to render this client's frames at.
    fn send_thumb_size(&self, handle: &Handle, logical: (u32, u32)) {
        let Some(client) = self.clients.get(handle) else { return };
        let s = self.scale_for(client) as u32;
        self.send(Cmd::SetThumbSize(handle.clone(), (logical.0 * s, logical.1 * s)));
    }

    /// Mask radius in physical pixels. Outputs may differ in scale; the
    /// radius is global, so use the largest scale in use (a 1 px error on
    /// a lower-scale output is invisible).
    fn send_corner_radius(&self) {
        let s = self.outputs.iter().map(|o| o.scale).max().unwrap_or(1) as u32;
        self.send(Cmd::SetCornerRadius(self.config.corner_radius * s));
    }
```

- [ ] **Step 2: Call sites**

- `create_surface`: right after `client.last_size = Some((width, height));` (and the `tracing::info!` line) add `self.send_thumb_size(handle, (width, height));`. Note `client` is a `&mut` borrow of `self.clients` — end it first: move the `send_thumb_size` call after the `tracing::info!(...)` line and before `let create = get_layer_surface(...)`; the borrow of `client` ends at its last use.
- `leave_canvas`: after `self.clients.get_mut(handle).unwrap().last_size = Some((w, h));` add `self.send_thumb_size(handle, (w, h));`.
- `resize_if_needed`: after `self.clients.get_mut(handle).unwrap().last_size = Some(size);` add `self.send_thumb_size(handle, size);`.
- `on_backend`, `Event::CmdSender(sender)` arm: after `self.cmd = Some(sender);` add `self.send_corner_radius();`.
- `apply_config`: where Task 1 removed the `corner_radius` block, add
  ```rust
        if new.corner_radius != self.config.corner_radius {
            self.config.corner_radius = new.corner_radius;
            self.send_corner_radius();
        }
  ```
  (before `self.config = new;` — it reads `self.config.corner_radius`, so set it first as shown).
- `update`, `Msg::Wayland(WaylandEvent::Output(..))` arm: after `self.on_output(event, output);` add `self.send_corner_radius();` (scale may have changed; cheap).

- [ ] **Step 3: Build and test**

Run: `cargo build -q 2>&1 | grep -E '^(error|warning)' -A6; cargo test -q 2>&1 | tail -2`
Expected: clean; `ok. 57 passed`.

- [ ] **Step 4: Smoke test with EVE running**

```bash
pkill -f target/debug/yutani; sleep 0.5
LOG=/tmp/claude-1000/-home-user-Yutani/eb30ce00-6f26-46d7-ad81-bfd5bce301f6/scratchpad/yutani.log
(RUST_LOG=yutani=debug setsid nohup ./target/debug/yutani >$LOG 2>&1 &); sleep 5
sed 's/\x1b\[[0-9;]*m//g' $LOG | grep -E 'GL|gl |create_surface|panic|error|warn' | cut -c1-160
```
Expected: `create_surface … width=484 height=274`, `GL thumbnail pass ready (…)`, no warnings/errors. Then:

```bash
cd /tmp/claude-1000/-home-user-Yutani/eb30ce00-6f26-46d7-ad81-bfd5bce301f6/scratchpad
timeout 15 cosmic-screenshot --interactive=false --modal=false --notify=false -s "$PWD" >/dev/null
python3 - <<'EOF'
from PIL import Image; import glob
im = Image.open(sorted(glob.glob('Screenshot_*.png'))[-1])
x0 = 2560 + 1038; y0 = 36 + 8          # DP-2 is the right half; panel is ~36 px
im.crop((x0-6, y0-6, x0+60, y0+60)).resize((66*8, 66*8), Image.NEAREST).save('corner_after.png')
im.crop((x0-4, y0+274-60, x0+300, y0+274+8)).resize((304*3, 68*3), Image.NEAREST).save('label_after.png')
EOF
```
Read `corner_after.png`: the image must show a curved top-left corner with the desktop visible outside the curve and the 2 px border following the curve. Read `label_after.png`: the "KestrelVance" pill must be fully visible over the image. Check `top -b -n1 -p $(pgrep -f target/debug/yutani)`: CPU no higher than before (~4 %).

Also: `echo '(corner_radius: 24)' > ~/.config/yutani/config.ron` → within a second the corner is visibly rounder (re-screenshot); `rm ~/.config/yutani/config.ron` → back to 8. `pkill -f target/debug/yutani`.

If the corner is still square: check the log for `GL thumbnail pass failed` (per-frame error text says why) and for `SetThumbSize` never being sent (add a `tracing::debug!` in `send_thumb_size` if needed).

- [ ] **Step 5: Spec amendments**

`docs/superpowers/specs/2026-09-11-yutani-design.md`:
- §5 (capture), append a paragraph: *Captured frames are post-processed on the GPU (EGL/GLES2 on the render node, `backend/gl.rs`) into thumbnail-sized ABGR8888 dmabufs with the corner mask baked into alpha before they reach the UI; if GL is unavailable the raw frame is shown (square corners). See `2026-09-12-yutani-gpu-thumbnails-design.md`.*
- §6 "Thumbnail widget": replace any mention of `cosmic_corner_radius_layer_v1` with: *the image subsurface sits below the parent (`z = -1`); border, name label and pin are iced-drawn over it; corners are rounded in the buffer itself.*
- §9 config example: delete the `opacity: 1.0,` line; change the `corner_radius` comment to `// px, baked into the frame on the GPU`.

`docs/superpowers/specs/2026-09-12-yutani-gpu-thumbnails-design.md`: in §2.1 change "ARGB8888" to "ABGR8888 (the capture format; GL RGBA byte order)" and replace the modifier sentence with *modifier chosen by gbm from the compositor's dmabuf-feedback modifiers for ABGR8888 (`create_buffer_object_with_modifiers2`), implicit + `LINEAR` when the feedback has none*. In §2.4 rename `mask_radius(corner_radius, scale, size)` to `mask_radius(radius_px, size)` and add `dmabuf_image_attribs`, `buffer_coords`.

- [ ] **Step 6: Commit**

```bash
git add src/ui/mod.rs docs/superpowers/specs/
git commit -m "feat(ui): send thumbnail size and mask radius to the backend; spec: GPU-masked thumbnails

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01R66vpAiTLPkLmk4SuttcFH"
```

---

## Self-review

**Spec coverage:** §2.1 GPU pass — Tasks 3-4 (context, import cache via `Buffer.source`, per-client two-target pool, `glFinish`, source released right away since the raw `release` is no longer awaited when processed, `Transform::Normal` on processed frames, `SetThumbSize`/`SetCornerRadius`, raw before size known). §2.2 UI — Task 1 (`z(-1)`, opacity removed, request machinery removed, `Output.scale`) and Task 5 (sizes/radius). §2.3 failure handling — Task 4 (`GlState`, `GL_MAX_FAILURES`), Task 3 (doctor line). §2.4 tests — Task 2 (7 unit tests), Task 3 (self-test), Task 5 (smoke). §2.5 deps — Task 2. §3 spec amendments — Task 5.

**Placeholder scan:** none; every code step carries the code. API-name risks are flagged with the exact grep to resolve them (`Transform` variant names, `Image::as_ptr`, `khronos_egl::Error`).

**Type consistency:** `gl::Gl::{new, import_source, create_target, render, self_test}` signatures identical in Tasks 3 and 4; `capture::ThumbPool { size, targets: [Target; 2], release }` used the same way in `gl_process` and `ready`; `Cmd::SetThumbSize(Handle, (u32, u32))` / `SetCornerRadius(u32)` identical in Tasks 4 and 5; `Output.scale: i32` defined in Task 1, read in Task 5; `Buffer.source: Option<gl::SourceTexture>` defined in Task 4 Step 1 and used in Step 2.
