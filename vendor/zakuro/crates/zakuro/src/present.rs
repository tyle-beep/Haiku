//! wiring a presentation backend to a winit window.

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextApi, ContextAttributesBuilder, PossiblyCurrentContext, Version};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SwapInterval, WindowSurface};
use glutin_winit::{DisplayBuilder, GlWindow};
use raw_window_handle::HasWindowHandle;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

use zakuro_gpu::backend::gl::GlPresenter;
use zakuro_gpu::backend::vulkan::VulkanPresenter;
use zakuro_gpu::{Overlay, PresentError, Presenter, RendererKind, ScreenImage, ScreenLayout, ScreenOrientation, SharedDevice};

// exactly one backend exists per process, so the size of the largest variant
// is not worth an extra allocation to avoid.
#[allow(clippy::large_enum_variant)]
pub enum Backend {
    OpenGl {
        presenter: GlPresenter,
        context: PossiblyCurrentContext,
        surface: Surface<WindowSurface>,
    },
    Vulkan(VulkanPresenter),
    /// no window output, used when a backend could not start but the emulator
    /// should still run.
    None,
}

impl Backend {
    pub fn create(
        event_loop: &ActiveEventLoop,
        attributes: WindowAttributes,
        kind: RendererKind,
    ) -> Result<(Window, Backend), String> {
        match kind {
            RendererKind::OpenGl => create_opengl(event_loop, attributes),
            RendererKind::Vulkan => create_vulkan(event_loop, attributes),
            RendererKind::Software => {
                let window = event_loop
                    .create_window(attributes)
                    .map_err(|e| e.to_string())?;
                Ok((window, Backend::None))
            }
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Backend::OpenGl { presenter, .. } => presenter.name(),
            Backend::Vulkan(presenter) => presenter.name(),
            Backend::None => "none",
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        let (width, height) = (width.max(1), height.max(1));
        match self {
            Backend::OpenGl {
                presenter,
                context,
                surface,
            } => {
                presenter.resize(width, height);
                surface.resize(
                    context,
                    std::num::NonZeroU32::new(width).unwrap(),
                    std::num::NonZeroU32::new(height).unwrap(),
                );
            }
            Backend::Vulkan(presenter) => presenter.resize(width, height),
            Backend::None => {}
        }
    }

    /// whether showing a frame waits for the display, the OpenGL swap waits
    /// for the vertical blank.
    pub fn waits_for_display(&self) -> bool {
        match self {
            Backend::OpenGl { .. } => true,
            Backend::Vulkan(presenter) => presenter.waits_for_display(),
            Backend::None => false,
        }
    }

    /// smooth: the present thread's next slot, the display's refresh
    /// period, the pictures waiting and the last one's slot; none where
    /// presents are not paced by it (see zakuro-gpu backend vulkan_pacer).
    pub fn pacing(&self) -> Option<(std::time::Instant, std::time::Duration, usize, Option<std::time::Instant>)> {
        match self {
            Backend::Vulkan(presenter) => presenter.pacing(),
            _ => None,
        }
    }

    /// smooth: when the next present goes out, none for at once.
    pub fn set_present_at(&mut self, at: Option<std::time::Instant>) {
        if let Backend::Vulkan(presenter) = self {
            presenter.set_present_at(at);
        }
    }

    /// smooth2: the next present draws this picture over the top screen,
    /// blended in this far (a cross-fade).
    pub fn set_crossfade(&mut self, crossfade: Option<(zakuro_gpu::GpuScreen, f32)>) {
        if let Backend::Vulkan(presenter) = self {
            presenter.set_crossfade(crossfade);
        }
    }

    /// pulse2: Divine Pulse's effect over the top screen in the next present.
    pub fn set_pulse_effect(&mut self, effect: Option<zakuro_gpu::PulseEffect>) {
        if let Backend::Vulkan(presenter) = self {
            presenter.set_pulse_effect(effect);
        }
    }

    /// pulse2: whether Divine Pulse's effects can be drawn.
    pub fn pulse_effects(&self) -> bool {
        matches!(self, Backend::Vulkan(presenter) if presenter.pulse_effects())
    }

    /// smooth2: with VSync off, the rate presents go out at (see
    /// VulkanPresenter::set_free_rate).
    pub fn set_free_rate(&mut self, rate: Option<f64>) {
        if let Backend::Vulkan(presenter) = self {
            presenter.set_free_rate(rate);
        }
    }

    /// vrrcap: the display's refresh rate is variable (see
    /// VulkanPresenter::set_vrr).
    pub fn set_vrr(&mut self, on: bool) {
        if let Backend::Vulkan(presenter) = self {
            presenter.set_vrr(on);
        }
    }

    /// smooth: the monitor's refresh rate, where DWM gives no refresh grid.
    pub fn set_refresh_hint(&mut self, hz: f64) {
        if let Backend::Vulkan(presenter) = self {
            presenter.set_refresh_hint(hz);
        }
    }

    /// hfr: whether presents wait for the display's refresh.
    pub fn set_vsync(&mut self, on: bool) {
        match self {
            Backend::OpenGl { context, surface, .. } => {
                let interval = match on {
                    true => SwapInterval::Wait(std::num::NonZeroU32::new(1).unwrap()),
                    false => SwapInterval::DontWait,
                };
                if let Err(error) = surface.set_swap_interval(context, interval) {
                    log::warn!("could not set the swap interval, {error}");
                }
            }
            Backend::Vulkan(presenter) => presenter.set_vsync(on),
            Backend::None => {}
        }
    }

    /// the Vulkan presenter's device, for the renderer to draw on so the
    /// screens are shown straight from it, when the two can share one.
    pub fn shared_device(&self) -> Option<std::sync::Arc<SharedDevice>> {
        match self {
            Backend::Vulkan(presenter) => presenter.shared_device(),
            _ => None,
        }
    }

    pub fn set_scaling(&mut self, filter: zakuro_gpu::ScreenFilter, integer: bool) {
        match self {
            Backend::OpenGl { presenter, .. } => presenter.set_scaling(filter, integer),
            Backend::Vulkan(presenter) => presenter.set_scaling(filter, integer),
            Backend::None => {}
        }
    }

    pub fn set_stereo(&mut self, stereo: zakuro_gpu::Stereo) {
        match self {
            Backend::OpenGl { presenter, .. } => presenter.set_stereo(stereo),
            Backend::Vulkan(presenter) => presenter.set_stereo(stereo),
            Backend::None => {}
        }
    }

    pub fn set_layout(&mut self, arrangement: ScreenLayout) {
        match self {
            Backend::OpenGl { presenter, .. } => presenter.set_layout(arrangement),
            Backend::Vulkan(presenter) => presenter.set_layout(arrangement),
            Backend::None => {}
        }
    }

    pub fn set_orientation(&mut self, orientation: ScreenOrientation) {
        match self {
            Backend::OpenGl { presenter, .. } => presenter.set_orientation(orientation),
            Backend::Vulkan(presenter) => presenter.set_orientation(orientation),
            Backend::None => {},
        }
    }

    pub fn set_top_width(&mut self, width: f32) {
        match self {
            Backend::OpenGl { presenter, .. } => presenter.set_top_width(width),
            Backend::Vulkan(presenter) => presenter.set_top_width(width),
            Backend::None => {}
        }
    }

    /// bottom: whether this presenter draws the frontend's composition.
    pub fn composes(&self) -> bool {
        matches!(self, Backend::Vulkan(_))
    }

    /// bottom: the window as the frontend composed it, for the next frame.
    pub fn set_composition(&mut self, composition: Option<zakuro_gpu::backend::bottom_panel::Composition>) {
        if let Backend::Vulkan(presenter) = self {
            presenter.set_composition(composition);
        }
    }

    pub fn present(
        &mut self,
        top: ScreenImage<'_>,
        bottom: ScreenImage<'_>,
        right: ScreenImage<'_>,
        overlay: &Overlay,
    ) -> Result<(), PresentError> {
        match self {
            Backend::OpenGl {
                presenter,
                context,
                surface,
            } => {
                presenter.present(top, bottom, right, overlay)?;
                surface
                    .swap_buffers(context)
                    .map_err(|e| PresentError::Backend(e.to_string()))
            }
            Backend::Vulkan(presenter) => presenter.present(top, bottom, right, overlay),
            Backend::None => Ok(()),
        }
    }
}

fn create_opengl(
    event_loop: &ActiveEventLoop,
    attributes: WindowAttributes,
) -> Result<(Window, Backend), String> {
    let template = ConfigTemplateBuilder::new().with_alpha_size(8);
    let (window, config) = DisplayBuilder::new()
        .with_window_attributes(Some(attributes))
        .build(event_loop, template, |configs| {
            // any config will do, prefer more color bits if offered.
            configs
                .reduce(|best, candidate| {
                    if candidate.num_samples() > best.num_samples() {
                        candidate
                    } else {
                        best
                    }
                })
                .expect("at least one GL config")
        })
        .map_err(|e| format!("no OpenGL configuration: {e}"))?;

    let window = window.ok_or("glutin did not create a window")?;
    let raw_handle = window
        .window_handle()
        .map_err(|e| e.to_string())?
        .as_raw();

    let display = config.display();
    // ask for 3.3 core, which is what the presentation shaders need.
    let context_attributes = ContextAttributesBuilder::new()
        .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
        .build(Some(raw_handle));
    let not_current = unsafe { display.create_context(&config, &context_attributes) }
        .map_err(|e| format!("could not create an OpenGL 3.3 context: {e}"))?;

    let surface_attributes = window
        .build_surface_attributes(Default::default())
        .map_err(|e| e.to_string())?;
    let surface = unsafe { display.create_window_surface(&config, &surface_attributes) }
        .map_err(|e| format!("could not create a window surface: {e}"))?;

    let context = not_current
        .make_current(&surface)
        .map_err(|e| format!("could not make the context current: {e}"))?;

    // present on vblank so the frontend's own pacing has something to work
    // against, a failure here is not fatal.
    let _ = surface.set_swap_interval(
        &context,
        SwapInterval::Wait(std::num::NonZeroU32::new(1).unwrap()),
    );

    let size = window.inner_size();
    let presenter = unsafe {
        GlPresenter::new(
            |name| {
                let name = std::ffi::CString::new(name).unwrap();
                display.get_proc_address(&name)
            },
            (size.width.max(1), size.height.max(1)),
        )
    }
    .map_err(|e| e.to_string())?;

    Ok((
        window,
        Backend::OpenGl {
            presenter,
            context,
            surface,
        },
    ))
}

fn create_vulkan(
    event_loop: &ActiveEventLoop,
    attributes: WindowAttributes,
) -> Result<(Window, Backend), String> {
    let window = event_loop
        .create_window(attributes)
        .map_err(|e| e.to_string())?;
    let size = window.inner_size();
    let presenter = VulkanPresenter::new(&window, (size.width.max(1), size.height.max(1)))
        .map_err(|e| e.to_string())?;
    Ok((window, Backend::Vulkan(presenter)))
}
