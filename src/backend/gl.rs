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
