# Vendored iced_wgpu 0.14.0 (patched)

A copy of `iced/wgpu` from libcosmic at rev `a401af8b`, the rev the root
`Cargo.toml` pins, with two changes:

- `Cargo.toml`: every `workspace = true` field and dependency is written
  out with the value iced's workspace gave it (the libcosmic-tree crates
  `iced_debug`, `iced_graphics` and `build_helpers` come from the same git
  rev), and the `[lints]` table is dropped.
- `src/window/compositor.rs`: the `Engine` is built with `None` instead of
  `settings.antialiasing`, so the triangle pipeline never uses MSAA.

## Why

libcosmic's `applet::run` always asks for antialiasing and gives an applet
no way to turn it off. With MSAA on, the panel popover kept frozen copies
of its canvas shapes — sand grains, the Launch EVE hazard caps, the `⋯`
mark — about 26 px below where they belong (2026-10-09). That happened
whenever an EVE login shrank the Accounts section and with it the
popover. The copies survived closing and reopening the popover (a new
surface), so they lived in GPU state the process shares between windows;
the MSAA targets are the one such piece the triangle pipeline owns. A build
with antialiasing off showed no copies, logged in or out. Quads and text
were never affected.

The exact fault inside the MSAA path is not pinned down. Most popover
canvases draw axis-aligned rectangles; only the diagonal hazard stripes and
the small arrows lose smooth edges, which is a small price.

## Remove when

The libcosmic pin moves (re-copy and re-apply, or check whether upstream
fixed the MSAA path), or libcosmic lets an applet set `antialiasing`.
Then delete this directory and the `[patch."https://github.com/pop-os/libcosmic"]`
entry in the root `Cargo.toml`.
