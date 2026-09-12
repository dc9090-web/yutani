//! GPU post-processing of captured frames: each capture dmabuf is drawn into
//! a thumbnail-sized dmabuf with rounded corners baked into its alpha.
//! cosmic-comp never clips layer surfaces to a corner radius (the
//! cosmic_corner_radius_layer_v1 hint only shapes blur), so this is the only
//! way to get rounded thumbnails. See the 2026-09-12 spec.

use anyhow::{Context as _, anyhow, ensure};
use cosmic::cctk::wayland_client::protocol::wl_output::Transform;
use cosmic::iced::platform_specific::shell::subsurface_widget::{BufferSource, Dmabuf, Plane};
use gbm::AsRaw;
use glow::HasContext;
use khronos_egl as egl;
use std::ffi::c_void;
use std::fs::File;
use std::num::NonZeroU32;
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

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
        // Out-of-range plane indices are skipped: eglCreateImage then fails
        // with a clear error instead of this indexing panicking.
        let Some(&[fd, offset, pitch, lo, hi]) = PLANE_ATTRIBS.get(plane.plane_idx as usize) else { continue };
        a.extend([fd, plane.fd.as_raw_fd() as egl::Attrib, offset, plane.offset as egl::Attrib, pitch, plane.stride as egl::Attrib]);
        if dma.modifier != DRM_FORMAT_MOD_INVALID {
            a.extend([lo, (dma.modifier & 0xffff_ffff) as egl::Attrib, hi, (dma.modifier >> 32) as egl::Attrib]);
        }
    }
    a.push(egl::ATTRIB_NONE);
    a
}

const EGL_PLATFORM_GBM_KHR: egl::Enum = 0x31D7;

// `u_size` is used by the fragment shader only: GLSL ES 1.00 requires a
// uniform shared by both stages to have the same precision, and the vertex
// stage defaults to highp while the fragment stage may lack it.
const VERT: &str = r#"
attribute vec2 a_pos;
uniform mat3 u_uv;
varying vec2 v_uv;
varying vec2 v_o;
void main() {
    // Clip space → output space in *Wayland* orientation (y down): GL row 0
    // is memory row 0, which the compositor treats as the top row.
    vec2 o = vec2(a_pos.x * 0.5 + 0.5, 0.5 - a_pos.y * 0.5);
    v_uv = (u_uv * vec3(o, 1.0)).xy;
    v_o = o;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}
"#;

const FRAG: &str = r#"
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
uniform sampler2D u_tex;
uniform vec2 u_size;
uniform float u_radius;
varying vec2 v_uv;
varying vec2 v_o;
void main() {
    vec2 h = u_size * 0.5; // `half` is reserved in GLSL ES 1.00
    vec2 d = abs(v_o * u_size - h) - (h - vec2(u_radius));
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
        // Mesa's GBM platform only offers window-surface configs, so a
        // pbuffer-capable config is asked for only when one is needed.
        let surfaceless = exts.contains("EGL_KHR_surfaceless_context");
        let mut config_attribs = vec![
            egl::RENDERABLE_TYPE, egl::OPENGL_ES2_BIT,
            egl::RED_SIZE, 8, egl::GREEN_SIZE, 8, egl::BLUE_SIZE, 8, egl::ALPHA_SIZE, 8,
        ];
        if !surfaceless {
            config_attribs.extend([egl::SURFACE_TYPE, egl::PBUFFER_BIT]);
        }
        config_attribs.push(egl::NONE);
        let config = egl
            .choose_first_config(display, &config_attribs)
            .map_err(|e| anyhow!("eglChooseConfig: {e}"))?
            .context("no EGL config with GLES2 + RGBA8")?;
        let context = egl
            .create_context(display, config, None, &[egl::CONTEXT_CLIENT_VERSION, 2, egl::NONE])
            .map_err(|e| anyhow!("eglCreateContext: {e}"))?;
        if surfaceless {
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

        // Bound outside the macro: `tracing::info!` brings `tracing::field::display`
        // into scope, which would shadow the local `display`.
        let vendor = egl.query_string(Some(display), egl::VENDOR).map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        tracing::info!("GL thumbnail pass ready ({vendor})");
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

#[cfg(test)]
mod tests {
    use super::*;
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
