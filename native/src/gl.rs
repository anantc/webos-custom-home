//! Skia's GLES backend on the app's own SDL window.
//!
//! Adapted from Punktfunk's src/console/gl.rs; see NOTICE.
//! The SDL window owns one GL context, with Skia as its only renderer.

use anyhow::{anyhow, Result};
use skia_safe::gpu::{self, DirectContext, SurfaceOrigin};
use skia_safe::{ColorType, Surface};

const GL_RGBA8: u32 = 0x8058; // RGBA8888 framebuffer format

/// Cache ceiling, not an upfront allocation.
pub(crate) const GPU_CACHE_BYTES: usize = 32 * 1024 * 1024;

pub(crate) struct ConsoleGl {
    // Field order is drop order: GPU resources must die before their GL context.
    surface: Option<(Surface, u32, u32)>,
    context: DirectContext,
    /// Preserved across menu entries to retain compiled shaders and glyph atlases.
    ctx: sdl3::video::GLContext,
    /// What the window's config actually granted, not what was asked for — Skia must be told
    /// the truth or it clips paths against a buffer that is not there.
    stencil: usize,
    /// Last interval handed to the driver, so only a CHANGE is pushed — see
    /// [`Self::set_swap_interval`].
    swap_vsync: Option<bool>,
}

impl Drop for ConsoleGl {
    fn drop(&mut self) {
        // Cached images can outlive this owner. Make their eventual drops safe too.
        if self.ctx.is_current() {
            self.context.release_resources_and_abandon();
        } else {
            // Not current (teardown order); GPU objects go with the context itself.
            self.context.abandon();
        }
    }
}

impl ConsoleGl {
    pub(crate) fn new(window: &sdl3::video::Window, video: &sdl3::VideoSubsystem) -> Result<Self> {
        let ctx = window
            .gl_create_context()
            .map_err(|e| anyhow!("console GL context: {e}"))?;
        // The event loop sets the swap interval after initialization.
        let attr = video.gl_attr();
        let stencil = attr.stencil_size() as usize;
        tracing::info!(
            "console: GL context up — {:?}, alpha {} bits, stencil {stencil} bits",
            attr.context_profile(),
            attr.alpha_size(),
        );
        // SDL's resolver knows the webOS loader.
        // The resolver returns Option<FnPtr>; Skia wants a pointer, so None becomes null.
        let interface = gpu::gl::Interface::new_load_with(|name| {
            video
                .gl_get_proc_address(name)
                .map_or(std::ptr::null(), |f| f as *const std::ffi::c_void)
        })
        .ok_or_else(|| anyhow!("Skia: no GL interface from SDL's resolver"))?;
        let mut context = gpu::direct_contexts::make_gl(interface, None)
            .ok_or_else(|| anyhow!("Skia: DirectContext over GLES failed"))?;
        context.set_resource_cache_limit(GPU_CACHE_BYTES);
        tracing::info!(
            "console: Skia GL context, {} MB budget",
            GPU_CACHE_BYTES >> 20
        );
        Ok(Self {
            ctx,
            context,
            surface: None,
            stencil,
            swap_vsync: None,
        })
    }

    /// Ensure the window's context is current before drawing.
    pub(crate) fn make_current(&self, window: &sdl3::video::Window) -> Result<()> {
        if self.ctx.is_current() {
            return Ok(());
        }
        window
            .gl_make_current(&self.ctx)
            .map_err(|e| anyhow!("console: gl_make_current: {e}"))
    }

    /// Block the swap on the panel, or let it return immediately.
    /// Best effort: the event loop also enforces a frame budget if VSync is unavailable.
    pub(crate) fn set_swap_interval(&mut self, video: &sdl3::VideoSubsystem, vsync: bool) {
        if self.swap_vsync == Some(vsync) {
            return;
        }
        let interval = if vsync {
            sdl3::video::SwapInterval::VSync
        } else {
            sdl3::video::SwapInterval::Immediate
        };
        match video.gl_set_swap_interval(interval) {
            Ok(()) => tracing::debug!(
                "console: swap interval {}",
                if vsync { "vsync" } else { "immediate" }
            ),
            Err(e) => tracing::debug!(
                "console: no {} swap interval ({e}) — pacing on the frame budget",
                if vsync { "vsync" } else { "immediate" },
            ),
        }
        self.swap_vsync = Some(vsync);
    }

    /// Re-wraps on drawable size change to keep Skia from clipping against a stale buffer.
    pub(crate) fn surface(&mut self, w: u32, h: u32) -> Result<&mut Surface> {
        if !matches!(self.surface, Some((_, sw, sh)) if sw == w && sh == h) {
            self.surface = None;
            let fb = gpu::gl::FramebufferInfo {
                fboid: 0,
                format: GL_RGBA8,
                protected: gpu::Protected::No,
            };
            let target =
                gpu::backend_render_targets::make_gl((w as i32, h as i32), None, self.stencil, fb);
            let surface = gpu::surfaces::wrap_backend_render_target(
                &mut self.context,
                &target,
                SurfaceOrigin::BottomLeft,
                ColorType::RGBA8888,
                None,
                None,
            )
            .ok_or_else(|| anyhow!("Skia: could not wrap the window framebuffer ({w}x{h})"))?;
            tracing::info!("console: drawing at {w}x{h}");
            self.surface = Some((surface, w, h));
        }
        // The branch above either returned an error or filled it.
        let surface = &mut self.surface.as_mut().expect("just wrapped").0;
        // A cached canvas must not retain a previous frame's transform.
        surface.canvas().reset_matrix();
        Ok(surface)
    }

    /// SDL still owns the swap.
    pub(crate) fn flush(&mut self) {
        self.context.flush_and_submit();
    }

    /// Release cached resources during clean shutdown.
    pub(crate) fn release_resources(&mut self) {
        self.surface = None;
        self.context.free_gpu_resources();
    }
}
