//! fates-gfx: the native renderer for the Fire Emblem Fates PC port.
//!
//! The game draws each screen through its engine's prim layer
//! (prim::Context, see `prim`). Overrides in overrides/gfx_native.c call the
//! hooks below at each step of Graphics::Render; this crate decodes the
//! display's prim::State list into native draws and renders them with its
//! own Vulkan device (`render`). States it can't draw natively yet (those
//! whose prim::Callback draws a 3D scene, effect or capture, and lines) are
//! "islands": the emulated path draws every state up to the last island, and
//! the native renderer draws the rest over that picture, so the order the
//! game drew in is kept exactly.
//!
//! FATES_NATIVE_GFX selects the mode:
//! - unset / 0: off, the emulated path draws everything (default);
//! - `1`: native, as above;
//! - `shadow`: the emulated path draws everything and the native renderer
//!   draws the frames it can draw on its own alongside, to compare them
//!   (FATES_GFX_DUMP=dir writes both pictures and the differences).

pub mod command_alter;
pub mod debug3d;
// gfx-eft: nw::eft particles
pub mod eft;
// gfx-final: the emulated GPU's work in a native session, and its lease
pub mod emulated;
pub mod h3d;
pub mod lines; // gfx-lines: prim line states
mod moflexhd; // moflexhd: a movie's pictures, few kept
// gfx-hfr: pictures in between the native frames
pub mod hfr;
pub mod hd; // hdpacks: texture-pack pictures, read in the background
pub mod models; // gfx-models: mesh translations kept while memory holds them
// gfx-perf: timing counters and content caches
pub mod perf;
pub mod cache;
pub mod blur;
pub mod bottom; // gfx-bottom
pub mod capture; // gfx-final: unit-icon textures
pub mod live2d; // jpn: Live2D models (My Room) drawn natively
// gfx-post: native post effects and the surfaces they draw into
pub mod post;
pub mod prim;
pub mod render;
pub mod scene;
pub mod scene2d; // gfx-terrain
pub mod setup;
// gfx-shadow: the shadow pass and its receivers
pub mod shadow;
// gfx-post: GPU memory as native surfaces (shared: post effects, shadow maps)
pub mod surfaces;
// gfx-gaps: map unit sprites and HP gauges (UnitIcon::BatchCallback)
pub mod unit_icon;
pub mod wide2; // wide2: full-width 2D effects
pub mod wide_ui; // wide-ui: the ultrawide sides of 5:3 screens
pub mod gfx3; // gfx3: corner vignettes at the wide screen's corners
pub mod portrait; // gfx-2d: talk portraits' cut ends faded in the wide sides
pub mod edges; // gfx2d3: atlas quads clamped half a texel inside their cell
pub mod sunglasses; // sunglasses: Haiku's Sunglasses drawn over the hair
pub mod freecam; // gfxcastle: the battle free camera's sky poles and distance haze
pub mod castlecam; // castlecam: models the camera turned out of view, in the pictures in between
pub mod rtpace; // rtpace: pictures in between drawn on a thread of their own in real time
pub mod drawdist; // drawdist: far grass drawn in batches (Full Draw Distance)
pub mod mapjit; // mapjit: dev, the map's meshes on the screen in each picture
pub mod skybg; // skybg: the battle backdrop (dev trace)
pub mod vein2; // vein2: dev, how pictures in between treat particle draws

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use zakuro_core::{Screen, System};

use prim::{Decoded as PrimDecoded, Draw, Island, TexRef, Tables};
use render::{Decoded, Finish, Output, Renderer, Start};

/// The 3D hook table, `fates_gfx_hooks_3d` in gfx_native.c.
#[repr(C)]
pub struct Hooks3D {
    pub draw_mesh: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32, u32) -> i32>,
}

/// gfx-eft: the effect hook table, `fates_gfx_hooks_eft` in gfx_eft.c.
#[repr(C)]
pub struct HooksEft {
    pub draw: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32, u32) -> i32>,
    pub fog: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u32, u32, u32)>,
}

/// The table overrides/gfx_native.c calls through, `fates_gfx_hooks` there.
#[repr(C)]
pub struct Hooks {
    pub prim_target: Option<unsafe extern "C" fn(*mut c_void, u32, u32) -> i32>,
    pub clear: Option<unsafe extern "C" fn(*mut c_void)>,
    pub target_display: Option<unsafe extern "C" fn(*mut c_void, u32)>,
    pub transfer: Option<unsafe extern "C" fn(*mut c_void, u32)>,
    pub frame: Option<unsafe extern "C" fn(*mut c_void)>,
    pub prim_target_done: Option<unsafe extern "C" fn(*mut c_void, u32, u32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Off,
    Shadow,
    Native,
}

pub fn mode_from_env() -> Mode {
    // native-final: the pure build has no emulated GPU to fall back to or
    // to compare with (shadow mode)
    if !cfg!(feature = "emulation") {
        return Mode::Native;
    }
    match std::env::var("FATES_NATIVE_GFX").unwrap_or_default().to_ascii_lowercase().as_str() {
        "1" | "on" | "native" | "true" => Mode::Native,
        "shadow" => Mode::Shadow,
        _ => Mode::Off,
    }
}

/// One display's frame as the native renderer has it.
/// The scene islands of the display list the original RenderTarget is
/// running (native 3D path).
struct Islands {
    disp: usize,
    /// per island, its callback object (the SceneSystem) and the index of
    /// the frame draw its meshes go before
    slots: Vec<(u32, usize)>,
    /// the mesh draws each island's DrawMesh calls made, with their
    /// texture keys
    draws: HashMap<u32, Vec<(Draw, [Option<u64>; 3])>>,
    /// where the state list is and what it held, when it was compacted
    /// (native mode; shadow mode lets the original draw every state too)
    list: u32,
    saved: Vec<u32>,
    compacted: bool,
    /// gfx-post: the post effects recorded natively, by callback object
    posts: HashMap<u32, Arc<post::Block>>,
}

/// SceneSystem's OnBegin: the 3D scene callback the native path draws.
const SCENE_SYSTEM_ON_BEGIN: u32 = 0x001A_4360;

/// gfx-final: on the presenter's device each display has a renderer of its
/// own (and one for its pictures in between): a renderer waits for its
/// last submission before it records the next, and the two screens' frames
/// (and pictures in between) are drawn one after the other, so with one
/// renderer each waited for the other's. FATES_GFX_SPLIT=0: one renderer.
fn split_renderers() -> bool {
    !crate::perf::var("FATES_GFX_SPLIT").is_ok_and(|v| v == "0")
}

fn native3d_enabled() -> bool {
    crate::perf::var("FATES_GFX_NATIVE3D").is_ok_and(|v| v == "1") // gfx-perf: cached env
}

struct Frame {
    id: u64,
    /// 0 upper, 1 lower.
    disp: u32,
    /// the colour and depth the game cleared to; None: it drew over the
    /// display's previous picture.
    clear: Option<([f32; 4], f32)>,
    /// the frame drawn natively in full: scale and portrait pixels,
    /// rendered as soon as the frame is complete.
    rendered: Option<(u32, Arc<Vec<u8>>)>,
    /// the emulated path drew the first states, the native draws go over
    /// its picture.
    hybrid: bool,
    draws: Vec<Draw>,
    /// the content hash of each draw's textures (units 0..2), filled in when
    /// the frame is complete.
    texture_keys: Vec<[Option<u64>; 3]>,
    /// gfx-hfr: its draws against the display's frame before, for pictures
    /// in between (with frame interpolation on)
    hfr: Option<Arc<hfr::Between>>,
}

#[derive(Default, Debug, Clone)]
struct Stats {
    /// gfxmodels3: RenderTargets with no states over a frame not yet shown,
    /// left as they were (see prim_target)
    empty_redraws: u64,
    frames: u64,
    native: [u64; 2],
    hybrid: [u64; 2],
    emulated: [u64; 2],
    draws: u64,
    states: u64,
    callbacks: u64,
    /// jpn: Live2D meshes drawn natively, and left out (unreadable)
    live2d: u64,
    live2d_skipped: u64,
    lines: u64,
    depth: u64,
    composed: u64,
    /// Model::DrawMesh calls seen
    meshes: u64,
    /// time spent, in microseconds: recording draws (prim_target), completing
    /// frames (texture reads, full native renders), composing screens
    us_record: u64,
    us_complete: u64,
    us_compose: u64,
    /// islands by the OnBegin address of their prim::Callback
    callback_kinds: std::collections::BTreeMap<u32, u64>,
    /// gfx-bottom: uncleared frames seeded, chained, reseeded
    bottom: bottom::Stats,
}

struct Gfx {
    mode: Mode,
    tables: Option<Tables>,
    target: u32,
    cleared: Option<([f32; 4], f32)>,
    /// gfxmodels3: the clear above was taken by a frame of that display
    /// (see prim_target: a second RenderTarget without a clear of its own)
    clear_taken: [bool; 2],
    pending: [Option<Frame>; 2],
    /// gfx-bottom: with the gfx frame each was queued in (bottom::queue)
    queued: HashMap<u32, VecDeque<(u64, Option<Frame>)>>,
    committed: HashMap<u32, Arc<Frame>>,
    decoded: HashMap<(TexRef, u64), Arc<Decoded>>,
    /// gfx-2d: the gfx frame each decoded texture was last handed out in
    /// (trim_decoded)
    decoded_used: HashMap<(TexRef, u64), u64>,
    /// gfxcastle: the battle free camera's haze (freecam.rs)
    freecam: freecam::FreeCam,
    composed: HashMap<u32, (u64, Screen)>,
    renderer: Option<Renderer>,
    /// perf3: the side queue's renderer (Renderer::new_side), for shadow
    /// maps and icon captures; None and failed: they use `renderer`
    side: Option<Renderer>,
    side_failed: bool,
    /// the renderer on the presenter's device, once a presenter shares it:
    /// from then on frames are drawn on the GPU when shown, not read back
    gpu: Option<Renderer>,
    gpu_failed: bool,
    /// gfx-final: the lower display's renderer on the presenter's device
    /// (see split_renderers), the upper one's being `gpu`
    gpu_lower: Option<Renderer>,
    /// per display: frames complete but not drawn on the GPU yet, in order
    gpu_queue: [VecDeque<Arc<Frame>>; 2],
    /// per display: the frame last drawn for the presenter, the emulated
    /// image it was drawn over (hybrids), and its picture
    gpu_shown: [Option<(u64, Option<ash::vk::ImageView>, zakuro_gpu::GpuScreen)>; 2],
    /// the meshes translated so far, by MeshData address
    meshes: HashMap<u32, Arc<h3d::Mesh>>,
    /// gfx-models: the native path's translations, checked against memory
    /// at every draw (models.rs)
    models: models::Meshes,
    /// this frame's native H3D draws (FATES_GFX_SCENE3D), with their
    /// texture keys
    frame_shaded: Vec<(Draw, [Option<u64>; 3])>,
    /// programs shared by the native H3D draws (gfx-models: the vertex
    /// arrays are kept with the translations, models.rs)
    shaded_programs: HashMap<u64, Arc<Vec<u32>>>,
    /// the native 3D path (FATES_GFX_NATIVE3D): while the original
    /// RenderTarget runs a display's compacted list, the scene islands'
    /// draw positions by their callback object, the draws their DrawMesh
    /// calls made, and the state list to put back
    islands: Option<Islands>,
    /// callbacks traced (FATES_GFX_TRACE_CALLBACKS)
    traced_callbacks: std::collections::HashSet<(u32, u32)>,
    /// sub-mesh draws seen per vertex buffer (FATES_GFX_ORACLE)
    oracle_seen: HashMap<u32, u32>,
    /// this frame's Model::DrawMesh calls (FATES_GFX_DEBUG3D)
    frame3d: Vec<debug3d::MeshDraw>,
    /// a geometry-check picture waiting to be written beside the emulated one
    debug3d_pending: Option<(u64, Vec<u8>)>,
    /// per display: the last hybrid frame, drawn again over each new
    /// emulated picture of it (hfr's pictures in between)
    gpu_last_hybrid: [Option<Arc<Frame>>; 2],
    renderer_failed: bool,
    /// per display: whether its last frame was drawn natively in full, and
    /// that picture (scale, portrait pixels), which a frame drawn without a
    /// clear starts from.
    chain_valid: [bool; 2],
    chain: [Option<(u32, Arc<Vec<u8>>)>; 2],
    next_id: u64,
    stats: Stats,
    dump: Option<PathBuf>,
    dumped: u64,
    /// how the portrait framebuffer maps onto the upright screen, bits:
    /// 1 flips screen x, 2 flips screen y.

    /// whether texture rows are flipped (row 0 at t = 1).
    flip_t: bool,
    report_every: u64,
    // gfx-shadow: the shadow pass and surfaces, the CommandAlter record
    shadow: shadow::Shadows,
    alter: command_alter::Alter,
    /// gfx-post: the native post-effect layer, and the callback object
    /// being recorded
    post: post::State,
    post_callback: Option<u32>,
    /// gfx-eft: what the effect path did, and the state eft draws inherit
    /// from the scene's last mesh
    eft: eft::Stats,
    eft_inherit: eft::Inherit,
    /// sunglasses: the critical cut-in model whose face material holds
    /// Haiku's Sunglasses layer this frame: (model, frame, its texture
    /// unit's registers), for drawing the layer again over the hair
    sunglasses_cutin: Option<(u32, u64, [u32; 6])>,
    /// gfx-gaps: the UnitIcon::BatchCallback being recorded, and the
    /// callback states scene2d_end met, for their draws' places
    unit_icon: Option<(UnitSlot, unit_icon::Recorder)>,
    unit_pending: VecDeque<UnitSlot>,
    /// gfx-perf: content caches
    cache: cache::Caches,
    /// hdpacks: texture-pack pictures
    hd: hd::Hd,
    /// gfx-hfr: pictures in between
    hfr: hfr::State,
    /// gfx-final: the emulated GPU's work and lease
    emu: emulated::Audit,
    /// gfx-final: the UnitIconInstance::OnBegin being recorded, the prim
    /// context and display last drawn, icons made natively, and those
    /// waiting to be compared with the emulated ones (FATES_GFX_CAPTURE_CHECK)
    capture: Option<capture::Recorder>,
    capture_prim: (u32, u32),
    captures: u64,
    capture_checks: Vec<(capture::Target, Vec<u8>)>,
    /// per display: a UnitIconInstance island is in its frame, and whether
    /// its OnBegin came through the hook (a tail call can bypass it: then
    /// the emulated GPU must render the icons)
    capture_expected: [bool; 2],
    capture_seen: [bool; 2],
}

/// gfx-gaps: a UnitIcon::BatchCallback state in a scene's command list:
/// the scene island, where its draws go among the island's draws, the
/// prim state and context, the display.
#[derive(Clone, Copy, Debug)]
struct UnitSlot {
    scene: u32,
    at: usize,
    state: u32,
    prim_ctx: u32,
    disp: u32,
}

static GFX: Mutex<Option<Gfx>> = Mutex::new(None);

pub(crate) fn with<R>(f: impl FnOnce(&mut Gfx) -> R) -> Option<R> {
    let mut guard = GFX.lock().ok()?;
    guard.as_mut().map(f)
}

/// Installs the native renderer when FATES_NATIVE_GFX asks for it: fills
/// the C hook table and plugs into zakuro-core's screen seam.
pub fn install_3d(hooks: &mut Hooks3D) {
    if mode_from_env() == Mode::Off {
        return;
    }
    hooks.draw_mesh = Some(hook_draw_mesh);
}

/// Model::DrawMesh(model, render state, layer, mesh, world). For now the
/// native renderer only reads and checks the mesh (FATES_GFX_TRACE3D=N
/// prints the meshes of game frame N); the original still draws it.
unsafe extern "C" fn hook_draw_mesh(ctx: *mut c_void, model: u32, state: u32, layer: u32, mesh: u32, world: u32) -> i32 {
    let system = unsafe { system(ctx) };
    let _t = perf::scope(perf::C::Mesh); // gfx-perf
    with(|g| g.draw_mesh(system, model, state, layer, mesh, world)).unwrap_or(0)
}

/// gfx-post: the post-effect hook table, `fates_gfx_hooks_post` in
/// overrides/gfx_post.c.
pub fn install_post(hooks: &mut post::Hooks) {
    if mode_from_env() == Mode::Off {
        return;
    }
    post::install(hooks, hook_post_begin, hook_post_end, hook_post_call);
}

unsafe extern "C" fn hook_post_begin(ctx: *mut c_void, callback: u32) {
    let system = unsafe { system(ctx) };
    with(|g| g.post_begin(system, callback));
}

unsafe extern "C" fn hook_post_end(ctx: *mut c_void, callback: u32) {
    let system = unsafe { system(ctx) };
    with(|g| g.post_end(system, callback));
}

unsafe extern "C" fn hook_post_call(ctx: *mut c_void, function: u32, args: *const u32, count: u32) {
    let system = unsafe { system(ctx) };
    let args = unsafe { std::slice::from_raw_parts(args, count.min(16) as usize) };
    // eur: the function is the running build's; the recorders know the USA ones
    let function = zakuro_core::region::to_us(function);
    // gfx-lines: dev, FATES_GFX_DRAW_TRACE=1: every Pica::Draw a recorder
    // sees, with its caller (the guest lr)
    if matches!(function, 0x0013_01A8 | 0x0013_019C) && perf::var_os("FATES_GFX_DRAW_TRACE").is_some() {
        let lr = unsafe { *(ctx as *const u32).add(14) };
        let frame = zakuro_core::native_gfx::HEADLESS_FRAME.load(std::sync::atomic::Ordering::Relaxed);
        with(|g| eprintln!("gfx-draw-trace {function:08X} frame {frame} lr {lr:08X} args {args:08X?} capture {} unit_icon {} post {:?}", g.capture.is_some(), g.unit_icon.is_some(), g.post_callback));
    }
    with(|g| {
        // gfx-final: a UnitIconInstance::OnBegin being recorded
        if let Some(rec) = g.capture.as_mut() {
            rec.call(system, function, args);
            // gfx-units: FrameBuffer::MakeCommand(cmd, bool), the display's
            // framebuffer back, is OnBegin's last step: the recording ends
            // here, whether or not OnBegin's end hook runs
            if function == 0x0010_653C {
                g.capture_end_instance(system, "display framebuffer");
            }
            return;
        }
        match g.unit_icon.as_mut() {
            // gfx-gaps: a UnitIcon::BatchCallback being recorded
            Some((_, rec)) => {
                rec.call(system, function, args);
                // gfx-units: its last step; the end hook may not run
                if rec.ends_batch(system, function, args) {
                    g.unit_icon_end(system, 1);
                }
            }
            None => g.post.call(system, function, args),
        }
    });
}

/// gfxmap: dev, FATES_GFX_UNIT_LOG=1: each unit-icon batch (the map's
/// sprites, marks and gauges) as recorded or not, with its gfx frame
fn unit_log(frame: u64, what: &str, draws: usize) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| perf::var("FATES_GFX_UNIT_LOG").is_ok_and(|v| v == "1")) {
        eprintln!("gfxmap unit batch: frame {frame} game {} {what} {draws}", zakuro_core::native_gfx::HEADLESS_FRAME.load(std::sync::atomic::Ordering::Relaxed));
    }
}

/// gfx-gaps: the unit-icon hook table, `fates_gfx_hooks_unit_icon` in
/// overrides/gfx_unit_icon.c, with the native 3D path's scene 2D on.
/// wide2: overrides/widescreen.c's note of the wallpapers DrawRect
/// stretched, which the native 2D layer draws at their own size and
/// extends instead (wide2.rs); left unset with the emulated GPU.
/// gfx-2d: the render scale the last frame was drawn at, for
/// overrides/gfx_units.c (sprites placed in 3D snap to its pixels).
static RENDER_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

/// gfx-2d: fates_units_snap_hook: the render scale, or 1 with
/// FATES_GFX_UNITS_SNAP=0 (whole console pixels, as the game snaps).
pub extern "C" fn units_snap() -> i32 {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if !*ON.get_or_init(|| std::env::var("FATES_GFX_UNITS_SNAP").map_or(true, |v| v != "0")) {
        return 1;
    }
    RENDER_SCALE.load(std::sync::atomic::Ordering::Relaxed).clamp(1, 16) as i32
}

pub fn install_wide2(hook: &mut Option<extern "C" fn(u32, u32, f32, f32)>) {
    if mode_from_env() == Mode::Off || !wide2::enabled() {
        return;
    }
    *hook = Some(wide2::rect_hook);
}

pub fn install_unit_icon(hooks: &mut unit_icon::Hooks) {
    if mode_from_env() == Mode::Off || !native3d_enabled() || !unit_icon::enabled() || std::env::var("FATES_GFX_SCENE2D").is_ok_and(|v| v == "0") {
        return;
    }
    hooks.begin = Some(hook_unit_icon_begin);
    hooks.end = Some(hook_unit_icon_end);
}

unsafe extern "C" fn hook_unit_icon_begin(ctx: *mut c_void, callback: u32) {
    let system = unsafe { system(ctx) };
    with(|g| g.unit_icon_begin(system, callback));
}

unsafe extern "C" fn hook_unit_icon_end(ctx: *mut c_void, callback: u32) {
    let system = unsafe { system(ctx) };
    with(|g| g.unit_icon_end(system, callback));
}

/// jpn: the Live2D hook table, `fates_gfx_hooks_live2d` in
/// overrides/gfx_live2d.c (native mode, FATES_GFX_LIVE2D not 0).
pub fn install_live2d(hooks: &mut live2d::Hooks) {
    if mode_from_env() == Mode::Off || !live2d::enabled() {
        return;
    }
    hooks.draw = Some(hook_live2d_draw);
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn hook_live2d_draw(ctx: *mut c_void, this: u32, tex: u32, index_count: u32, vertex_count: u32, indices: u32, xys: u32, uvs: u32, opacity: u32, composition: u32) -> u32 {
    let system = unsafe { system(ctx) };
    with(|g| g.live2d_draw(system, this, tex, index_count, vertex_count, indices, xys, uvs, f32::from_bits(opacity), composition)).unwrap_or(0)
}

/// gfx-final: the unit-icon capture hook table, `fates_gfx_hooks_capture`
/// in overrides/gfx_capture.c (native mode, FATES_GFX_CAPTURE not 0).
pub fn install_capture(hooks: &mut capture::Hooks) {
    if mode_from_env() == Mode::Off || !capture::enabled() {
        return;
    }
    hooks.instance = Some(hook_capture_instance);
    hooks.capture = Some(hook_capture_texture);
}

unsafe extern "C" fn hook_capture_instance(ctx: *mut c_void, instance: u32, after: u32) {
    let system = unsafe { system(ctx) };
    with(|g| g.capture_instance(system, instance, after != 0));
}

unsafe extern "C" fn hook_capture_texture(ctx: *mut c_void, object: u32, _kind: u32, after: u32) {
    let system = unsafe { system(ctx) };
    with(|g| g.capture_texture(system, object, after != 0));
}

/// gfx-eft: the effect hook, with the native renderer on and FATES_GFX_EFT
/// not 0.
pub fn install_eft(hooks: &mut HooksEft) {
    if mode_from_env() == Mode::Off || std::env::var("FATES_GFX_EFT").is_ok_and(|v| v == "0") {
        return;
    }
    hooks.draw = Some(hook_eft_draw);
    hooks.fog = Some(hook_eft_fog);
}

/// gfx-eft: Pica::FogConfig(on, flip, r, g, b).
unsafe extern "C" fn hook_eft_fog(ctx: *mut c_void, on: u32, flip: u32, r: u32, g: u32, b: u32) {
    let system = unsafe { system(ctx) };
    with(|gfx| gfx.eft_fog(system, on, flip, r, g, b));
}

/// gfx-eft: MakeDrawEmitterSetCommand(system, from, set) wrote [from, to)
/// for RenderState `state`; nonzero takes the words back out.
unsafe extern "C" fn hook_eft_draw(ctx: *mut c_void, state: u32, _eft_system: u32, _set: u32, from: u32, to: u32) -> i32 {
    let system = unsafe { system(ctx) };
    let _t = perf::scope(perf::C::Eft); // gfx-perf
    with(|g| g.eft_draw(system, state, from, to)).unwrap_or(0)
}

pub fn install(hooks: &mut Hooks) {
    let mode = mode_from_env();
    if mode == Mode::Off {
        return;
    }
    let env_u = |name: &str, default: u64| std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default);
    *GFX.lock().unwrap() = Some(Gfx {
        mode,
        tables: None,
        target: 0,
        cleared: None,
        clear_taken: [false; 2], // gfxmodels3
        pending: [None, None],
        queued: HashMap::new(),
        committed: HashMap::new(),
        decoded: HashMap::new(),
        decoded_used: HashMap::new(), // gfx-2d
        freecam: freecam::FreeCam::default(), // gfxcastle
        composed: HashMap::new(),
        renderer: None,
        side: None, // perf3
        side_failed: false, // perf3
        gpu: None,
        gpu_failed: false,
        gpu_lower: None,
        gpu_queue: [VecDeque::new(), VecDeque::new()],
        gpu_shown: [None, None],
        gpu_last_hybrid: [None, None],
        meshes: HashMap::new(),
        models: models::Meshes::default(), // gfx-models
        frame3d: Vec::new(),
        oracle_seen: HashMap::new(),
        islands: None,
        traced_callbacks: Default::default(),
        frame_shaded: Vec::new(),
        shaded_programs: HashMap::new(),
        debug3d_pending: None,
        renderer_failed: false,
        chain_valid: [false; 2],
        chain: [None, None],
        next_id: 1,
        stats: Stats::default(),
        dump: std::env::var_os("FATES_GFX_DUMP").map(PathBuf::from),
        dumped: 0,
        flip_t: std::env::var("FATES_GFX_FLIPT").map_or(true, |v| v != "0"),
        report_every: env_u("FATES_GFX_REPORT", 600),
        shadow: shadow::Shadows::new(),
        alter: command_alter::Alter::default(),
        post: post::State::default(),
        post_callback: None,
        eft: eft::Stats::default(),
        eft_inherit: eft::Inherit::default(),
        sunglasses_cutin: None, // sunglasses
        unit_icon: None,
        unit_pending: VecDeque::new(),
        cache: cache::Caches::default(), // gfx-perf
        hd: hd::Hd::default(), // hdpacks
        hfr: hfr::State::default(), // gfx-hfr
        emu: emulated::Audit::new(), // gfx-final
        capture: None,
        capture_prim: (0, 0),
        captures: 0,
        capture_checks: Vec::new(),
        capture_expected: [false; 2],
        capture_seen: [false; 2],
    });
    if let Some(dir) = std::env::var_os("FATES_GFX_DUMP") {
        let _ = std::fs::create_dir_all(dir);
    }
    hooks.prim_target = Some(hook_prim_target);
    hooks.clear = Some(hook_clear);
    hooks.target_display = Some(hook_target_display);
    hooks.transfer = Some(hook_transfer);
    hooks.frame = Some(hook_frame);
    hooks.prim_target_done = Some(hook_prim_target_done);
    zakuro_core::native_gfx::install(Box::new(Screens));
    // gfx-final: the SDK's set-up before the first frame needs no emulated GPU
    if mode == Mode::Native && emulated::lease_enabled() {
        zakuro_core::native_gfx::lease_boot(120);
    }
    // gfx-perf: a headless run takes every screen only to compare or dump them
    zakuro_core::native_gfx::set_every_screen(
        mode == Mode::Shadow || std::env::var_os("FATES_GFX_DUMP").is_some() || !cache::lazy() || std::env::var("FATES_GFX_EVERY_SCREEN").is_ok_and(|v| v == "1"),
    );
    eprintln!("gfx: native renderer {mode:?}");
}

/// # Safety
/// ctx is the running context an override was entered with.
pub(crate) unsafe fn system<'a>(ctx: *mut c_void) -> &'a mut System {
    unsafe { &mut *zakuro_core::recompiled::gfx_system_of(ctx) }
}

unsafe extern "C" fn hook_frame(ctx: *mut c_void) {
    let system = unsafe { system(ctx) };
    with(|g| {
        // gfx-shadow: the last frame's shadow maps against the emulated ones
        if std::env::var("FATES_GFX_SHADOW_COMPARE").is_ok_and(|v| v != "transfer") {
            g.shadow_compare(system);
        }
        // gfx-perf: the renderer goes on the emulated GPU's device: one GPU
        // context, so waiting for a frame does not wait for the driver to
        // switch between two
        if !g.cache.device_asked {
            g.cache.device_asked = true;
            if !std::env::var("FATES_GFX_OWN_DEVICE").is_ok_and(|v| v == "1") {
                g.cache.device = system.gpu.hardware_device();
            }
        }
        // gfx-eft: dev check (FATES_GFX_EFT_CHECK)
        g.eft_check(system, "next-frame", false);
        g.stats.frames += 1;
        g.hd.frame(g.stats.frames); // hdpacks
        zakuro_core::moflexhd::frame(g.stats.frames); // selena: a movie picture's place stays one while pictures go there
        g.emu.frame(g.stats.frames, g.mode == Mode::Native); // gfx-final
        g.capture_check(system); // gfx-final: dev
        bottom::peek(system, g.stats.frames); // gfx-bottom: dev
        zakuro_core::native_gfx::skip_frame(); // gfx-perf
        // gfx-perf: FATES_GFX_PERF=N
        if perf::every() > 0 && g.stats.frames % perf::every() == 0 {
            perf::report(g.stats.frames);
        }
        g.shadow.frame(); // gfx-shadow
        if g.report_every > 0 && g.stats.frames % g.report_every == 0 {
            eprintln!("gfx: {:?}", g.stats);
            // gfx-shadow
            let (s, a) = (&g.shadow, &g.alter);
            eprintln!(
                "gfx-shadow: passes {} (native only {}), caster meshes {}, maps compared {} (mean {:.3}, worst {:.3}); alterations {} {:?} over {} frames, right-eye copies {}",
                s.passes, s.replaced, s.casters, s.compared, s.diff_sum / s.compared.max(1) as f64, s.diff_worst, a.recorded, a.by_kind, a.frames, a.executed
            );
            eprintln!("gfx-eft: {:?}", g.eft);
            eprintln!("{}", lines::summary()); // gfx-lines
            // gfx-hfr
            let h = &g.hfr;
            eprintln!("gfx-hfr: frames matched {} (smooth {}), pictures in between drawn {}, drawn as the frame (no picture before) {}, with post effects reading earlier frames {}, put into the buffer shown {}, lower pictures older than the upper's shown as they are {}, before the frame before {}, held for a model gone {}", h.tracked, h.smooth, h.drawn_between, h.fallbacks, h.carried, h.into_shown, h.stale_lower, h.before_previous, h.vanished); // gfx-models: carried, into_shown, stale_lower, before_previous; gfxleft: vanished
            // sharp: while pictures are drawn smaller, the upper pictures shown at full scale
            eprintln!("sharp: upper pictures while drawn smaller {} at the smaller scale, {} at full scale; frames drawn at it {}", h.shown_scaled, h.shown_full, h.reduced_frames);
            eprintln!("{}", g.emu.report()); // gfx-final
            eprintln!("{}", g.models.line()); // gfx-models
        }
    });
}

unsafe extern "C" fn hook_target_display(_ctx: *mut c_void, display: u32) {
    debug(format_args!("target {display:X}"));
    with(|g| {
        g.target = display;
        g.cleared = None;
        g.clear_taken = [false; 2]; // gfxmodels3
    });
}

/// FrameBuffer::Clear: the colour and depth the game clears the display's
/// framebuffer to, from its nn::gr::CTR::FrameBuffer (pointer in the literal
/// pool at 0x00106564): clear colour floats at +0x10, depth at +0x34.
unsafe extern "C" fn hook_clear(ctx: *mut c_void) {
    let system = unsafe { system(ctx) };
    let fb = prim::r32(system, zakuro_core::region::us(0x0010_6564));
    let mut color = [0f32; 4];
    for (i, c) in color.iter_mut().enumerate() {
        *c = f32::from_bits(prim::r32(system, fb + 0x10 + 4 * i as u32)).clamp(0.0, 1.0);
    }
    let depth = f32::from_bits(prim::r32(system, fb + 0x34)).clamp(0.0, 1.0);
    with(|g| {
        g.cleared = Some((color, depth));
        g.clear_taken = [false; 2]; // gfxmodels3
    });
    if crate::perf::var_os("FATES_GFX_FRAME_LOG").is_some() {
        eprintln!("gfx-frame game {} clear {color:?}", zakuro_core::native_gfx::HEADLESS_FRAME.load(std::sync::atomic::Ordering::Relaxed)); // gfxmodels3: dev
    }
}

static DEBUG_CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
fn debug(what: std::fmt::Arguments) {
    if std::env::var_os("FATES_GFX_DEBUG").is_some() && DEBUG_CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) < 200 {
        eprintln!("gfx-debug {what}");
    }
}

unsafe extern "C" fn hook_prim_target(ctx: *mut c_void, prim_ctx: u32, disp: u32) -> i32 {
    let system = unsafe { system(ctx) };
    debug(format_args!("prim_target ctx {prim_ctx:08X} disp {disp}"));
    with(|g| {
        let start = std::time::Instant::now();
        let _t = perf::scope(perf::C::Record); // gfx-perf
        let before = (g.cleared.is_some(), disp < 2 && g.chain_valid[disp as usize & 1]); // gfx-bottom
        g.capture_prim = (prim_ctx, disp); // gfx-final
        let keep = g.prim_target(system, prim_ctx, disp);
        // gfx-final: a frame the emulated GPU draws all or part of
        if g.mode == Mode::Native && disp < 2 {
            match g.pending[disp as usize].as_ref() {
                None => g.emu.need("emulated frame"),
                Some(f) if f.hybrid => g.emu.need("hybrid frame"),
                _ => {}
            }
        }
        if bottom::traced(g.stats.frames) && disp < 2 {
            // gfx-bottom: dev, FATES_GFX_BOTTOM_TRACE=from-to
            let p = g.pending[disp as usize].as_ref().map(|f| (f.id, f.hybrid, f.clear.is_some(), f.draws.len()));
            eprintln!("gfx-bottom frame {} disp {disp}: cleared {} chain {} -> keep {keep} chain {} pending {p:?} states {}", g.stats.frames, before.0, before.1, g.chain_valid[disp as usize], prim::state_list(system, prim_ctx, disp).len());
        }
        g.stats.us_record += start.elapsed().as_micros() as u64;
        keep
    })
    .unwrap_or(-1)
}

/// after the original RenderTarget: the state list put back, the native
/// 3D islands' draws into their places in the frame.
unsafe extern "C" fn hook_prim_target_done(ctx: *mut c_void, _prim_ctx: u32, disp: u32) {
    let system = unsafe { system(ctx) };
    with(|g| g.prim_target_done(system, disp));
}

/// gfx-gaps: what Model::DrawMesh does to the game's own state besides
/// building commands, for a call the native path skips: ModelState::
/// BeginCommand (0x005071F8) calls Graphics::SetClearType(top, 2)
/// (0x004EBB64, one byte at `*(lit 0x004EBB70) + display`) for a model
/// with flags 1 and 0x800 (+8). Skipped, the next frame was cleared as if
/// no such model had drawn.
fn skipped_draw_mesh(system: &mut System, model: u32) {
    let flags = prim::r16(system, model + 8);
    if flags & 1 != 0 && flags & 0x800 != 0 {
        let types = prim::r32(system, zakuro_core::region::us(0x004E_BB70));
        system.memory.write_bytes(types, &[2]);
    }
}

/// gfx-gaps: dev, FATES_GFX_MEMDUMP (see post_end)
static MEMDUMP_PENDING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

unsafe extern "C" fn hook_transfer(ctx: *mut c_void, displaybuffer: u32) {
    let system = unsafe { system(ctx) };
    if MEMDUMP_PENDING.swap(false, std::sync::atomic::Ordering::Relaxed) {
        if let Ok(spec) = std::env::var("FATES_GFX_MEMDUMP") {
            for part in spec.split(';') {
                let mut it = part.split(',').filter_map(|v| u32::from_str_radix(v.trim().trim_start_matches("0x"), 16).ok());
                if let (Some(addr), Some(len)) = (it.next(), it.next()) {
                    system.sync_gpu(addr, len);
                    let mut bytes = vec![0u8; len as usize];
                    system.memory.read_bytes(addr, &mut bytes);
                    let _ = std::fs::write(format!("mem-{addr:08X}.bin"), &bytes);
                }
            }
        }
    }
    with(|g| g.eft_check(system, "transfer", true)); // gfx-eft: dev check
    let current = prim::r32(system, displaybuffer + 0x2C);
    let address = prim::r32(system, displaybuffer + current * 4 + 0x10);
    debug(format_args!("transfer db {displaybuffer:08X} -> {address:08X}"));
    if crate::perf::var_os("FATES_GFX_FRAME_LOG").is_some() {
        // gfxmodels3: dev
        with(|g| eprintln!("gfx-frame game {} transfer target {:X} -> {address:08X} pending {:?}", zakuro_core::native_gfx::HEADLESS_FRAME.load(std::sync::atomic::Ordering::Relaxed), g.target, g.pending.iter().map(|p| p.as_ref().map(|f| (f.id, f.draws.len()))).collect::<Vec<_>>()));
    }
    with(|g| {
        let index = match g.target {
            0x400 => 0,
            0x410 => {
                // the right eye: stereo is off in this port. gfx-final: its
                // picture is never shown, so the emulated GPU leaves its
                // transfer out too (in native mode)
                if g.mode == Mode::Native && emulated::lease_enabled() {
                    let physical = zakuro_core::services::gsp::virtual_to_physical(system, address);
                    cache::native_display(physical);
                }
                return;
            }
            _ => 1,
        };
        if index == 0 {
            g.debug3d_frame(system);
        }
        g.splice_islands(system);
        // gfx-perf: a picture drawn natively in full needs no emulated
        // display transfer (FATES_GFX_SKIP_COPIES=0 keeps it). gfx-hfr: a
        // presenter showing the screens from the GPU finds them by the LCD's
        // configuration now (native_gfx::gpu_screen), not by the emulated
        // picture the transfer leaves (FATES_GFX_WINDOW_TRANSFERS=1 keeps
        // them there)
        let keep = system.gpu.shows_directly() && perf::var("FATES_GFX_WINDOW_TRANSFERS").is_ok_and(|v| v == "1");
        if g.mode == Mode::Native && !keep && g.pending[index].as_ref().is_some_and(|f| !f.hybrid) {
            let physical = zakuro_core::services::gsp::virtual_to_physical(system, address);
            cache::native_display(physical);
        }
        // gfx-final: a UnitIconInstance island whose OnBegin did not come
        // through the hook (a tail call): its icons are the emulated GPU's
        if std::mem::take(&mut g.capture_expected[index]) && !std::mem::take(&mut g.capture_seen[index]) {
            g.emu.need("unit-icon capture outside the hook");
        }
        g.capture_seen[index] = false;
        // gfx-final: the emulated GPU is not needed for a picture drawn
        // natively in full; one the native path has no frame for (and that
        // is not a repeat of this gfx frame's) needs it
        if g.mode == Mode::Native {
            match g.pending[index].as_ref() {
                Some(f) if !f.hybrid => g.emu.native_transfer(),
                Some(_) => {}
                None => {
                    let repeat = g.queued.get(&address).and_then(|q| q.back()).is_some_and(|(at, _)| *at == g.stats.frames);
                    if !repeat {
                        g.emu.need("picture without a native frame");
                    }
                }
            }
        }
        if bottom::traced(g.stats.frames) {
            // gfx-bottom: dev
            eprintln!("gfx-bottom transfer db {displaybuffer:08X} target {:X} -> {address:08X} pending {:?}", g.target, g.pending[index].as_ref().map(|f| f.id));
        }
        // None: the emulated path alone draws this picture
        let frame = g.pending[index].take();
        let queue = g.queued.entry(address).or_default();
        bottom::queue(queue, g.stats.frames, frame, &mut g.stats.bottom); // gfx-bottom
        while queue.len() > 4 {
            queue.pop_front();
        }
    });
}

impl Gfx {
    /// gfx-final: UnitIconInstance::OnBegin starts (recording its calls)
    /// or ends
    fn capture_instance(&mut self, system: &mut System, _instance: u32, after: bool) {
        if after {
            self.capture_end_instance(system, "");
            return;
        }
        self.capture_end_instance(system, "next OnBegin"); // gfx-units
        if self.unit_icon.is_some() || self.post_callback.is_some() {
            return;
        }
        let (prim_ctx, disp) = self.capture_prim;
        self.capture_seen[(disp & 1) as usize] = true; // gfx-final
        let tables = *self.tables.get_or_insert_with(|| Tables::read(system));
        // the world matrix UpdateState left for the callback's state
        let state = if prim_ctx != 0 { prim::r32(system, prim_ctx + 0x1C) } else { 0 };
        let world = if state != 0 { prim::state_setup(system, &tables, prim_ctx, disp.min(1), state).world } else { [1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0.] };
        self.capture = Some(capture::Recorder::new(system, prim_ctx, world));
        post::set_recording(true);
    }

    /// gfx-final: a UnitIconObject::CaptureTexture starts, or ends: its
    /// draws rendered into the icon's texture natively, the texels
    /// written to guest memory
    fn capture_texture(&mut self, system: &mut System, object: u32, after: bool) {
        if self.capture.is_none() {
            return;
        }
        // gfx-units: a capture whose end hook did not run is done when the
        // next one starts
        self.capture_finish(system, if after { "" } else { "next CaptureTexture" });
        if !after {
            if let Some(rec) = self.capture.as_mut() {
                rec.begin_capture(object);
            }
        }
    }

    /// gfx-units: the UnitIconInstance::OnBegin recording ends, its last
    /// capture finished first. The hooks after an original (overrides/
    /// gfx_capture.c, gfx_unit_icon.c) do not run when the original leaves
    /// the override at an svc or budget exit (recomp.h `CALL` returns at
    /// once; the guest goes on from r15 later), so this is also reached
    /// from the next event that shows the recording is over. `why` names
    /// that event (empty: the end hook itself).
    fn capture_end_instance(&mut self, system: &mut System, why: &str) {
        if self.capture.is_none() {
            return;
        }
        self.capture_finish(system, why);
        if !why.is_empty() && crate::perf::var_os("FATES_GFX_CAPTURE_TRACE").is_some() {
            eprintln!("gfx-capture frame {}: OnBegin ended at {why}", self.stats.frames);
        }
        self.capture = None;
        post::set_recording(false);
    }

    /// gfx-final: a UnitIconObject::CaptureTexture done: its draws rendered
    /// into the icon's texture natively, the texels written to guest memory
    /// (gfx-units: split out of capture_texture, so a capture whose end hook
    /// was lost is finished too)
    fn capture_finish(&mut self, system: &mut System, why: &str) {
        let Some(rec) = self.capture.as_mut() else { return };
        let Some(cap) = rec.end_capture() else { return };
        if !why.is_empty() && crate::perf::var_os("FATES_GFX_CAPTURE_TRACE").is_some() {
            eprintln!("gfx-capture frame {}: object {:08X} finished at {why}", self.stats.frames, cap.object);
        }
        let object = cap.object;
        if !rec.unsupported.is_empty() {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("gfx-capture: not drawn natively in full: {:?}", rec.unsupported);
            }
        }
        let Some(t) = cap.target.filter(|t| t.width > 0 && t.height > 0 && t.width <= 1024 && t.height <= 1024) else { return };
        let mut keys = Vec::with_capacity(cap.draws.len());
        for d in &cap.draws {
            let mut k = [None; 3];
            if let Some((tex, _)) = d.textures[0] {
                k[0] = self.texture(system, tex);
            }
            keys.push(k);
        }
        self.ensure_readback(); // perf3
        let Some(renderer) = (if self.side.is_some() { self.side.as_mut() } else { self.renderer.as_mut() }) else { return };
        let (uw, uh) = (t.height, t.width);
        let upright = match renderer.render(2, uw, uh, 1.0, Start::Clear([0.0; 4]), 1.0, &cap.draws, &keys, &self.decoded, Finish::Readback) {
            Ok(Output::Pixels(p)) => p,
            Ok(_) => return,
            Err(e) => {
                eprintln!("gfx-capture: render failed, {e}");
                return;
            }
        };
        let Some(bytes) = capture::encode(&upright, &t) else { return };
        self.captures += 1;
        if crate::perf::var_os("FATES_GFX_CAPTURE_TRACE").is_some() {
            eprintln!("gfx-capture frame {}: object {object:08X} target {t:X?} draws {} sheets {:?}", self.stats.frames, cap.draws.len(), cap.draws.iter().filter_map(|d| d.textures[0].map(|(t, s)| (t.width, t.height, t.format, s.linear_mag))).collect::<std::collections::BTreeSet<_>>());
        }
        if crate::perf::var_os("FATES_GFX_CAPTURE_CHECK").is_some() {
            self.capture_checks.push((t, bytes.clone()));
        }
        if self.mode == Mode::Native {
            system.memory.write_bytes(t.addr, &bytes);
        }
    }

    /// gfx-final: dev, FATES_GFX_CAPTURE_CHECK: each native icon against
    /// the one the emulated GPU drew (FATES_GFX_EMULATED=keep), once its
    /// list ran
    fn capture_check(&mut self, system: &mut System) {
        for (t, native) in std::mem::take(&mut self.capture_checks) {
            system.sync_gpu(t.addr, native.len() as u32);
            let mut emulated = vec![0u8; native.len()];
            system.memory.read_bytes(t.addr, &mut emulated);
            let px = |b: &[u8], i: usize| u16::from_le_bytes([b[2 * i], b[2 * i + 1]]);
            let n = native.len() / 2;
            let (mut differ, mut worst) = (0usize, 0i32);
            for i in 0..n {
                let (a, b) = (px(&native, i), px(&emulated, i));
                if a != b {
                    differ += 1;
                    for k in 0..4 {
                        worst = worst.max((((a >> (4 * k)) & 0xF) as i32 - ((b >> (4 * k)) & 0xF) as i32).abs());
                    }
                }
            }
            eprintln!("gfx-capture check {:08X} {}x{}: {differ} of {n} texels differ (worst {worst} of 15)", t.addr, t.width, t.height);
            if let Some(dir) = crate::perf::var_os("FATES_GFX_CAPTURE_DUMP") {
                let _ = std::fs::create_dir_all(&dir);
                let _ = std::fs::write(std::path::Path::new(&dir).join(format!("native-{:08X}.bin", t.addr)), &native);
                let _ = std::fs::write(std::path::Path::new(&dir).join(format!("emulated-{:08X}.bin", t.addr)), &emulated);
            }
        }
    }

    /// gfx-gaps: UnitIcon::BatchCallback::OnBegin starts: recorded when
    /// it is the callback state scene2d_end met next (the prim context's
    /// current state is that state).
    fn unit_icon_begin(&mut self, system: &mut System, _callback: u32) {
        let traced = unit_icon::trace_window(self.stats.frames); // gfx-units: dev
        // gfx-units: a unit-icon capture recording still open (its end
        // hook lost) would take this batch's calls
        self.capture_end_instance(system, "BatchCallback");
        // gfx-units: so would a batch whose end hook was lost
        if self.unit_icon.is_some() {
            self.unit_icon_end(system, 0);
        }
        let Some(slot) = self.unit_pending.pop_front() else {
            unit_log(self.stats.frames, "no slot", 0); // gfxmap
            if traced {
                eprintln!("gfx-uniticon frame {}: batch with no noted slot (islands {})", self.stats.frames, self.islands.is_some());
            }
            return;
        };
        if prim::r32(system, slot.prim_ctx + 0x1C) != slot.state || self.islands.is_none() || self.post_callback.is_some() {
            unit_log(self.stats.frames, "not recorded", 0); // gfxmap
            if traced {
                eprintln!("gfx-uniticon frame {}: batch not recorded (state {:08X} vs {:08X}, islands {}, post {})", self.stats.frames, prim::r32(system, slot.prim_ctx + 0x1C), slot.state, self.islands.is_some(), self.post_callback.is_some());
            }
            self.unit_pending.clear();
            return;
        }
        let tables = *self.tables.get_or_insert_with(|| Tables::read(system));
        unit_icon::TRACING.store(unit_icon::trace_batch(self.stats.frames), std::sync::atomic::Ordering::Relaxed); // gfx-units: dev
        let rec = unit_icon::Recorder::new(system, tables, slot.prim_ctx, slot.disp, slot.state);
        self.unit_icon = Some((slot, rec));
        post::set_recording(true);
    }

    /// gfx-gaps: the callback returned: its draws into the scene island's
    /// draws at its state's place.
    fn unit_icon_end(&mut self, _system: &mut System, _callback: u32) {
        let Some((slot, mut rec)) = self.unit_icon.take() else { return };
        if _callback == 0 && crate::perf::var_os("FATES_GFX_UNITICON_TRACE_LOST").is_some() {
            eprintln!("gfx-uniticon frame {}: batch ended without its end hook", self.stats.frames); // gfx-units: dev
        }
        post::set_recording(false);
        rec.finish(_system);
        if !rec.unsupported.is_empty() {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("gfx-uniticon: not drawn natively in full: {:?}", rec.unsupported);
            }
        }
        if unit_icon::TRACING.swap(false, std::sync::atomic::Ordering::Relaxed) {
            // gfx-units: dev, the batch's gfx frame
            eprintln!("gfx-uniticon frame {}", self.stats.frames);
            eprintln!("gfx-uniticon scene {:08X} at {} state {:08X}: {} draws", slot.scene, slot.at, slot.state, rec.draws.len());
            for d in rec.draws.iter().take(16) {
                let p: [f32; 3] = std::array::from_fn(|k| f32::from_le_bytes(d.vertices[k * 4..k * 4 + 4].try_into().unwrap()));
                let w: [f32; 3] = std::array::from_fn(|r| d.world[r * 4] * p[0] + d.world[r * 4 + 1] * p[1] + d.world[r * 4 + 2] * p[2] + d.world[r * 4 + 3]);
                let c: [f32; 4] = std::array::from_fn(|r| d.view_proj[r * 4] * w[0] + d.view_proj[r * 4 + 1] * w[1] + d.view_proj[r * 4 + 2] * w[2] + d.view_proj[r * 4 + 3]);
                eprintln!("gfx-uniticon   v0 {p:?} world {w:?} clip {c:?} ndc {:?} rgba {:?} vp {:?}", [c[0] / c[3], c[1] / c[3], c[2] / c[3]], &d.vertices[20..24], d.view_proj);
            }
            if let Some(d) = rec.draws.first() {
                // gfx-units: dev, everything but the vertices
                let mut d = d.clone();
                d.vertices.clear();
                eprintln!("gfx-uniticon   first {d:?}");
            }
            for d in &rec.draws {
                let v: Vec<[f32; 5]> = d.vertices.chunks_exact(24).take(4).map(|c| std::array::from_fn(|k| f32::from_le_bytes(c[k * 4..k * 4 + 4].try_into().unwrap()))).collect();
                eprintln!(
                    "gfx-uniticon   tex {:X?} depth {} {} {} alpha {} {} {} st2 {:?} st5 {:?} world {:?} n {} v {:?}",
                    d.textures[0].map(|t| (t.0.addr, t.0.width, t.0.height, t.0.format)),
                    d.depth_test,
                    d.depth_func,
                    d.depth_write,
                    d.alpha_test,
                    d.alpha_func,
                    d.alpha_ref,
                    d.combiner.stages[2],
                    d.combiner.stages[5],
                    d.world,
                    d.indices.len(),
                    v
                );
            }
        }
        let n = rec.draws.len();
        unit_log(self.stats.frames, if self.islands.is_some() { "drawn" } else { "no islands" }, n); // gfxmap
        let Some(islands) = self.islands.as_mut() else { return };
        let list = islands.draws.entry(slot.scene).or_default();
        let at = slot.at.min(list.len());
        list.splice(at..at, rec.draws.into_iter().map(|d| (d, [None; 3])));
        for p in self.unit_pending.iter_mut().filter(|p| p.scene == slot.scene && p.at >= slot.at) {
            p.at += n;
        }
    }

    /// gfx-post: a PrimPostEffectCallback starts. When it is an island of
    /// the frame the native 3D path is drawing, what it does is recorded.
    fn post_begin(&mut self, system: &mut System, callback: u32) {
        let Some(islands) = self.islands.as_ref() else { return };
        if !islands.slots.iter().any(|(o, _)| *o == callback) || !post::supported(system, callback) {
            return;
        }
        // gfx-perf: in a frame drawn natively in full, the emulated GPU
        // leaves out the effect's copies and fills
        cache::arm_copies(islands.compacted && self.mode == Mode::Native);
        self.post.begin(system, islands.disp as u8);
        self.post_callback = Some(callback);
    }

    /// gfx-post: the callback is done: its recording goes to its island,
    /// with the guest textures it reads decoded.
    fn post_end(&mut self, system: &mut System, callback: u32) {
        if self.post_callback != Some(callback) {
            return;
        }
        self.post_callback = None;
        cache::arm_copies(false); // gfx-perf
        let Some(mut block) = self.post.end() else { return };
        for op in block.ops.iter_mut() {
            if let post::Op::Draw(d) = op {
                for u in d.units.iter_mut() {
                    if let post::Unit::Guest { tex, key, .. } = u {
                        *key = self.texture_oriented(system, *tex, true);
                    }
                }
            }
        }
        if !block.unsupported.is_empty() {
            static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
                eprintln!("gfx-post: not drawn natively in full: {:?}", block.unsupported);
            }
        }
        // gfx-gaps: dev, FATES_GFX_POST_OPS=N: the ops of one block from
        // gfx frame N on
        static OPS_TRACED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if std::env::var("FATES_GFX_POST_OPS").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| self.stats.frames >= n) && !OPS_TRACED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            for (i, op) in block.ops.iter().enumerate() {
                match op {
                    post::Op::Draw(d) => {
                        let v: Vec<[f32; 4]> = d.vertices.chunks_exact(40).take(4).map(|c| std::array::from_fn(|k| f32::from_le_bytes(c[[0, 4, 12, 16][k]..[0, 4, 12, 16][k] + 4].try_into().unwrap()))).collect();
                        eprintln!("gfx-post-op {i}: draw target {:?} vp {:?} sc {:?} units {:?} n {} v(x,y,u,v) {:?}", d.target, d.viewport, d.scissor, d.units, d.indices.len(), v);
                    }
                    other => eprintln!("gfx-post-op {i}: {other:?}"),
                }
            }
        }
        // gfx-gaps: dev, FATES_GFX_MEMDUMP=virt,len (with
        // FATES_GFX_POST_SURFDUMP_AT=n): guest memory at the transfer after
        // the nth post block, to compare the emulated GPU's surfaces with
        // the native ones FATES_GFX_POST_SURFDUMP writes
        if std::env::var_os("FATES_GFX_MEMDUMP").is_some() {
            let at = std::env::var("FATES_GFX_POST_SURFDUMP_AT").ok().and_then(|v| v.parse().ok()).unwrap_or(30u64);
            if self.post.blocks == at + 1 {
                MEMDUMP_PENDING.store(true, std::sync::atomic::Ordering::Relaxed);
            }
        }
        if let Some(islands) = self.islands.as_mut() {
            islands.posts.insert(callback, Arc::new(block));
        }
    }

    fn draw_mesh(&mut self, system: &mut System, model: u32, state: u32, layer: u32, mesh: u32, world: u32) -> i32 {
        self.stats.meshes += 1;
        // gfx-models: dev, FATES_GFX_SLOW_MESH_US=n spins n microseconds per
        // DrawMesh: a game thread too slow for its refreshes, as on a busy
        // machine, for checking what the frames look like then
        if let Some(us) = crate::perf::var("FATES_GFX_SLOW_MESH_US").ok().and_then(|v| v.parse::<u64>().ok()) {
            let until = std::time::Instant::now() + std::time::Duration::from_micros(us);
            while std::time::Instant::now() < until {
                std::hint::spin_loop();
            }
        }
        // FATES_GFX_ORACLE=<buffer>|all with FATES_GFX_ORACLE_DIR=dir: the
        // native setup of the sub-mesh draws zakuro-gpu dumps the state of
        // (see there), for dev/gfx/oracle.py
        if let (Ok(want), Some(dir)) = (crate::perf::var("FATES_GFX_ORACLE"), crate::perf::var_os("FATES_GFX_ORACLE_DIR")) { // gfx-perf: cached env
            let translated = match self.meshes.get(&mesh) {
                Some(m) => Some(m.clone()),
                None => h3d::mesh(system, mesh).map(Arc::new).inspect(|m| {
                    self.meshes.insert(mesh, m.clone());
                }),
            };
            let env = |name: &str, default: u32| crate::perf::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default); // gfx-perf: cached env
            let (skip, per) = (env("FATES_GFX_ORACLE_SKIP", 0), env("FATES_GFX_ORACLE_PER", 2));
            if let Some(m) = translated.filter(|m| want == "all" || u32::from_str_radix(&want, 16).is_ok_and(|w| w == m.buffer_phys)) {
                let o = setup::Objects::read(system, model, state, layer, mesh, world);
                let mut base = None;
                for sub in 0..m.submeshes.len() as u32 {
                    let count = self.oracle_seen.entry(m.buffer_phys).or_insert(0);
                    let n = *count;
                    *count += 1;
                    if n < skip || n >= skip + per {
                        continue;
                    }
                    let base = base.get_or_insert_with(|| setup::mesh_setup(system, &o));
                    let s = setup::submesh_setup(system, &o, base, sub);
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::fs::write(std::path::Path::new(&dir).join(format!("native-{:08X}-{}.bin", m.buffer_phys, n - skip)), s.dump());
                    let _ = std::fs::write(std::path::Path::new(&dir).join(format!("objects-{:08X}-{}.txt", m.buffer_phys, n - skip)), format!("{o:X?}"));
                }
            }
        }
        // gfx-final: dev, FATES_GFX_TRACE_BUFFER=hex: a mesh's vertex
        // layout as the native path reads it, once
        if let Ok(want) = crate::perf::var("FATES_GFX_TRACE_BUFFER") {
            static DONE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if let Some(m) = h3d::mesh(system, mesh).filter(|m| u32::from_str_radix(&want, 16).is_ok_and(|w| w == m.buffer_phys)) {
                if !DONE.swap(true, std::sync::atomic::Ordering::Relaxed) {
                    eprintln!("gfx-trace-buffer {:08X}: stride {} attrs {:?} fixed {:?} position offset {:?}", m.buffer_phys, m.stride, m.attrs, m.fixed, m.position_offset);
                    let v: Vec<String> = m.vertices.chunks(m.stride.max(1) as usize).take(6).map(|c| c.iter().map(|b| format!("{b:02X}")).collect::<String>()).collect();
                    eprintln!("gfx-trace-buffer   first vertices {v:?}");
                }
            }
        }
        // gfx-shadow: a caster of the running shadow pass
        let t_caster = perf::scope(perf::C::Caster); // perf3
        let caster = self.shadow_caster(system, model, state, layer, mesh, world);
        drop(t_caster);
        if let Some(skip) = caster {
            if skip != 0 {
                skipped_draw_mesh(system, model); // gfx-gaps
            }
            return skip;
        }
        if let Some(scene) = self.islands.as_ref().map(|_| prim::r32(system, state + 0xC)) {
            if self.islands.as_ref().is_some_and(|i| i.slots.iter().any(|(o, _)| *o == scene)) {
                let disp = self.islands.as_ref().unwrap().disp;
                #[allow(unused_mut)]
                let mut made = self.shade_draws(system, model, state, layer, mesh, world, disp, None);
        let t_haze = perf::scope(perf::C::Haze); // perf3
                skybg::trace(system, model, state, mesh, &made); // skybg: dev, FATES_SKYBG_TRACE
                self.freecam_haze(system, model, state, mesh, &mut made); // gfxcastle
                drop(t_haze);
                let t_wide2 = perf::scope(perf::C::Wide2); // perf3
                if disp == 0 {
                    gfx3::trace_made(&mut made, zakuro_gpu::wide::factor()); // gfx3: dev
                    gfx3::skill_wash(&mut made, zakuro_gpu::wide::factor()); // gfx3: no skill vignette at wide aspects
                    wide2::extend(&mut made, zakuro_gpu::wide::factor()); // wide2: full-width telop layers
                    wide2::soften(&mut self.decoded, &mut made); // wide2
                }
                drop(t_wide2);
                let islands = self.islands.as_mut().unwrap();
                islands.draws.entry(scene).or_default().extend(made);
                if islands.compacted {
                    skipped_draw_mesh(system, model); // gfx-gaps
                }
                return islands.compacted as i32;
            }
        }
        // gfx-shadow: drawn by the emulated GPU: is it a shadow receiver?
        if self.mode == Mode::Native {
            self.shadow_note_emulated(system, model, mesh);
        }
        // gfx-eft: outside native islands no mesh setup is built; with the
        // oracle on, record the mesh's fog for the effects after it anyway,
        // so their state can be checked in hybrid frames too
        if crate::perf::var_os("FATES_GFX_ORACLE").is_some() { // gfx-perf: cached env
            let o = setup::Objects::read(system, model, state, layer, mesh, world);
            let base = setup::mesh_setup(system, &o);
            self.eft_inherit.mesh(&base);
        }
        if crate::perf::var_os("FATES_GFX_SCENE3D").is_some() { // gfx-perf: cached env
            self.shade_mesh(system, model, state, layer, mesh, world);
        }
        if crate::perf::var_os("FATES_GFX_DEBUG3D").is_some() { // gfx-perf: cached env
            let translated = match self.meshes.get(&mesh) {
                Some(m) => Some(m.clone()),
                None => h3d::mesh(system, mesh).map(Arc::new).inspect(|m| {
                    self.meshes.insert(mesh, m.clone());
                }),
            };
            if let Some(m) = translated {
                let md = debug3d::read(system, model, state, m, world);
                self.frame3d.push(md);
            }
        }
        // FATES_GFX_TRACE3D=N: the meshes of the first frame from game frame
        // N on that draws any
        static TRACE_FRAME: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let wanted = crate::perf::var("FATES_GFX_TRACE3D").ok().and_then(|v| v.parse::<u64>().ok()); // gfx-perf: cached env
        let Some(wanted) = wanted else { return 0 };
        if self.stats.frames < wanted {
            return 0;
        }
        let traced = TRACE_FRAME.load(std::sync::atomic::Ordering::Relaxed);
        if traced == 0 {
            TRACE_FRAME.store(self.stats.frames, std::sync::atomic::Ordering::Relaxed);
        } else if traced != self.stats.frames {
            return 0;
        }
        let Some(m) = h3d::mesh(system, mesh) else {
            eprintln!("gfx-3d mesh {mesh:08X}: unreadable");
            return 0;
        };
        let render = prim::r32(system, state + 8);
        let mut view = [0f32; 12];
        for (i, v) in view.iter_mut().enumerate() {
            *v = f32::from_bits(prim::r32(system, render + 0x98 + 4 * i as u32));
        }
        let mut w = [0f32; 12];
        for (i, v) in w.iter_mut().enumerate() {
            *v = f32::from_bits(prim::r32(system, world + 4 * i as u32));
        }
        eprintln!(
            "gfx-3d model {model:08X} layer {layer} mesh {mesh:08X} mat {} flags {:X} node {} stride {} buffer {:08X} verts {} attrs {:?} fixed {:?} posofs {:?} subs {}",
            m.material,
            m.flags,
            m.node,
            m.stride,
            m.buffer_phys,
            m.vertices.len() / m.stride.max(1) as usize,
            m.attrs,
            m.fixed,
            m.position_offset,
            m.submeshes.len()
        );
        for (i, s) in m.submeshes.iter().enumerate() {
            eprintln!(
                "gfx-3d   sub {i}: skin {} bones {:?} indices {} prim {} bools {:04X} first {:?}",
                s.skinning,
                s.bones,
                s.indices.len(),
                s.primitive,
                s.bool_uniforms,
                &s.indices[..s.indices.len().min(6)]
            );
        }
        if let Some(a) = m.attrs.iter().find(|a| a.name == h3d::AttrName::Position) {
            let v: Vec<[f32; 3]> = (0..3usize)
                .map(|k| {
                    let o = k * m.stride as usize + a.offset as usize;
                    std::array::from_fn(|c| match a.format {
                        3 => f32::from_le_bytes(m.vertices[o + 4 * c..o + 4 * c + 4].try_into().unwrap_or([0; 4])),
                        2 => i16::from_le_bytes([m.vertices[o + 2 * c], m.vertices[o + 2 * c + 1]]) as f32 * a.scale,
                        1 => m.vertices[o + c] as f32 * a.scale,
                        _ => m.vertices[o + c] as i8 as f32 * a.scale,
                    })
                })
                .collect();
            eprintln!("gfx-3d   positions {v:?}");
        }
        eprintln!("gfx-3d   world {w:?} view {view:?}");
        0
    }

    fn prim_target(&mut self, system: &mut System, prim_ctx: u32, disp: u32) -> i32 {
        if disp > 1 {
            return -1;
        }
        shadow::test_drop_post(system, prim_ctx, disp); // gfx-shadow: test aid
        let index = disp as usize;
        let tables = *self.tables.get_or_insert_with(|| Tables::read(system));
        let mut clear = self.cleared;
        // gfxmodels3: the game can run RenderTarget for a display again
        // before the display's transfer, without clearing again (the
        // opening movie does at the movie's own rate: three calls in one
        // refresh, the later two with no states). The framebuffer keeps
        // what the first drew, and so does the console's picture; here
        // each call made a new frame from the clear the first one took,
        // with nothing drawn: a black picture shown for that refresh, "the
        // cutscene flickering black every few seconds". A call with no
        // states over a frame not yet shown leaves that frame as it is.
        // FATES_GFX_EMPTY_REDRAW=0 for the old way.
        if self.clear_taken[index] && self.pending[index].is_some() && prim::state_list(system, prim_ctx, disp).is_empty() && perf::var("FATES_GFX_EMPTY_REDRAW").map_or(true, |v| v != "0") {
            self.stats.empty_redraws += 1;
            if crate::perf::var_os("FATES_GFX_FRAME_LOG").is_some() {
                eprintln!("gfx-frame game {} disp {disp} empty redraw kept the frame before it", zakuro_core::native_gfx::HEADLESS_FRAME.load(std::sync::atomic::Ordering::Relaxed));
            }
            return match self.mode {
                Mode::Native => 0,
                _ => -1,
            };
        }
        if clear.is_some() {
            self.clear_taken[index] = true;
        }
        // gfx-perf: a frame drawn without a clear, whose display's last
        // frame was not native, starts from black. Both displays draw into
        // one colour buffer, so what such a frame draws over is the other
        // display's picture, which the game covers.
        // gfx-bottom: on by default (FATES_GFX_NOCLEAR_BLACK=0 turns it
        // off), see bottom.rs
        if clear.is_none() && !self.chain_valid[index] {
            if let Some(rgba) = bottom::seed() {
                clear = Some((rgba, 1.0));
                self.stats.bottom.seeded[index] += 1;
            }
        }
        if clear.is_none() && !self.chain_valid[index] {
            // not cleared: the framebuffer keeps what was drawn before,
            // which only the emulated path has
            self.stats.emulated[index] += 1;
            return -1;
        }
        let states = prim::state_list(system, prim_ctx, disp);
        self.stats.states += states.len() as u64;
        let decoded: Vec<PrimDecoded> = states.iter().map(|&s| prim::decode(system, &tables, prim_ctx, disp, s)).collect();
        wide2::trace(system, prim_ctx, disp, &states, &decoded); // wide2: FATES_WIDE2_TRACE
        gfx2d3_trace(self.stats.frames, disp, &decoded); // gfx2d3: dev, FATES_GFX2D3_TRACE=a-b
        if disp == 0 {
            wide2::frame_rects(); // wide2: the wallpapers DrawRect stretched for this frame
        }
        static STATES_TRACED: [std::sync::atomic::AtomicBool; 2] = [std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(false)];
        // gfx-terrain: dev, FATES_GFX_TRACE_STATES=N: one frame's states
        // from gfx frame N, with their callbacks' OnBegin
        if crate::perf::var("FATES_GFX_TRACE_STATES").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| self.stats.frames >= n) && !STATES_TRACED[(disp & 1) as usize].swap(true, std::sync::atomic::Ordering::Relaxed) { // gfx-perf: cached env
            for (i, &st) in states.iter().enumerate() {
                let (drawn, cb) = prim::state_callback(system, prim_ctx, st);
                let vt = if cb != 0 { prim::r32(system, cb) } else { 0 };
                let on_begin = if vt != 0 { prim::r32(system, vt) } else { 0 };
                let w: Vec<String> = (0..6).map(|k| format!("{:08X}", prim::r32(system, st + 4 * k))).collect();
                eprintln!("gfx-state disp {disp} {i}: {} drawn {drawn} callback {cb:08X} on_begin {on_begin:08X}", w.join(" "));
            }
        }
        let mut last_island: Option<usize> = None;
        for (i, d) in decoded.iter().enumerate() {
            match d {
                PrimDecoded::Island(Island::Callback(kind)) => {
                    if crate::perf::var_os("FATES_GFX_TRACE_CALLBACKS").is_some() { // gfx-perf: cached env
                        // a post effect's effect object and what it runs
                        let (_, object) = prim::state_callback(system, prim_ctx, states[i]);
                        let effect = prim::r32(system, object + 4);
                        let vtable = prim::r32(system, effect);
                        let run = zakuro_core::region::to_us(prim::r32(system, vtable + 8));
                        let key = (*kind, if *kind == 0x0021_E220 { run } else { 0 });
                        if self.traced_callbacks.insert(key) {
                            eprintln!("gfx-callback disp {disp} frame {} kind {:08X} object {object:08X} effect {effect:08X} run {run:08X} at {i}/{}", self.stats.frames, kind, decoded.len());
                            // gfx-post: what a post effect's object holds
                            if *kind == 0x0021_E220 {
                                post::trace_effect(system, effect);
                            }
                        }
                    }
                    self.stats.callbacks += 1;
                    *self.stats.callback_kinds.entry(*kind).or_default() += 1;
                    last_island = Some(i);
                }
                PrimDecoded::Island(_) => {
                    self.stats.lines += 1;
                    last_island = Some(i);
                }
                // depth-tested after an island: it tests against depth only
                // the emulated path has
                PrimDecoded::Draw(d) if last_island.is_some() && d.depth_test && d.depth_func != 1 => {
                    self.stats.depth += 1;
                    last_island = Some(i);
                }
                _ => {}
            }
        }
        // the native 3D path: when every island is a 3D scene, the frame is
        // drawn natively in full, the scenes' meshes through DrawMesh while
        // the original RenderTarget runs the scene callbacks alone
        // gfx-bottom: an uncleared frame chains from the display's previous
        // native picture here too (the lower display's field ShadowScene)
        if self.mode != Mode::Off && native3d_enabled() && last_island.is_some() && (clear.is_some() || self.chain_valid[index]) {
            // gfx-shadow: the shadow pass is native too
            // gfx-bottom: and callbacks that only render into a texture of
            // their own (UnitIconInstance's unit-icon captures), left to the
            // original RenderTarget in the compacted list
            let native_kind = |k: u32| k == SCENE_SYSTEM_ON_BEGIN || (k == shadow::ON_BEGIN && shadow::enabled()) || bottom::offscreen(k) || (k == live2d::ON_BEGIN && live2d::enabled()); // jpn: live2d
            // gfx-post: post effects the native layer draws count too
            let post_ok = |system: &mut System, i: usize| {
                let object = prim::state_callback(system, prim_ctx, states[i]).1;
                post::enabled() && post::supported(system, object)
            };
            let scenes_only = decoded.iter().enumerate().all(|(i, d)| match d {
                PrimDecoded::Island(Island::Callback(k)) => native_kind(*k) || (*k == post::POST_ON_BEGIN && post_ok(system, i)),
                PrimDecoded::Island(Island::Lines) => false,
                _ => true,
            });
            if scenes_only {
                if clear.is_none() {
                    self.stats.bottom.chained_islands[index] += 1; // gfx-bottom
                }
                return self.native_islands(system, prim_ctx, disp, states, decoded, clear);
            }
        }
        let keep = last_island.map_or(0, |i| i + 1);
        // gfx-bottom: an uncleared frame with islands the native path does
        // not draw, after a native frame: the emulated path's colour buffer
        // lacks the native picture either way, so a hybrid (the emulated
        // part over what the buffer holds, which the game's first draws
        // cover, then the native draws) is no worse than leaving the whole
        // frame to it, and draws more natively
        if keep > 0 && clear.is_none() && self.mode == Mode::Native {
            if let Some(rgba) = bottom::seed() {
                clear = Some((rgba, 1.0));
                self.stats.bottom.hybrid_seeded[index] += 1;
            }
        }
        if keep > 0 && (self.mode == Mode::Shadow || clear.is_none()) {
            // shadow mode compares whole native frames only; a hybrid
            // without a clear would draw over a picture missing the native
            // draws of the frame before
            self.stats.emulated[index] += 1;
            self.chain_valid[index] = false;
            return -1;
        }
        self.chain_valid[index] = keep == 0;
        let draws: Vec<Draw> = decoded
            .into_iter()
            .zip(states.iter().copied())
            .skip(keep)
            .flat_map(|(d, state)| match d {
                #[allow(unused_mut)]
                PrimDecoded::Draw(mut d) => {
                    // wide2: full-width 2D layers' sides, right after them
                    let sides = if disp == 0 { wide2::extend_2d(&mut d, zakuro_gpu::wide::factor(), state) } else { None };
                    std::iter::once(d).chain(sides).collect::<Vec<_>>()
                }
                _ => Vec::new(),
            })
            .collect();
        self.stats.draws += draws.len() as u64;
        if keep > 0 {
            self.stats.hybrid[index] += 1;
        } else {
            self.stats.native[index] += 1;
        }
        // FATES_GFX_TRACE=N: the draws of the first frame of each display
        // from game frame N on
        static TRACED: [std::sync::atomic::AtomicBool; 2] =
            [std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(false)];
        if crate::perf::var("FATES_GFX_TRACE").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| self.stats.frames >= n) // gfx-perf: cached env
            && !TRACED[index].swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            for (i, d) in draws.iter().enumerate() {
                let v: Vec<[f32; 5]> = d
                    .vertices
                    .chunks_exact(24)
                    .take(4)
                    .map(|c| std::array::from_fn(|k| f32::from_le_bytes(c[k * 4..k * 4 + 4].try_into().unwrap())))
                    .collect();
                let rgba: Vec<[u8; 4]> = d.vertices.chunks_exact(24).take(4).map(|c| [c[20], c[21], c[22], c[23]]).collect();
                eprintln!(
                    "gfx-trace disp {disp} draw {i}: tex {:?} color {:?} blend {:?} tev {}/{} ops {:?} alpha {} {} {} depth {} {} {} cull {} sc {:?} vp {:?} n {} v {:?} vc {:?} world {:?} vpm {:?} texm {:?}",
                    d.textures, d.color, d.blend, d.combiner.stages[0].comb_rgb, d.combiner.stages[0].comb_a, d.combiner.stages[0].op_rgb, d.alpha_test, d.alpha_func, d.alpha_ref,
                    d.depth_test, d.depth_func, d.depth_write, d.cull, d.scissor, d.viewport, d.indices.len(), v, rgba, d.world, d.view_proj, d.tex_matrix
                );
            }
        }
        let id = self.next_id;
        self.next_id += 1;
        let n = draws.len();
        // gfxmodels3: dev, FATES_GFX_FRAME_LOG=1: each frame's states, draws and clear
        if crate::perf::var_os("FATES_GFX_FRAME_LOG").is_some() {
            eprintln!("gfx-frame game {} id {id} disp {disp} states {} draws {n} clear {:?} keep {keep}", zakuro_core::native_gfx::HEADLESS_FRAME.load(std::sync::atomic::Ordering::Relaxed), states.len(), clear);
        }
        self.pending[index] = Some(Frame { id, disp, clear, rendered: None, hybrid: keep > 0, draws, texture_keys: vec![[None; 3]; n], hfr: None });
        match self.mode {
            Mode::Native => keep as i32,
            _ => -1,
        }
    }

    /// a frame whose islands are all 3D scenes, drawn natively in full: the
    /// state list the original RenderTarget gets is compacted to the scene
    /// callbacks, which run the game's scene code; their DrawMesh calls
    /// are drawn natively (draw_mesh) into the islands' places.
    fn native_islands(&mut self, system: &mut System, prim_ctx: u32, disp: u32, states: Vec<u32>, decoded: Vec<PrimDecoded>, clear: Option<([f32; 4], f32)>) -> i32 {
        self.splice_islands(system);
        let index = disp as usize;
        let compacted = self.mode == Mode::Native;
        let mut draws = Vec::new();
        let mut slots = Vec::new();
        let mut callbacks = Vec::new();
        for (state, d) in states.iter().zip(decoded) {
            match d {
                #[allow(unused_mut)]
                PrimDecoded::Draw(mut d) => {
                    // wide2: full-width 2D layers' sides, right after them
                    let sides = if disp == 0 { wide2::extend_2d(&mut d, zakuro_gpu::wide::factor(), *state) } else { None };
                    draws.push(d);
                    draws.extend(sides);
                }
                PrimDecoded::Island(Island::Callback(kind)) => {
                    // gfx-final: an island left to the emulated GPU in the
                    // compacted list (UnitIconInstance's captures)
                    if compacted && bottom::offscreen(kind) {
                        if capture::enabled() {
                            // gfx-final: native, if its OnBegin reaches the hook
                            self.capture_expected[index] = true;
                        } else {
                            self.emu.need("unit-icon capture");
                        }
                    }
                    let (_, object) = prim::state_callback(system, prim_ctx, *state);
                    slots.push((object, draws.len()));
                    callbacks.push(*state);
                }
                _ => {}
            }
        }
        let list = prim_ctx + 0x1878 + disp * 12;
        let begin = prim::r32(system, list);
        let saved = states.clone();
        if compacted {
            for (k, &state) in callbacks.iter().enumerate() {
                prim::w32(system, begin + 4 * k as u32, state);
            }
        }
        self.stats.draws += draws.len() as u64;
        self.stats.native[index] += 1;
        self.chain_valid[index] = true;
        let id = self.next_id;
        self.next_id += 1;
        let n = draws.len();
        self.pending[index] = Some(Frame { id, disp, clear, rendered: None, hybrid: false, draws, texture_keys: vec![[None; 3]; n], hfr: None });
        self.islands = Some(Islands { disp: index, slots, draws: HashMap::new(), list, saved, compacted, posts: HashMap::new() });
        if compacted {
            callbacks.len() as i32
        } else {
            -1
        }
    }

    /// jpn: a Live2D mesh (live2d.rs) into the frame being composed, at its
    /// Live2DCallBack's place; 1 when it is drawn natively (the game's
    /// drawTexture is then skipped), 0 when it is left to the game.
    #[allow(clippy::too_many_arguments)]
    fn live2d_draw(&mut self, system: &mut System, this: u32, tex: u32, index_count: u32, vertex_count: u32, indices: u32, xys: u32, uvs: u32, opacity: f32, composition: u32) -> u32 {
        let Some(islands) = self.islands.as_mut() else { return 0 };
        // the island whose callback is a Live2DCallBack
        let mut object = None;
        for &(o, _) in &islands.slots {
            let vtable = prim::r32(system, o);
            if vtable != 0 && zakuro_core::region::to_us(prim::r32(system, vtable)) == live2d::ON_BEGIN {
                object = Some(o);
            }
        }
        let Some(object) = object else { return 0 };
        let disp = islands.disp as u32;
        match live2d::mesh(system, this, tex, index_count, vertex_count, indices, xys, uvs, opacity, composition, disp) {
            Some(d) => {
                islands.draws.entry(object).or_default().push((d, [None; 3]));
                self.stats.live2d += 1;
            }
            // a mesh it can't read is left out, never sent to a GPU there isn't
            None => self.stats.live2d_skipped += 1,
        }
        1
    }

    /// the original RenderTarget is done with a compacted list: the list
    /// put back, the islands' mesh draws into the frame.
    fn prim_target_done(&mut self, system: &mut System, _disp: u32) {
        self.splice_islands(system);
    }

    /// the islands' mesh draws into their frame, the state list put back
    /// when it was compacted.
    fn splice_islands(&mut self, system: &mut System) {
        // gfx-post: a recording a budget stop left open ends here
        if self.post_callback.take().is_some() {
            cache::arm_copies(false); // gfx-perf
            let _ = self.post.end();
        }
        // gfx-gaps: unit-icon callbacks left open or not met. gfx-units: one
        // whose end hook was lost (the original left the override at an svc
        // or budget exit) still has its draws spliced in; so does a capture
        self.capture_end_instance(system, "frame end");
        if self.unit_icon.is_some() {
            self.unit_icon_end(system, 0);
        }
        self.unit_pending.clear();
        let Some(islands) = self.islands.take() else { return };
        if islands.compacted {
            let begin = prim::r32(system, islands.list);
            for (k, &state) in islands.saved.iter().enumerate() {
                prim::w32(system, begin + 4 * k as u32, state);
            }
        }
        let disp = islands.disp;
        let frames = self.stats.frames;
        let Some(frame) = self.pending[islands.disp].as_mut() else { return };
        let mut islands = islands;
        // back to front, so the earlier positions stay where they are
        for (object, at) in islands.slots.iter().rev() {
            // gfx-post: a post effect at its callback's place
            if let Some(block) = islands.posts.remove(object) {
                frame.texture_keys.insert(*at, [None; 3]);
                frame.draws.insert(*at, post::as_draw(block));
            }
            let Some(mut mesh_draws) = islands.draws.remove(object) else { continue };
            // gfx-final: dev, FATES_GFX_SKIP_BLEND=hex: leave out the mesh
            // draws with that blend function register (to see what they draw)
            if let Ok(skip) = crate::perf::var("FATES_GFX_SKIP_BLEND") {
                let skip = u32::from_str_radix(&skip, 16).ok();
                mesh_draws.retain(|(d, _)| d.shaded.as_ref().map_or(true, |s| s.key.blend != skip));
            }
            // gfx-final: dev, FATES_GFX_NODEPTH_BLEND=hex: those draws
            // without their depth test
            if let Ok(which) = crate::perf::var("FATES_GFX_NODEPTH_BLEND") {
                let which = u32::from_str_radix(&which, 16).ok();
                for (d, _) in mesh_draws.iter_mut() {
                    if let Some(s) = d.shaded.as_mut().filter(|s| s.key.blend == which) {
                        let mut t = (**s).clone();
                        t.key.depth_test = false;
                        *s = Arc::new(t);
                    }
                }
            }
            let (d, k): (Vec<Draw>, Vec<[Option<u64>; 3]>) = mesh_draws.into_iter().unzip();
            frame.texture_keys.splice(*at..*at, k);
            frame.draws.splice(*at..*at, d);
        }
        gfx2d3_trace_shaded(frames, disp, &frame.draws); // gfx2d3: dev, FATES_GFX2D3_TRACE=a-b
        skybg::trace_frame(disp as u32, &frame.draws); // skybg: dev, FATES_SKYBG_TRACE
        // FATES_GFX_TRACE_ISLANDS=N: the draws of one frame of each display
        // from game frame N on
        static TRACED: [std::sync::atomic::AtomicBool; 2] = [std::sync::atomic::AtomicBool::new(false), std::sync::atomic::AtomicBool::new(false)];
        if crate::perf::var("FATES_GFX_TRACE_ISLANDS").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|n| frames >= n) // gfx-perf: cached env
            && !TRACED[disp].swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            for (i, d) in frame.draws.iter().enumerate() {
                match &d.shaded {
                    Some(sh) => eprintln!("gfx-islands disp {disp} draw {i}: {sh:?}"),
                    None => eprintln!(
                        "gfx-islands disp {disp} draw {i}: tex {:X?} color {:?} blend {:?} alpha {} {} {} depth {} {} {} vp {:?} sc {:?} n {} st0 {:?} vc {:?}",
                        d.textures.map(|t| t.map(|t| (t.0.addr, t.0.width, t.0.height, t.0.format))), d.color, d.blend, d.alpha_test, d.alpha_func, d.alpha_ref, d.depth_test, d.depth_func, d.depth_write, d.viewport, d.scissor, d.indices.len(), d.combiner.stages[0],
                        d.vertices.chunks_exact(24).take(2).map(|c| [c[20], c[21], c[22], c[23]]).collect::<Vec<_>>()
                    ),
                }
            }
        }
    }

    /// a display transfer into `output` ran: the frame queued for it is
    /// complete. Its textures are read now, after the emulated GPU drew
    /// anything they were rendered into.
    fn transferred(&mut self, system: &mut System, output: u32) {
        debug(format_args!("transferred {output:08X} queued {:?}", self.queued.keys().collect::<Vec<_>>()));
        if std::env::var("FATES_GFX_SHADOW_COMPARE").is_ok_and(|v| v == "transfer") {
            self.shadow_compare(system); // gfx-shadow
        }
        if bottom::traced(self.stats.frames) {
            // gfx-bottom: dev
            eprintln!("gfx-bottom transferred {output:08X} queued {:?}", self.queued.get(&output).map(|q| q.iter().map(|f| (f.0, f.1.as_ref().map(|f| f.id))).collect::<Vec<_>>()));
        }
        let Some(mut frame) = (match self.queued.get_mut(&output).and_then(|q| q.pop_front()) {
            Some((_, frame)) => frame,
            None => {
                // gfx-bottom: a transfer nothing was queued for (a repeat
                // bottom::queue left out): the buffer's picture is unchanged.
                // Buffers never queued (the right eye's) are not counted.
                if self.queued.contains_key(&output) {
                    self.stats.bottom.unqueued += 1;
                }
                return;
            }
        }) else {
            // the emulated path alone drew this picture
            self.committed.remove(&output);
            return;
        };
        RENDER_SCALE.store(system.gpu.scale().max(1), std::sync::atomic::Ordering::Relaxed); // gfx-2d
        // each texture is read and hashed once per frame, however many
        // draws use it
        let mut seen: HashMap<TexRef, Option<u64>> = HashMap::new();
        let mut seen_mip: HashMap<(TexRef, u32), Option<u64>> = HashMap::new(); // gfx-2d
        for (n, d) in frame.draws.iter().enumerate() {
            if let Some(s) = d.shaded.as_deref() {
                // decoded for raster.frag when the mesh was drawn; gfx-2d:
                // and looked at again now that the command list ran. The
                // GPU reads a mesh's textures when it runs the list, after
                // the DMAs queued before it: a model loaded this frame
                // (the avatar creator's new body or hair) has its textures
                // copied to VRAM after DrawMesh recorded it, and read then
                // its face came out black or white for a frame. A texture
                // whose bytes did not change is a watch check
                // (cache.rs); FATES_GFX_MESH_TEXTURES_LATE=0 keeps the
                // keys from the record
                if perf::var("FATES_GFX_MESH_TEXTURES_LATE").map_or(true, |v| v != "0") {
                    for unit in 0..3 {
                        let Some((tex, _)) = s.textures[unit] else { continue };
                        if wide2::softened(tex).is_some() {
                            continue;
                        }
                        let levels = s.mips[unit].levels as u32;
                        let key = match seen_mip.get(&(tex, levels)) {
                            Some(&key) => key,
                            None => {
                                let key = self.texture_mip(system, tex, levels);
                                seen_mip.insert((tex, levels), key);
                                key
                            }
                        };
                        if key.is_some() {
                            frame.texture_keys[n][unit] = key;
                        }
                    }
                }
                continue;
            }
            for unit in 0..3 {
                let Some((tex, _)) = d.textures[unit] else { continue };
                let key = match seen.get(&tex) {
                    Some(&key) => key,
                    None => {
                        let key = self.texture(system, tex);
                        seen.insert(tex, key);
                        key
                    }
                };
                frame.texture_keys[n][unit] = key;
            }
        }
        // wide-ui: a 5:3 screen's sides framed
        if frame.disp == 0 {
            wide_ui::finish(&mut frame.draws, &mut frame.texture_keys, &mut self.decoded, zakuro_gpu::wide::factor());
        }
        self.hfr_track(system, &mut frame); // gfx-hfr
        // gfx-models: a frame put into the buffer the screen shows now (the
        // swap to it came first): counted, see hfr.rs pinned
        if let Some(fb) = system.gpu.framebuffers.get(frame.disp as usize) {
            let shown = fb.address_left();
            if shown != 0 && zakuro_core::services::gsp::physical_to_virtual(system, shown) == output {
                self.hfr.into_shown += 1;
            }
        }
        // rtpace: dev, an upper-screen frame committed, in the timing log
        // (a = its id)
        if frame.disp == 0 && zakuro_core::pcos::rtpace::wanted() {
            zakuro_gpu::smooth::point("commit", std::time::Instant::now(), frame.id as i64, 0.0);
        }
        if self.gpu.is_some() {
            // drawn on the GPU when shown, in order
            let frame = Arc::new(frame);
            self.hfr_commit(&frame); // gfx-hfr
            let queue = &mut self.gpu_queue[frame.disp as usize];
            queue.push_back(frame.clone());
            while queue.len() > 8 {
                queue.pop_front();
            }
            self.committed.insert(output, frame);
            return;
        }
        if !frame.hybrid {
            // drawn natively in full: render it now, in order, so a frame
            // after it drawn without a clear can start from its picture
            let scale = system.gpu.scale().max(1);
            frame.rendered = self.render_full(&frame, scale);
        }
        let frame = Arc::new(frame);
        self.hfr_commit(&frame); // gfx-hfr
        self.committed.insert(output, frame);
    }

    /// reads and decodes a texture, returning its content hash.
    fn texture(&mut self, system: &mut System, tex: TexRef) -> Option<u64> {
        // wide-ui: the side panels' own texture
        if let Some(key) = wide_ui::texture_key(&mut self.decoded, tex) {
            return Some(key);
        }
        // wide2: a full-width layer's side samples a softened copy
        if let Some((base, along_t)) = wide2::softened(tex) {
            let key = self.texture(system, base)?;
            let key = wide2::soft_key(&mut self.decoded, tex, base, key, along_t);
            let key = self.note_decoded(tex, key); // gfx-2d
            self.moflexhd_keep_few(tex); // moflexhd: a movie picture's softened copies too
            return key;
        }
        let flip = self.flip_t;
        let key = self.texture_oriented(system, tex, flip);
        self.note_decoded(tex, key) // gfx-2d
    }

    /// gfx-2d: a decoded texture handed out to a frame.
    fn note_decoded(&mut self, tex: TexRef, key: Option<u64>) -> Option<u64> {
        if let Some(k) = key {
            self.decoded_used.insert((tex, k), self.stats.frames);
        }
        key
    }

    /// moflexhd: a movie's picture (a texture the YUV-to-RGB converter
    /// writes, a new picture 30 or 60 times a second) keeps its last few
    /// pictures decoded, not every one it ever had: each used to stay until
    /// 4096 textures were held (2 GB for the game's own movies, 16 GB for a
    /// 1024x1024 HD one). The current picture, and any drawn in the last
    /// frames, always stay.
    fn moflexhd_keep_few(&mut self, tex: TexRef) {
        if !moflexhd::few_kept(tex) {
            return;
        }
        let now = self.stats.frames;
        let mut versions: Vec<((TexRef, u64), u64)> =
            self.decoded.keys().filter(|k| k.0 == tex).map(|k| (*k, self.decoded_used.get(k).copied().unwrap_or(now))).collect();
        if versions.len() <= moflexhd::KEPT_VERSIONS {
            return;
        }
        versions.sort_by_key(|&(_, used)| std::cmp::Reverse(used));
        for (k, used) in versions.into_iter().skip(moflexhd::KEPT_VERSIONS) {
            if used + 3 <= now {
                self.decoded.remove(&k);
                self.decoded_used.remove(&k);
            }
        }
    }

    /// gfx-2d: the decoded textures, past their limit, cut down to those
    /// handed out in the last frames. They used to be dropped all at once:
    /// a texture decoded earlier in the same frame (one whose bytes change
    /// often, or one just loaded) then had no texels when the frame was
    /// drawn, and the renderer drew it with its white stand-in for that
    /// frame; a menu's selection box flashed white now and then.
    fn trim_decoded(&mut self) {
        let now = self.stats.frames;
        let used = &self.decoded_used;
        let recent = |k: &(TexRef, u64)| used.get(k).is_some_and(|&f| now.saturating_sub(f) <= 120);
        let before = self.decoded.len();
        self.decoded.retain(|k, _| recent(k));
        self.decoded_used.retain(|_, f| now.saturating_sub(*f) <= 120);
        if self.decoded.len() > 3072 {
            // still too many in use: the oldest go regardless, but never
            // this frame's
            self.decoded.retain(|k, _| self.decoded_used.get(k).is_some_and(|&f| f + 2 >= now));
        }
        if perf::var_os("FATES_GFX_DECODED_LOG").is_some() {
            eprintln!("gfx-2d: decoded textures trimmed at gfx frame {now}: {before} -> {}", self.decoded.len());
        }
    }

    /// a texture decoded with row 0 at t = 0 (`flip`, the 2D layer's
    /// shaders) or at t = 1 (raster.frag's, which samples 1 - t); the two
    /// are kept under different keys.
    fn texture_oriented(&mut self, system: &mut System, tex: TexRef, flip: bool) -> Option<u64> {
        // gfx-perf: read again only when guest memory says its bytes changed
        if let Some(key) = self.shadow_texture(system, tex, flip) {
            return Some(key);
        }
        let found = self.cache.textures.lookup(system, tex, flip, &self.decoded);
        let synced = std::mem::take(&mut self.cache.textures.synced); // gfx-final
        self.emu.dependency(self.stats.frames, tex.addr, 0, "watched texture", synced);
        if let Some(key) = found {
            return Some(key);
        }
        self.cache.pack_pending = false;
        let key = self.texture_oriented_read(system, tex, flip);
        if let Some(k) = key.filter(|_| !self.cache.pack_pending) {
            self.cache.textures.store(system, tex, flip, k);
        }
        key
    }

    fn texture_oriented_read(&mut self, system: &mut System, tex: TexRef, flip: bool) -> Option<u64> {
        // gfx-shadow: a shadow map the native renderer drew
        if let Some(key) = self.shadow_texture(system, tex, flip) {
            return Some(key);
        }
        let format = zakuro_gpu::texture::TextureFormat::from_raw(tex.format);
        if matches!(format, zakuro_gpu::texture::TextureFormat::Unknown(_)) || tex.width == 0 || tex.height == 0 {
            return None;
        }
        if tex.width > 1024 || tex.height > 1024 {
            return None;
        }
        let size = tex.width * tex.height * format.bits_per_pixel() / 8;
        // moflexhd: FATES_MOVIE_STATS times a large texture's steps
        let mut timed = (tex.width * tex.height >= 256 * 1024 && zakuro_core::moflexhd::on()).then(std::time::Instant::now);
        let mut step = |kind: usize| {
            if let Some(started) = timed {
                zakuro_core::moflexhd::note(kind, started.elapsed().as_nanos() as u64, 0);
                timed = Some(std::time::Instant::now());
            }
        };
        // gfx-bottom: the emulated GPU keeps its buffers by the guest's
        // virtual address (as DMA syncs them), so a texture it drew into
        // (UnitIconObject::CaptureTexture's unit icons) only comes down
        // through that address; synced by its physical one it never did
        let writes = system.sync_gpu_counted(tex.addr, size); // gfx-final: counted
        if writes > 0 {
            self.emu.dependency(self.stats.frames, tex.addr, size, &format!("texture {}x{} format {} flip {flip}", tex.width, tex.height, tex.format), writes);
        }
        step(zakuro_core::moflexhd::kind::TEX_SYNC);
        let mut raw = vec![0u8; size as usize];
        system.memory.read_bytes(tex.addr, &mut raw);
        let content = fnv(&raw);
        let hash = content ^ if flip { 0 } else { 0x00F1_1F00_F11F_0000 };
        // mods: recorded for making texture packs, when asked to
        zakuro_gpu::pack::record_native(&raw, format, tex.width, tex.height);
        step(zakuro_core::moflexhd::kind::TEX_READ);
        // a texture pack's picture in its place (the same pack the emulated
        // GPU draws with, e.g. the controller glyph swaps), once it is read
        // hdpacks: read in the background and kept within a budget (hd.rs)
        // moflexdec: not an HD movie's picture (every one new; no pack has it)
        let hd_picture = moflexhd::hd_picture(tex);
        let found = if hd_picture { hd::Got::None } else { self.hd.get(&raw, content, format, tex.width, tex.height, flip) };
        step(zakuro_core::moflexhd::kind::TEX_PACK);
        match found {
            hd::Got::Ready(picture) => {
                let key = (tex, hash ^ 0x9E37_79B9_7F4A_7C15);
                self.decoded.entry(key).or_insert(picture);
                return Some(key.1);
            }
            hd::Got::Pending => self.cache.pack_pending = true, // gfx-perf: not read yet
            hd::Got::None => {}
        }
        let key = (tex, hash);
        if !self.decoded.contains_key(&key) {
            let (w, h) = (tex.width, tex.height);
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            // moflexhd: a tile at a time (a movie's picture changes every
            // frame; an HD one is 1024x1024), texel for texel the same
            let timed = (w * h >= 256 * 1024 && zakuro_core::moflexhd::on()).then(std::time::Instant::now);
            if hd_picture {
                moflexhd::decode_bands(&raw, format, w, h, flip, &mut rgba); // moflexdec: the same texels, on a few threads
            } else {
                zakuro_gpu::texture::decode_texture(&raw, format, w, h, flip, &mut rgba);
            }
            if let Some(started) = timed {
                zakuro_core::moflexhd::note(zakuro_core::moflexhd::kind::TEXELS, started.elapsed().as_nanos() as u64, (w * h) as u64);
            }
            if self.decoded.len() > 4096 {
                self.trim_decoded(); // gfx-2d
            }
            if let Some(dir) = std::env::var_os("FATES_GFX_TEXDUMP") {
                let mut shown = rgba.clone();
                for px in shown.chunks_exact_mut(4) {
                    if px[0] == 0 && px[1] == 0 && px[2] == 0 {
                        px[0] = px[3];
                        px[1] = px[3];
                        px[2] = px[3];
                        px[3] = 255;
                    }
                }
                let _ = std::fs::create_dir_all(&dir);
                let path = std::path::Path::new(&dir).join(format!("{:08X}-{}x{}-f{}-{:016X}.png", tex.addr, w, h, tex.format, hash));
                let _ = write_png(&path, &shown, w, h);
            }
            self.decoded.insert(key, Arc::new(Decoded { width: w, height: h, rgba, levels: 1 }));
            self.moflexhd_keep_few(tex); // moflexhd
        }
        Some(hash)
    }

    /// gfx-terrain: a shaded draw's texture with `levels` sizes, each half
    /// the last, one after the other in memory (zakuro-gpu raster.rs
    /// `texture_levels`), decoded as raster.frag samples them (row 0 at
    /// t = 1). One size, or a surface the renderer drew, is
    /// `texture_oriented`'s.
    fn texture_mip(&mut self, system: &mut System, tex: TexRef, levels: u32) -> Option<u64> {
        // skybg: a cube map's six faces, stacked
        if let Some(key) = self.cube_key(system, tex) {
            return self.note_decoded(tex, key);
        }
        let key = self.texture_mip_inner(system, tex, levels);
        self.note_decoded(tex, key) // gfx-2d
    }

    fn texture_mip_inner(&mut self, system: &mut System, tex: TexRef, levels: u32) -> Option<u64> {
        // gfx-perf: read again only when guest memory says its bytes changed
        if levels > 1 && self.shadow_texture(system, tex, false).is_none() {
            let found = self.cache.textures.lookup_mip(system, tex, levels, &self.decoded);
            let synced = std::mem::take(&mut self.cache.textures.synced); // gfx-final
            self.emu.dependency(self.stats.frames, tex.addr, 0, "watched mip texture", synced);
            if let Some(key) = found {
                return Some(key);
            }
            self.cache.pack_pending = false; // hdpacks
            let key = self.texture_mip_read(system, tex, levels);
            if let Some(k) = key.filter(|_| !self.cache.pack_pending) {
                self.cache.textures.store_mip(system, tex, levels, k);
            }
            return key;
        }
        self.texture_mip_read(system, tex, levels)
    }

    fn texture_mip_read(&mut self, system: &mut System, tex: TexRef, levels: u32) -> Option<u64> {
        if levels <= 1 {
            return self.texture_oriented(system, tex, false);
        }
        if let Some(key) = self.shadow_texture(system, tex, false) {
            return Some(key);
        }
        let format = zakuro_gpu::texture::TextureFormat::from_raw(tex.format);
        if matches!(format, zakuro_gpu::texture::TextureFormat::Unknown(_)) || tex.width > 1024 || tex.height > 1024 {
            return self.texture_oriented(system, tex, false);
        }
        let bits = format.bits_per_pixel();
        let sizes: Vec<(u32, u32, usize)> =
            (0..levels).map(|l| (tex.width >> l, tex.height >> l, ((tex.width >> l) * (tex.height >> l) * bits / 8) as usize)).collect();
        let size: usize = sizes.iter().map(|s| s.2).sum();
        let writes = system.sync_gpu_counted(tex.addr, size as u32); // gfx-bottom: by its virtual address
        self.emu.dependency(self.stats.frames, tex.addr, size as u32, "mip texture", writes); // gfx-final
        let mut raw = vec![0u8; size];
        system.memory.read_bytes(tex.addr, &mut raw);
        let hash = fnv(&raw) ^ 0x00F1_1F00_F11F_0000 ^ (levels as u64) << 56;
        // hdpacks: the pack's picture for its largest size, as Citra hashes
        // a texture with smaller sizes by its first (gfx-terrain's mip
        // chains: the map floors, models)
        if self.hd.active() || zakuro_gpu::pack::recording() {
            let first = &raw[..sizes[0].2];
            zakuro_gpu::pack::record_native(first, format, tex.width, tex.height);
            match self.hd.get(first, fnv(first), format, tex.width, tex.height, false) {
                hd::Got::Ready(picture) => {
                    let key = (tex, hash ^ 0x9E37_79B9_7F4A_7C15);
                    self.decoded.entry(key).or_insert(picture);
                    return Some(key.1);
                }
                hd::Got::Pending => self.cache.pack_pending = true,
                hd::Got::None => {}
            }
        }
        let key = (tex, hash);
        if !self.decoded.contains_key(&key) {
            let mut rgba = Vec::new();
            let mut at = 0usize;
            for &(w, h, bytes) in &sizes {
                let level = &raw[at..at + bytes];
                for y in 0..h {
                    for x in 0..w {
                        rgba.extend_from_slice(&zakuro_gpu::texture::sample_texel(level, format, w, x, y));
                    }
                }
                at += bytes;
            }
            if self.decoded.len() > 4096 {
                self.trim_decoded(); // gfx-2d
            }
            if let Some(dir) = std::env::var_os("FATES_GFX_TEXDUMP") {
                let _ = std::fs::create_dir_all(&dir);
                let path = std::path::Path::new(&dir).join(format!("{:08X}-{}x{}-f{}-m{levels}-{hash:016X}.png", tex.addr, tex.width, tex.height, tex.format));
                let _ = write_png(&path, &rgba[..(tex.width * tex.height * 4) as usize], tex.width, tex.height);
            }
            self.decoded.insert(key, Arc::new(Decoded { width: tex.width, height: tex.height, rgba, levels }));
        }
        Some(hash)
    }

    fn compose(&mut self, system: &mut System, index: usize, address: u32, emulated: &Screen) -> Option<Screen> {
        if index == 0 {
            if let Some((n, pixels)) = self.debug3d_pending.take() {
                if let Some(dir) = std::env::var_os("FATES_GFX_DEBUG3D") {
                    let dir = PathBuf::from(dir);
                    let _ = std::fs::create_dir_all(&dir);
                    let (w, h) = (emulated.1, emulated.2);
                    if pixels.len() == (w * h * 4) as usize {
                        let mut both = vec![0u8; (w * 2 * h * 4) as usize];
                        for y in 0..h as usize {
                            let row = w as usize * 4;
                            both[y * row * 2..y * row * 2 + row].copy_from_slice(&emulated.0[y * row..(y + 1) * row]);
                            both[y * row * 2 + row..(y + 1) * row * 2].copy_from_slice(&pixels[y * row..(y + 1) * row]);
                        }
                        let _ = write_png(&dir.join(format!("d3-{n:05}.png")), &both, w * 2, h);
                    }
                }
            }
        }
        debug(format_args!("compose {index} {address:08X} committed {:?}", self.committed.keys().collect::<Vec<_>>()));
        let frame = self.committed.get(&address)?.clone();
        if frame.disp as usize != index {
            return None;
        }
        if bottom::traced(self.stats.frames) {
            // gfx-bottom: dev
            eprintln!("gfx-bottom compose {index} {address:08X} frame {} cached {:?} pictures {:?}", frame.id, self.composed.get(&address).map(|c| c.0), self.cache.pictures[index].iter().map(|p| p.0).collect::<Vec<_>>());
        }
        if let Some((id, picture)) = self.composed.get(&address) {
            if *id == frame.id {
                return if self.mode == Mode::Native { Some(picture.clone()) } else { None };
            }
        }
        let (ew, eh) = (emulated.1, emulated.2);
        let (lw, lh) = prim::display_size(frame.disp); // portrait 240 x 400 / 320
        // the scale from the screen's height: a widescreen top screen is
        // wider than 400 * scale
        let scale = (eh / lw as u32).max(1);
        if eh != lw as u32 * scale || ew < lh as u32 * scale {
            return None;
        }
        if let Some((s, pixels)) = frame.rendered.clone() {
            if s != scale {
                return None;
            }
            // its own width: a widescreen top screen's native picture is
            // wider than a console-width emulated one
            let rw = (pixels.len() / 4) as u32 / eh;
            let picture: Screen = (pixels, rw, eh);
            self.stats.composed += 1;
            if let Some(dir) = self.dump.clone() {
                self.dump_pair(&dir, index, &frame, &picture, emulated);
            }
            self.composed.insert(address, (frame.id, picture.clone()));
            return if self.mode == Mode::Native { Some(picture) } else { None };
        }
        if !frame.hybrid {
            return self.compose_kept(index, address, &frame, emulated); // gfx-perf
        }
        self.cache.resident[index] = None; // gfx-perf: the target is drawn over
        self.ensure_renderer();
        let _ = &system;
        let keys = &frame.texture_keys;
        let renderer = self.renderer.as_mut()?;
        // the native target is the upright screen: the emulated picture is
        // its starting contents as it is
        let pixels = match renderer.render(
            index,
            ew,
            eh,
            scale as f32,
            Start::Pixels(emulated.0.as_slice()),
            frame.clear.map_or(1.0, |c| c.1),
            &frame.draws,
            keys,
            &self.decoded,
            Finish::Readback,
        ) {
            Ok(Output::Pixels(p)) => p,
            Ok(_) => return None,
            Err(e) => {
                eprintln!("gfx: render failed, {e}");
                return None;
            }
        };
        let picture: Screen = (Arc::new(pixels), ew, eh);
        self.stats.composed += 1;
        if let Some(dir) = self.dump.clone() {
            self.dump_pair(&dir, index, &frame, &picture, emulated);
        }
        self.composed.insert(address, (frame.id, picture.clone()));
        match self.mode {
            Mode::Native => Some(picture),
            _ => None,
        }
    }

    /// the native H3D draws of one DrawMesh call (scene.rs), into this
    /// frame's debug list (FATES_GFX_SCENE3D, top screen).
    fn shade_mesh(&mut self, system: &mut System, model: u32, state: u32, layer: u32, mesh: u32, world: u32) {
        let disp = match self.target {
            0x400 => 0usize,
            0x410 => return,
            _ => 1,
        };
        if disp != 0 {
            return;
        }
        let made = self.shade_draws(system, model, state, layer, mesh, world, disp, None);
        self.frame_shaded.extend(made);
    }

    /// the native draws of one DrawMesh call into display `disp`, with
    /// their texture keys.
    #[allow(clippy::too_many_arguments)]
    fn shade_draws(
        &mut self,
        system: &mut System,
        model: u32,
        state: u32,
        layer: u32,
        mesh: u32,
        world: u32,
        disp: usize,
        offscreen: Option<&shadow::Target>, // gfx-shadow: a shadow pass's target
    ) -> Vec<(Draw, [Option<u64>; 3])> {
        let mut out = Vec::new();
        // gfx-models: translated again when the address holds another mesh
        // now, or its buffers were written (models.rs); the translation
        // was kept by address forever before, and a battle's models drawn
        // with the meshes of the one before at the same addresses
        let found = self.models.get(system, mesh);
        let vertices = found.as_ref().map(|f| f.1.clone());
        let fresh = found.as_ref().is_some_and(|f| f.2);
        let Some(m) = found.map(|f| f.0).inspect(|m| {
                // gfx-terrain: FATES_GFX_MESH_LOG=1, each mesh as translated
                if fresh && crate::perf::var_os("FATES_GFX_MESH_LOG").is_some() { // gfx-perf: cached env
                    let subs: Vec<(u32, usize, u8)> = m.submeshes.iter().map(|s| (s.primitive, s.indices.len(), s.skinning)).collect();
                    eprintln!(
                        "gfx-mesh {mesh:08X} buffer {:08X} stride {} verts {} attrs {:?} fixed {:?} subs {:?}",
                        m.buffer_phys,
                        m.stride,
                        m.vertices.len() / m.stride.max(1) as usize,
                        m.attrs,
                        m.fixed,
                        subs
                    );
                }
            }) else {
            if crate::perf::var_os("FATES_GFX_MESH_LOG").is_some() { // gfx-perf: cached env
                eprintln!("gfx-mesh {mesh:08X} untranslated");
            }
            // gfx-eft: an untranslated mesh still sets the fog a following
            // effect draw inherits
            let o = setup::Objects::read(system, model, state, layer, mesh, world);
            let base = setup::mesh_setup(system, &o);
            self.eft_inherit.mesh(&base);
            if crate::perf::var_os("FATES_GFX_EFT_TRACE").is_some() { // gfx-perf: cached env
                eprintln!("gfx-eft untranslated mesh {mesh:08X} fog {:X}", base.regs[0x0E0]);
            }
            return out;
        };
        let t_setup = perf::scope(perf::C::MeshSetup); // gfx-perf
        let o = setup::Objects::read(system, model, state, layer, mesh, world);
        let mut base = setup::mesh_setup(system, &o);
        drop(t_setup);
        match offscreen {
            Some(t) => shadow::target_setup(&mut base, t),
            None => setup::target(&mut base, disp),
        }
        let vertices = vertices.unwrap_or_else(|| Arc::new(m.vertices.clone())); // gfx-models: kept with the translation
        for (k, sub) in m.submeshes.iter().enumerate() {
            // gfx-terrain: a culled mesh draws only when its model keeps a
            // culling state (Model +0x60; DrawMesh)
            if sub.culled && prim::r32(system, model + 0x60) == 0 && crate::perf::var_os("FATES_GFX_CULLED_ALL").is_none() /* gfx-perf: cached env */ {
                if crate::perf::var_os("FATES_GFX_MESH_LOG").is_some() { // gfx-perf: cached env
                    eprintln!("gfx-mesh {mesh:08X} culled, model {model:08X} has no culling state: skipped");
                }
                continue;
            }
            #[allow(unused_mut)]
            let t_setup = perf::scope(perf::C::MeshSetup); // gfx-perf
            let mut s = setup::submesh_setup(system, &o, &base, k as u32);
            drop(t_setup);
            // gfx-terrain: dev experiment, FATES_GFX_EXP_BUF=<buffer> with
            // FATES_GFX_EXP=<source>: that buffer's draws output one
            // combiner source only (3 tex0, 4 tex1, 5 tex2, 0 vertex colour,
            // 1 fragment primary colour)
            if let (Ok(b), Ok(src)) = (crate::perf::var("FATES_GFX_EXP_BUF"), crate::perf::var("FATES_GFX_EXP")) /* gfx-perf: cached env */ {
                if u32::from_str_radix(&b, 16).is_ok_and(|b| b == m.buffer_phys) {
                    let src: u32 = src.parse().unwrap_or(0);
                    for (i, st) in [0x0C0usize, 0x0C8, 0x0D0, 0x0D8, 0x0F0, 0x0F8].into_iter().enumerate() {
                        let x = if i == 0 { src } else { 0xF };
                        s.regs[st] = x | x << 16;
                        s.regs[st + 1] = 0;
                        s.regs[st + 2] = 0;
                        s.regs[st + 4] = 0;
                    }
                    s.regs[0x0C0] = src | (0xE << 16);
                    s.regs[0x0C3] = 0xFFFF_FFFF;
                }
            }
            // gfx-eft: what a following effect draw inherits
            self.eft_inherit.mesh(&s);
            let t_shaded = perf::scope(perf::C::MeshShaded); // gfx-perf
            let program = cache::program(&s); // gfx-perf: shared by content
            let virt = |phys: u32| prim::virt(system, phys);
            let Some(mut shaded) = scene::shaded(&s, &m, sub, vertices.clone(), program, &virt) else { continue };
            drawdist::apply(system, &o, k as u32, &mut shaded); // drawdist: a batch of tufts
            // sunglasses: the 3D glasses' depth pulled toward the camera,
            // over the bangs (sunglasses::pull)
            if sunglasses::is_3d(system, o.material) && sunglasses::pull(system, &mut s, &o).is_some() {
                let program = cache::program(&s);
                let virt = |phys: u32| prim::virt(system, phys);
                if let Some(pulled) = scene::shaded(&s, &m, sub, vertices.clone(), program, &virt) {
                    shaded = pulled;
                }
            }
            // sunglasses: the critical cut-in draws face_mat (the face and the
            // facial detail), then hair_mat over it. For Haiku's Sunglasses the
            // face material draws without the layer and the hair's geometry
            // draws it again over the hair (sunglasses::over_hair), so it is
            // laid down once, over both
            let mut extra = None;
            if let Some(unit) = (0..3).find(|&u| shaded.textures[u].is_some_and(|(t, _)| prim::is_sunglasses(system, &t))) {
                self.sunglasses_cutin = Some((model, self.stats.frames, sunglasses::unit_regs(&s, unit)));
                let s2 = sunglasses::without_layer(&s, unit);
                let program = cache::program(&s2);
                let virt = |phys: u32| prim::virt(system, phys);
                if let Some(face) = scene::shaded(&s2, &m, sub, vertices.clone(), program, &virt) {
                    shaded = face;
                }
            } else if let Some((m_at, frame, unit_regs)) = self.sunglasses_cutin {
                if m_at == model && frame == self.stats.frames && sunglasses::material_name(system, o.material) == "hair_mat" {
                    let s2 = sunglasses::over_hair(&s, unit_regs);
                    let program = cache::program(&s2);
                    let virt = |phys: u32| prim::virt(system, phys);
                    extra = scene::shaded(&s2, &m, sub, vertices.clone(), program, &virt);
                }
            }
            drop(t_shaded);
            let _t = perf::scope(perf::C::MeshTextures); // gfx-perf
            let mut keys = [None; 3];
            for unit in 0..3 {
                if let Some((tex, _)) = shaded.textures[unit] {
                    keys[unit] = self.texture_mip(system, tex, shaded.mips[unit].levels as u32); // gfx-terrain
                }
            }
            let draw = Draw {
                vertices: Vec::new(),
                indices: Vec::new(),
                world: [0.0; 12],
                view_proj: [0.0; 16],
                tex_matrix: [0.0; 12],
                color: [1.0; 4],
                textures: shaded.textures,
                combiner: prim::Combiner::prim_reset(),
                blend: prim::Blend::prim(7),
                depth_test: false,
                depth_func: 0,
                depth_write: false,
                alpha_test: false,
                alpha_func: 0,
                alpha_ref: 0,
                cull: 0,
                scissor: [0, 0, 0, 0],
                viewport: [0, 0, 0, 0],
                shaded: Some(Arc::new(shaded)),
                post: None,
                depth_offset: 0.0, // gfx-gaps
            };
            // sunglasses: the layer over the hair, after the hair's draw
            if let Some(over) = extra {
                out.push((draw.clone(), keys));
                let mut keys2 = [None; 3];
                if let Some((tex, _)) = over.textures[0] {
                    keys2[0] = self.texture_mip(system, tex, over.mips[0].levels as u32);
                }
                out.push((Draw { textures: over.textures, shaded: Some(Arc::new(over)), ..draw }, keys2));
            } else {
                out.push((draw, keys));
            }
        }
        out
    }

    /// FATES_GFX_DEBUG3D=dir: every FATES_GFX_DEBUG3D_EVERY game frames the
    /// frame's meshes drawn by the geometry check, written beside the
    /// emulated picture of the same frame when it is composed.
    fn debug3d_frame(&mut self, system: &mut System) {
        let shaded = std::mem::take(&mut self.frame_shaded);
        if !shaded.is_empty() && std::env::var_os("FATES_GFX_DEBUG3D").is_some() {
            self.frame3d.clear();
            let every = std::env::var("FATES_GFX_DEBUG3D_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(100u64);
            if self.stats.frames % every != 0 {
                return;
            }
            let scale = system.gpu.scale().max(1);
            let (w, h) = (400 * scale, 240 * scale);
            self.ensure_renderer();
            let (draws, keys): (Vec<Draw>, Vec<[Option<u64>; 3]>) = shaded.into_iter().unzip();
            self.cache.resident[0] = None; // gfx-perf
            let Some(renderer) = self.renderer.as_mut() else { return };
            match renderer.render(0, w, h, scale as f32, Start::Clear([0.0, 0.0, 0.0, 1.0]), 1.0, &draws, &keys, &self.decoded, Finish::Readback) {
                Ok(Output::Pixels(p)) => {
                    eprintln!("gfx-scene3d frame {}: {} draws", self.stats.frames, draws.len());
                    self.debug3d_pending = Some((self.stats.frames, p));
                }
                Ok(_) => {}
                Err(e) => eprintln!("gfx-scene3d frame {}: {e}", self.stats.frames),
            }
            return;
        }
        let list = std::mem::take(&mut self.frame3d);
        if list.is_empty() || std::env::var_os("FATES_GFX_DEBUG3D").is_none() {
            return;
        }
        let every = std::env::var("FATES_GFX_DEBUG3D_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(100u64);
        if self.stats.frames % every != 0 {
            return;
        }
        let with_world = std::env::var("FATES_GFX_DEBUG3D_WORLD").is_ok_and(|v| v == "1");
        let draws = debug3d::draws(&list, with_world);
        let scale = system.gpu.scale().max(1);
        let (w, h) = (400 * scale, 240 * scale);
        self.ensure_renderer();
        let keys = vec![[None; 3]; draws.len()];
        self.cache.resident[0] = None; // gfx-perf
        let Some(renderer) = self.renderer.as_mut() else { return };
        if let Ok(Output::Pixels(p)) = renderer.render(0, w, h, scale as f32, Start::Clear([0.1, 0.0, 0.1, 1.0]), 1.0, &draws, &keys, &self.decoded, Finish::Readback) {
            eprintln!("gfx-debug3d frame {}: {} meshes, {} draws", self.stats.frames, list.len(), draws.len());
            self.debug3d_pending = Some((self.stats.frames, p));
        }
    }

    /// perf3: the renderer for pictures the game's thread reads back at once
    /// (shadow maps, icon captures): the side queue's where the device has
    /// one, else `renderer`.
    pub(crate) fn ensure_readback(&mut self) {
        if self.side.is_none() && !self.side_failed {
            match self.cache.device.clone().filter(|d| d.side_queue().is_some()).map(Renderer::new_side) {
                Some(Ok(r)) => {
                    eprintln!("gfx: shadow maps and captures drawn on a queue of their own");
                    self.side = Some(r);
                }
                Some(Err(e)) => {
                    eprintln!("gfx: no side-queue renderer, {e}");
                    self.side_failed = true;
                }
                None => self.side_failed = true,
            }
        }
        if self.side.is_none() {
            self.ensure_renderer();
        }
    }

    fn ensure_renderer(&mut self) {
        if self.renderer.is_none() && !self.renderer_failed {
            let made = match self.cache.device.clone() {
                Some(device) => Renderer::new_shared(device), // gfx-perf
                None => Renderer::new(),
            };
            match made {
                Ok(r) => {
                    eprintln!("gfx: native renderer on {}", r.name);
                    self.renderer = Some(r);
                }
                Err(e) => {
                    eprintln!("gfx: no native renderer, {e}");
                    self.renderer_failed = true;
                }
            }
        }
    }

    /// renders a frame the native renderer draws in full, from its clear
    /// colour or the display's previous native picture.
    fn render_full(&mut self, frame: &Frame, scale: u32) -> Option<(u32, Arc<Vec<u8>>)> {
        // gfx-perf: drawn and kept on the GPU, read back only when asked for
        if cache::lazy() {
            self.render_full_lazy(frame, scale);
            return None;
        }
        let index = frame.disp as usize;
        let (lw, lh) = prim::display_size(frame.disp);
        // upright: the screen's width is the portrait framebuffer's height,
        // widened for a widescreen top screen as the wide agent's renderer
        // widens it
        let k = if frame.disp == 0 { zakuro_gpu::wide::factor().max(1.0) } else { 1.0 };
        let (uw, uh) = ((lh as f32 * scale as f32 * k).round() as u32, lw as u32 * scale);
        let base = match frame.clear {
            Some(_) => None,
            None => match &self.chain[index] {
                Some((s, pixels)) if *s == scale => Some(pixels.clone()),
                _ => {
                    self.chain[index] = None;
                    // gfx-bottom: the predecessor's picture is gone: from
                    // the seed, as an uncleared frame without one starts
                    if bottom::seed().is_none() {
                        return None;
                    }
                    self.stats.bottom.reseeded[index] += 1;
                    None
                }
            },
        };
        self.ensure_renderer();
        let renderer = self.renderer.as_mut()?;
        let start = match (&base, frame.clear) {
            (Some(pixels), _) => Start::Pixels(pixels.as_slice()),
            (None, Some((rgba, _))) => Start::Clear(rgba),
            (None, None) => Start::Clear(bottom::seed().unwrap_or([0.0, 0.0, 0.0, 1.0])), // gfx-bottom
        };
        let result = renderer.render(
            index,
            uw,
            uh,
            scale as f32,
            start,
            frame.clear.map_or(1.0, |c| c.1),
            &frame.draws,
            &frame.texture_keys,
            &self.decoded,
            Finish::Readback,
        );
        match result {
            Ok(Output::Pixels(pixels)) => {
                let out = (scale, Arc::new(pixels));
                self.chain[index] = Some(out.clone());
                Some(out)
            }
            Ok(_) => None,
            Err(e) => {
                eprintln!("gfx: render failed, {e}");
                self.chain[index] = None;
                None
            }
        }
    }

    /// the GPU path: the native picture for the display buffer at `address`
    /// drawn on the presenter's device, over the emulated picture's image
    /// for a hybrid frame.
    fn compose_gpu_frame(
        &mut self,
        system: &mut System,
        index: usize,
        address: u32,
        emulated: Option<zakuro_gpu::GpuScreen>,
        (width, height): (u32, u32),
    ) -> Option<zakuro_gpu::GpuScreen> {
        if self.gpu.is_none() {
            if self.gpu_failed || std::env::var("FATES_GFX_GPU").is_ok_and(|v| v == "0") {
                return None;
            }
            let shared = system.config.device.clone()?;
            match Renderer::new_shared(shared) {
                Ok(r) => {
                    eprintln!("gfx: native renderer on the presenter's device ({})", r.name);
                    self.gpu = Some(r);
                }
                Err(e) => {
                    eprintln!("gfx: cannot draw on the presenter's device, {e}");
                    self.gpu_failed = true;
                    return None;
                }
            }
        }
        // gfx-final: the lower display draws with a renderer of its own
        if index == 1 && split_renderers() && self.gpu_lower.is_none() && !self.gpu_failed {
            if let Some(shared) = system.config.device.clone() {
                match Renderer::new_shared(shared) {
                    Ok(r) => self.gpu_lower = Some(r),
                    Err(e) => eprintln!("gfx: no renderer of its own for the lower screen ({e}), the upper one's draws it"),
                }
            }
        }
        let frame = self.committed.get(&address)?.clone();
        if frame.disp as usize != index {
            return None;
        }
        self.prune_pictures(); // gfx-final
        let base_view = emulated.map(|e| e.view());
        if let Some((id, view, picture)) = self.gpu_shown[index] {
            if id == frame.id && (!frame.hybrid || view == base_view) {
                return Some(picture);
            }
        }
        // gfx-hfr: a frame drawn before the newest (the picture before, at
        // the start of the newest one's time) is still on the GPU
        if let Some(picture) = self.hfr.drawn(index, frame.id, if frame.hybrid { base_view } else { None }) {
            return Some(picture);
        }
        // the same hybrid frame over another emulated picture (one in
        // between, or the one before): draw it again
        if frame.hybrid && self.gpu_shown[index].is_some_and(|(id, _, _)| id == frame.id) {
            if let Some(last) = self.gpu_last_hybrid[index].clone().filter(|last| last.id == frame.id) {
                self.gpu_queue[index].push_front(last);
            }
        }
        let (lw, lh) = prim::display_size(frame.disp);
        let scale = height / lw as u32;
        if scale == 0 || height != lw as u32 * scale || width < lh as u32 * scale {
            return None;
        }
        // sharp: while every picture follows a smaller scale, the frames are
        // drawn at it (hfr.rs frame_scale), as the pictures in between size
        // theirs (prepare_between)
        let (width, height, scale) = match self.hfr.frame_scale(scale) {
            Some(s) => ((width as f32 * self.hfr.scaling.fraction()).round() as u32, lw as u32 * s, s),
            None => (width, height, scale),
        };
        // the frames before it, drawn in order so one drawn without a clear
        // finds its predecessor's picture; hybrids among them are skipped
        // (no frame after a hybrid keeps its picture)
        let mut shown = None;
        while let Some(next) = self.gpu_queue[index].front().cloned() {
            if next.id > frame.id {
                break;
            }
            self.gpu_queue[index].pop_front();
            // qol: at a game speed above 1x, a frame a later one (up to the
            // one before the shown one, which pictures in between start
            // from) draws over from a clear is not drawn
            if zakuro_core::native_gfx::latest_only() && !next.hybrid && next.id + 1 < frame.id {
                let covered = self.gpu_queue[index].iter().any(|later| later.id < frame.id && !later.hybrid && later.clear.is_some());
                if covered {
                    continue;
                }
            }
            let start = if next.hybrid {
                if next.id != frame.id {
                    continue;
                }
                let emulated = emulated?;
                self.gpu_last_hybrid[index] = Some(next.clone());
                Start::View(emulated.view(), emulated.area())
            } else {
                match next.clear {
                    Some((rgba, _)) => Start::Clear(rgba),
                    None => Start::Keep,
                }
            };
            // gfx-final: the display's own renderer
            let renderer = if index == 1 && self.gpu_lower.is_some() { self.gpu_lower.as_mut()? } else { self.gpu.as_mut()? };
            match renderer.render(
                index,
                width,
                height,
                scale as f32,
                start,
                next.clear.map_or(1.0, |c| c.1),
                &next.draws,
                &next.texture_keys,
                &self.decoded,
                Finish::Present,
            ) {
                Ok(Output::Screen(view, w, h)) => {
                    let (w, h) = (w as f32, h as f32);
                    let picture = zakuro_gpu::GpuScreen::new(view, [0.0, 0.0, 1.0, 1.0], [0.5 / w, 0.5 / h, 1.0 - 0.5 / w, 1.0 - 0.5 / h]);
                    self.gpu_shown[index] = Some((next.id, if next.hybrid { base_view } else { None }, picture));
                    self.hfr.note_drawn(index, next.id, if next.hybrid { base_view } else { None }, picture); // gfx-hfr
                    if next.id == frame.id {
                        shown = Some(picture);
                    }
                }
                Ok(_) => {}
                Err(e) => {
                    eprintln!("gfx: GPU render failed, {e}");
                    return None;
                }
            }
        }
        self.stats.composed += 1;
        shown
    }

    /// gfx-final: every renderer on the presenter's device (textures to forget)
    pub(crate) fn gpu_renderers(&mut self) -> impl Iterator<Item = &mut Renderer> {
        [self.gpu.as_mut(), self.gpu_lower.as_mut(), self.hfr.renderer.as_mut(), self.hfr.renderer_lower.as_mut()].into_iter().flatten()
    }

    /// gfx-final: pictures kept by the view of a presented image that no
    /// renderer has any more are forgotten (see Renderer::presents)
    fn prune_pictures(&mut self) {
        // dev: FATES_GFX_PRUNE=0 keeps them (the device-loss repro)
        if crate::perf::var("FATES_GFX_PRUNE").is_ok_and(|v| v == "0") {
            return;
        }
        let between = self.hfr.renderer.take();
        let between_lower = self.hfr.renderer_lower.take();
        let (gpu, gpu_lower) = (self.gpu.as_ref(), self.gpu_lower.as_ref());
        let alive = |v: ash::vk::ImageView| {
            [gpu, gpu_lower, between.as_ref(), between_lower.as_ref()].into_iter().flatten().any(|r| r.presents(v))
        };
        for shown in self.gpu_shown.iter_mut() {
            if shown.is_some_and(|(_, _, p)| !alive(p.view())) {
                *shown = None;
            }
        }
        self.hfr.prune(&alive);
        self.hfr.renderer = between;
        self.hfr.renderer_lower = between_lower;
    }

    #[allow(clippy::too_many_arguments)]
    fn dump_pair(&mut self, dir: &std::path::Path, index: usize, frame: &Frame, native: &Screen, emulated: &Screen) {
        self.dumped += 1;
        let every = std::env::var("FATES_GFX_DUMP_EVERY").ok().and_then(|v| v.parse().ok()).unwrap_or(60u64);
        if self.dumped % every != 1 && every > 1 {
            return;
        }
        let name = if index == 0 { "top" } else { "bottom" };
        let kind = if frame.hybrid { "hybrid" } else { "native" };
        let n = self.stats.frames;
        let _ = write_png(&dir.join(format!("{name}-{n:05}-{kind}.png")), &native.0, native.1, native.2);
        if self.mode == Mode::Shadow {
            let _ = write_png(&dir.join(format!("{name}-{n:05}-emulated.png")), &emulated.0, emulated.1, emulated.2);
            let (diff, over) = diff_stats(&native.0, &emulated.0);
            eprintln!(
                "gfx-diff {name} frame {n} draws {}: mean abs {:.3}, pixels off by >16: {:.3}%",
                frame.draws.len(),
                diff,
                over * 100.0,
            );
        }
    }
}

/// mean absolute difference per channel, and the share of pixels with any
/// channel off by more than 16.
pub fn diff_stats(a: &[u8], b: &[u8]) -> (f64, f64) {
    let n = a.len().min(b.len());
    if n == 0 {
        return (0.0, 0.0);
    }
    let mut sum = 0u64;
    let mut over = 0u64;
    for (pa, pb) in a[..n].chunks_exact(4).zip(b[..n].chunks_exact(4)) {
        let mut worst = 0;
        for c in 0..3 {
            let d = (pa[c] as i32 - pb[c] as i32).abs();
            sum += d as u64;
            worst = worst.max(d);
        }
        if worst > 16 {
            over += 1;
        }
    }
    (sum as f64 / (n / 4 * 3) as f64, over as f64 / (n / 4) as f64)
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut chunks = bytes.chunks_exact(8);
    for c in &mut chunks {
        h ^= u64::from_le_bytes(c.try_into().unwrap());
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    for &b in chunks.remainder() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

pub fn write_png(path: &std::path::Path, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(rgba).map_err(|e| e.to_string())
}

/// zakuro-core's seam, forwarding to the renderer's state.
struct Screens;

impl zakuro_core::native_gfx::NativeScreens for Screens {
    fn compose_gpu(
        &mut self,
        system: &mut System,
        index: usize,
        address: u32,
        emulated: Option<zakuro_gpu::GpuScreen>,
        size: (u32, u32),
        pick: zakuro_core::native_gfx::Pick,
    ) -> Option<zakuro_gpu::GpuScreen> {
        with(|g| {
            let start = std::time::Instant::now();
            let _t = perf::scope(perf::C::Compose); // gfx-perf
            let picture = g.compose_gpu(system, index, address, emulated, size, pick); // gfx-hfr: pick
            g.stats.us_compose += start.elapsed().as_micros() as u64;
            picture
        })
        .flatten()
    }

    fn transferred(&mut self, system: &mut System, output: u32) {
        with(|g| {
            let start = std::time::Instant::now();
            let _t = perf::scope(perf::C::Complete); // gfx-perf
            g.transferred(system, output);
            g.stats.us_complete += start.elapsed().as_micros() as u64;
        });
    }

    // gfx-hfr
    fn newest(&mut self, index: usize, address: u32) -> Option<(u64, Option<zakuro_gpu::Matches>)> {
        with(|g| g.hfr_newest(index, address)).flatten()
    }

    // rtpace
    fn begin_between(&mut self, system: &mut System, index: usize, address: u32, emulated: Option<zakuro_gpu::GpuScreen>, size: (u32, u32), at: f32) -> bool {
        with(|g| g.begin_between(system, index, address, emulated, size, at)).unwrap_or(false)
    }

    // gfx-hfr
    fn compose_at(&mut self, system: &mut System, index: usize, address: u32, pick: zakuro_core::native_gfx::Pick, emulated: &Screen) -> Option<Screen> {
        with(|g| {
            let _t = perf::scope(perf::C::Compose);
            g.compose_at(system, index, address, pick, emulated)
        })
        .flatten()
    }

    fn compose(&mut self, _system: &mut System, index: usize, address: u32, emulated: &Screen) -> Option<Screen> {
        with(|g| {
            let start = std::time::Instant::now();
            let _t = perf::scope(perf::C::Compose); // gfx-perf
            let picture = g.compose(_system, index, address, emulated);
            g.stats.us_compose += start.elapsed().as_micros() as u64;
            picture
        })
        .flatten()
    }
}

/// gfx2d3: dev, FATES_GFX2D3_TRACE=a-b: every textured prim draw of gfx
/// frames a..=b (texture, sampling, blend, depth, the first vertices)
fn gfx2d3_trace(frame: u64, disp: u32, decoded: &[PrimDecoded]) {
    let Some(range) = crate::perf::var("FATES_GFX2D3_TRACE").ok() else { return };
    let mut it = range.split('-').filter_map(|v| v.trim().parse::<u64>().ok());
    let (a, b) = (it.next().unwrap_or(0), it.next().unwrap_or(u64::MAX));
    if frame < a || frame > b {
        return;
    }
    eprintln!("gfx2d3 f{frame} d{disp}: {} states", decoded.len());
    for (i, d) in decoded.iter().enumerate() {
        let d = match d {
            PrimDecoded::Draw(d) => d,
            PrimDecoded::Skip => continue,
            PrimDecoded::Island(k) => {
                eprintln!("gfx2d3 f{frame} d{disp} {i}: island {k:?}");
                continue;
            }
        };
        if d.shaded.is_some() {
            continue;
        }
        if d.post.is_some() {
            eprintln!("gfx2d3 f{frame} d{disp} {i}: post");
            continue;
        }
        let Some((t, s)) = d.textures[0] else { continue };
        let v: Vec<String> = d.vertices.chunks(24).take(4).map(|c| {
            let f = |k: usize| f32::from_le_bytes([c[k], c[k + 1], c[k + 2], c[k + 3]]);
            format!("({:.1},{:.1},{:.2} uv {:.4},{:.4} c{:02X}{:02X}{:02X}{:02X})", f(0), f(4), f(8), f(12), f(16), c[20], c[21], c[22], c[23])
        }).collect();
        eprintln!(
            "gfx2d3 f{frame} d{disp} {i}: tex {:08X} {}x{} fmt {} wrap {}/{} lin {}/{} sharp {} blend {:?} depth {} {} w{} at {} {} {} color {:?} n{} {}",
            t.addr, t.width, t.height, t.format, s.wrap, s.wrap_t, s.linear_mag, s.linear_min, s.sharp,
            (d.blend.enable, d.blend.eq, d.blend.func), d.depth_test, d.depth_func, d.depth_write,
            d.alpha_test, d.alpha_func, d.alpha_ref, d.color, d.indices.len(), v.join(" ")
        );
    }
}

/// gfx2d3: dev, FATES_GFX2D3_TRACE=a-b: the 3D (shaded) draws of a frame,
/// with each texture unit's size, format, filters and levels of detail
fn gfx2d3_trace_shaded(frame: u64, disp: usize, draws: &[Draw]) {
    let Some(range) = crate::perf::var("FATES_GFX2D3_TRACE").ok() else { return };
    let mut it = range.split('-').filter_map(|v| v.trim().parse::<u64>().ok());
    let (a, b) = (it.next().unwrap_or(0), it.next().unwrap_or(u64::MAX));
    if frame < a || frame > b {
        return;
    }
    for (i, d) in draws.iter().enumerate() {
        let Some(sh) = d.shaded.as_deref() else { continue };
        let t: Vec<String> = (0..3).filter_map(|u| sh.textures[u].map(|(t, s)| format!("u{u} {:08X} {}x{} fmt {} wrap {}/{} lin {}/{} mip {:?}", t.addr, t.width, t.height, t.format, s.wrap, s.wrap_t, s.linear_mag, s.linear_min, sh.mips[u]))).collect();
        eprintln!("gfx2d3 f{frame} d{disp} {i}: shaded n{} v{} vp {:?} {:?} {}", sh.indices.len(), sh.vertices.len(), sh.viewport, sh.key, t.join(" | "));
    }
}

/// moflexhd: the movie picture's texels, read a tile at a time
/// (`zakuro_gpu::texture::decode_texture`), are texel for texel what
/// reading each texel with `sample_texel` gave, every format, both ways up.
#[cfg(test)]
mod moflexhd_tests {
    use zakuro_gpu::texture::{decode_texture, sample_texel, TextureFormat};

    #[test]
    fn whole_textures_decode_as_their_texels_do() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let data: Vec<u8> = (0..64 * 32 * 4)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        for raw in 0..16 {
            let format = TextureFormat::from_raw(raw);
            for (w, h) in [(64u32, 32u32), (8, 8), (16, 40), (12, 8)] {
                for flip in [false, true] {
                    let mut fast = vec![0u8; (w * h * 4) as usize];
                    decode_texture(&data, format, w, h, flip, &mut fast);
                    for y in 0..h {
                        for x in 0..w {
                            let row = if flip { h - 1 - y } else { y };
                            let o = ((row * w + x) * 4) as usize;
                            assert_eq!(fast[o..o + 4], sample_texel(&data, format, w, x, y), "{format:?} {w}x{h} flip {flip} at {x},{y}");
                        }
                    }
                }
            }
        }
    }

    /// how long a 1024x1024 movie texture takes either way (--nocapture)
    #[test]
    fn a_movie_texture_decodes_faster() {
        let data: Vec<u8> = (0..1024 * 1024 * 3).map(|i| (i * 7 % 251) as u8).collect();
        let mut out = vec![0u8; 1024 * 1024 * 4];
        let started = std::time::Instant::now();
        for y in 0..1024 {
            for x in 0..1024 {
                let o = ((y * 1024 + x) * 4) as usize;
                out[o..o + 4].copy_from_slice(&sample_texel(&data, TextureFormat::Rgb8, 1024, x, y));
            }
        }
        let each = started.elapsed();
        let mut fast = vec![0u8; 1024 * 1024 * 4];
        let started = std::time::Instant::now();
        decode_texture(&data, TextureFormat::Rgb8, 1024, 1024, false, &mut fast);
        let whole = started.elapsed();
        assert!(fast == out);
        eprintln!("moflexhd: a 1024x1024 RGB8 texture: texel by texel {each:.2?}, a tile at a time {whole:.2?}");
    }

    /// moflexdec: decoded in bands on several threads, a movie texture has
    /// the same texels as decoded whole, both ways up, every size the
    /// player makes and some uneven ones (bands that don't divide evenly)
    #[test]
    fn a_movie_texture_in_bands_is_the_same() {
        let mut seed = 0x0123_4567_89AB_CDEFu64;
        let data: Vec<u8> = (0..1024 * 1024 * 4)
            .map(|_| {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                seed as u8
            })
            .collect();
        for format in [TextureFormat::Rgb8, TextureFormat::Rgba8, TextureFormat::Rgb565] {
            for (w, h) in [(1024u32, 1024u32), (1024, 512), (512, 1024), (1024, 584), (520, 1000), (64, 8)] {
                for flip in [false, true] {
                    let mut whole = vec![0u8; (w * h * 4) as usize];
                    decode_texture(&data, format, w, h, flip, &mut whole);
                    let mut bands = vec![1u8; (w * h * 4) as usize];
                    crate::moflexhd::decode_bands(&data, format, w, h, flip, &mut bands);
                    assert!(whole == bands, "{format:?} {w}x{h} flip {flip}");
                }
            }
        }
        let mut out = vec![0u8; 1024 * 1024 * 4];
        let started = std::time::Instant::now();
        decode_texture(&data, TextureFormat::Rgb8, 1024, 1024, false, &mut out);
        let whole = started.elapsed();
        let started = std::time::Instant::now();
        crate::moflexhd::decode_bands(&data, TextureFormat::Rgb8, 1024, 1024, false, &mut out);
        eprintln!("moflexdec: a 1024x1024 RGB8 texture: whole {whole:.2?}, in bands {:.2?}", started.elapsed());
    }
}

/// moflexhd: the softened copies' box passes, with running sums, give
/// every texel what adding its window up gave (wide2.rs before).
#[cfg(test)]
mod moflexhd_blur_tests {
    fn reference(src: &[u8], dst: &mut [u8], w: usize, h: usize, along_t: bool, r: usize) {
        for y in 0..h {
            for x in 0..w {
                let mut sum = [0u32; 4];
                let mut n = 0u32;
                let (a, b) = if along_t { (y.saturating_sub(r), (y + r).min(h - 1)) } else { (x.saturating_sub(r), (x + r).min(w - 1)) };
                for i in a..=b {
                    let o = if along_t { (i * w + x) * 4 } else { (y * w + i) * 4 };
                    for c in 0..4 {
                        sum[c] += src[o + c] as u32;
                    }
                    n += 1;
                }
                let o = (y * w + x) * 4;
                for c in 0..4 {
                    dst[o + c] = (sum[c] / n) as u8;
                }
            }
        }
    }

    #[test]
    fn running_sums_blur_as_adding_up_did() {
        let mut seed = 0x1234_5678_9ABC_DEF1u64;
        for (w, h) in [(1usize, 1usize), (8, 8), (64, 32), (33, 17), (512, 256), (3, 200), (1024, 512), (520, 1009)] {
            let src: Vec<u8> = (0..w * h * 4)
                .map(|_| {
                    seed ^= seed << 13;
                    seed ^= seed >> 7;
                    seed ^= seed << 17;
                    seed as u8
                })
                .collect();
            for along_t in [true, false] {
                for r in [1usize, 2, 4, 10, 42, 300] {
                    let (mut a, mut b) = (vec![0u8; w * h * 4], vec![0u8; w * h * 4]);
                    reference(&src, &mut a, w, h, along_t, r);
                    crate::wide2::box_pass(&src, &mut b, w, h, along_t, r);
                    assert!(a == b, "{w}x{h} along_t {along_t} r {r}");
                }
            }
        }
    }

    /// how long softening a 1024x1024 movie picture takes (--nocapture)
    #[test]
    fn a_large_picture_softens_quickly() {
        let (w, h) = (1024usize, 1024usize);
        let src: Vec<u8> = (0..w * h * 4).map(|i| (i * 13 % 253) as u8).collect();
        let (mut a, mut b) = (vec![0u8; w * h * 4], vec![0u8; w * h * 4]);
        let started = std::time::Instant::now();
        reference(&src, &mut a, w, h, true, h / 24);
        let old = started.elapsed();
        let started = std::time::Instant::now();
        crate::wide2::box_pass(&src, &mut b, w, h, true, h / 24);
        let new = started.elapsed();
        assert!(a == b);
        eprintln!("moflexhd: one pass down a 1024x1024 picture, radius {}: adding up {old:.2?}, running sums {new:.2?}", h / 24);
    }
}
