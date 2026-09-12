# Yutani — GPU-masked thumbnails (rounded corners done right)

**Status:** approved (Daniel, 2026-09-12)
**Amends:** `2026-09-11-yutani-design.md` §5 (capture), §6 (thumbnail widget), §9 (config)

## 1. Problem

Rounded thumbnail corners were implemented by asking cosmic-comp for a
`cosmic_corner_radius_layer_v1` radius on each thumbnail layer surface. That
never rounded the captured image. Verified in cosmic-comp source (1.7.0 and
the 1.8.0 running on the target machine since 2026-09-12): the layer radius is
a *hint* — it shapes the background-blur element only. Pixel clipping
(`ClippedSurfaceRenderElement`) happens only when `should_clip` is true, and
that is set only for toplevel windows; `Stage::LayerSurface` passes `false`.
So the thumbnail's subsurface was never clipped at any opacity; the 0.999-alpha
workaround in commit `6443d3e` and the "verified by eye" note in plan 3 were
both wrong (what looked rounded was the 2 px iced border).

Nothing client-side can mask a subsurface through Wayland (`wp_viewporter`
crops rectangles, `wp_alpha_modifier` is uniform), and this iced fork cannot
import a dmabuf into its wgpu image pipeline. The corners therefore have to be
baked into the buffer we attach to the subsurface.

A second defect found while investigating: the subsurface widget defaults to
`z = 0`, which places it **above** the parent layer surface. The iced-drawn
border, the character-name label and the pin glyph are all painted on the
parent and were hidden under the image (faintly visible only because opacity
was 0.98).

## 2. Design

### 2.1 Backend GPU pass — `src/backend/gl.rs` (new)

A GLES2 context on the compositor thread, created lazily on the first
captured frame:

- `eglGetPlatformDisplayEXT(EGL_PLATFORM_GBM_KHR, gbm_device)` on the render
  node `gbm_devices.rs` already opens; `EGL_KHR_surfaceless_context` (pbuffer
  fallback); calls through `glow` loaded with `eglGetProcAddress`.
- One program. Vertex: full-screen triangle. Fragment: bilinear sample of the
  captured frame through a UV matrix that undoes the frame's
  `wl_output::Transform`, multiplied by a rounded-rect SDF alpha, written as
  premultiplied ARGB.
- **Input:** the capture dmabuf (planes/fds/modifier from the existing
  `Buffer`) imported with `eglCreateImageKHR(EGL_LINUX_DMA_BUF_EXT)` and bound
  via `glEGLImageTargetTexture2DOES`. Cached per capture buffer (the pool has
  two), so imports happen twice per client, not per frame.
- **Output:** per client, a pool of two thumbnail-sized gbm BOs, ABGR8888
  (the capture format; GL RGBA byte order), modifier chosen by gbm from the
  compositor's dmabuf-feedback modifiers for ABGR8888
  (`create_buffer_object_with_modifiers2`), implicit + `LINEAR` when the
  feedback has none. Each is imported as
  an EGLImage → renderbuffer → FBO, and wrapped as a `wl_buffer` through the
  existing `zwp_linux_dmabuf` path so it can be shipped to the UI as a
  `SubsurfaceBuffer` exactly like today's raw buffers. After rendering,
  `glFinish()`; the source capture buffer is released immediately, so capture
  no longer waits on the compositor releasing a full-resolution buffer. The
  next render into an output buffer waits for the compositor's release of it
  (same `SubsurfaceBufferRelease` await as today, moved to the output pool).
- `CaptureImage` for a processed frame carries `Transform::Normal` and the
  **source** window's size in display orientation (axes swapped for a
  90°/270° capture transform) — not the target's. `width`/`height` exist
  only so the UI can derive the thumbnail's aspect; reporting the target size
  (which includes the border) fed back into that computation and grew the
  thumbnail by 1 px per frame (found in the Task 5 smoke test).

New commands from the UI:

- `Cmd::SetThumbSize(Handle, (u32, u32))` — target size in physical pixels.
  Sent whenever a client's `last_size` is set or changes (create, hover zoom,
  config). A change reallocates that client's output pool.
- `Cmd::SetCornerRadius(u32)` — mask radius in physical pixels, sent at start
  and on config change.

A frame that arrives before a size is known ships raw (as today).

### 2.2 UI

- `Subsurface::new(..).z(-1)`: the image sits below the parent, so border,
  label and pin glyph draw over it. No `.alpha()`, `.transform(Normal)`.
- Delete `opacity` everywhere (config field, `validate`, spec, tests, the
  `.alpha()` hack). Thumbnails are always fully opaque. Old config files with
  `opacity:` still parse — serde ignores unknown fields.
- Delete `round_surface`, `awaiting_radius`, `Client.rounded`, the
  `corner_radius` protocol request and its `RedrawRequested` hook.
- `corner_radius` (config, default 8) stays and drives both the iced border
  radius and the mask radius: `mask_radius = corner_radius × output scale`,
  clamped to half the short side of the target. Output scale is the integer
  `scale_factor` from the sctk `OutputInfo` the UI already receives
  (`Output` gains a `scale: i32` field, default 1).

### 2.3 Failure handling

- EGL/GL setup failure (no EGL, no GBM platform, no dma-buf import extension,
  shader compile error): one `warn!`, `gl = None`, and every frame ships raw.
  Square corners, but label and border are visible thanks to `z(-1)`.
- Per-frame failure (image import, FBO incomplete, GL error): ship the raw
  frame for that capture; three consecutive failures disable GL for the rest
  of the session with a `warn!`.
- `yutani doctor` gains a `gl:` line: EGL init on the main device plus a
  64×64 offscreen render, reported `ok` / `unavailable: <reason>`.

### 2.4 Testing

Pure, unit-tested in `gl.rs`:

- `uv_matrix(transform) -> [f32; 6]` for all eight `wl_output::Transform`
  values (table test against known corner mappings).
- `mask_radius(radius_px, size)` clamping.
- `dmabuf_image_attribs` (EGL attribute list for a dmabuf import) and
  `buffer_coords` (per-transform UV mapping behind `uv_matrix`;
  `swaps_axes` alongside it).
- `pick_modifier(feedback_mods, egl_mods) -> Modifier` (intersection, LINEAR
  fallback, INVALID handling).

GL itself is verified by `yutani doctor` and by the smoke test: a screenshot
crop of a thumbnail corner shows a curve, the name label is visible, and
`top` shows CPU no higher than before. Config tests are updated for the
removed field.

### 2.5 Dependencies

`khronos-egl` 6 with the `dynamic` feature (libEGL.so.1 loaded at runtime
with `libloading`, exactly as wgpu-hal does — a missing library is just
another reason for the raw fallback, not a link failure) and `glow` 0.16.
Both are already in `Cargo.lock` via wgpu-hal — no new dependency tree.
`gbm::AsRaw` exposes the raw `gbm_device` pointer needed for the platform
display.

## 3. Spec amendments (applied to the main design doc)

- §5: capture buffers are post-processed on the GPU into thumbnail-sized,
  corner-masked buffers before display; raw fallback when GL is unavailable.
- §6 thumbnail widget: subsurface below the parent; overlays drawn by iced.
- §9: `opacity` removed from the config example; `corner_radius` semantics
  noted as client-side.
