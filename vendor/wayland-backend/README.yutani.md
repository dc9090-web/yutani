# Vendored wayland-backend 0.3.17 (patched)

A verbatim copy of `wayland-backend` 0.3.17 from crates.io with one change,
in `src/sys/client_impl/mod.rs`, `impl Drop for ConnectionState`: each
proxy's user data is cleared and its `alive` flag stored `false` *before*
the `ProxyUserData` box is freed — the same order `destroy_object_inner`
already uses for a single object.

## Why

Yutani (and any libcosmic app) crashed with a heap-use-after-free on the
SCTK event-loop thread whenever a burst of layer surfaces was destroyed and
recreated. AddressSanitizer (2026-09-13, three identical reports) showed:

- **freed by** the main thread in `ConnectionState::drop`, reached from
  `drop_glue::<iced_winit::clipboard::Clipboard>` — libcosmic's iced fork
  replaces its clipboard when the window it was connected to goes away, and
  `smithay_clipboard` holds a *guest* backend (`from_foreign_display`) on
  the app's own `wl_display`, with its own `wl_output` binds;
- **read by** the SCTK thread in `InnerBackend::get_data`, from
  `WlOutput::from_id` inside `WlSurface::parse_event` — a
  `wl_surface.enter(wl_output)` for a new surface that named the *old*
  clipboard's output bind (the compositor sends `enter` for every bound
  output resource), buffered in wayland-client's queue between the
  dispatcher building the `ObjectId` and the callback parsing it.

libwayland refcounts a proxy that a queued event names, so the proxy struct
outlives `wl_proxy_destroy`; its user-data pointer still pointed at the
freed box, and the `alive` clone inside the `ObjectId` still said `true`.

## Remove when

Upstream `Smithay/wayland-rs` ships a release whose `ConnectionState::drop`
clears the user data and stores `alive = false` before freeing. Then delete
this directory and the `[patch.crates-io]` entry in the root `Cargo.toml`.
