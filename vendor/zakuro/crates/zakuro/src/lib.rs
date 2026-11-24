//! Zakuro's frontend, a window showing the library of games or a game with
//! a menu over it, and the loop that drives the emulated console one frame
//! at a time.

mod audio;
// bottom: the bottom screen on a single PC display, and a scripted check of it.
mod bottom_check;
mod bottom_scene; // bottom2
mod bottom_screen;
mod camera;
// cameras: the battle free camera and walking in My Castle's view
pub mod cameras;
pub mod castlecam; // castlecam: the castle view's turning camera
mod cli;
// ui: the game's own interface (options menu, keyboard, notices)
mod fates_ui;
mod gamepad;
mod gui;
pub mod haiku_mods; // mods: Haiku's mod manager
pub mod haiku_codemods; // codemods: code patches recompiled, native plugins
pub mod haiku_builtin; // sunglasses: Haiku's own RomFS layer (assets/builtin)
pub mod haiku_voices; // jpvoice: the Japanese voice layer (data/voices-ja)
pub mod haiku_setup_entry; // oneexe: Options > Interface > Haiku Setup
pub mod haiku_versions; // versions: Options > Interface > Game Version
pub mod haiku_import; // hdpacks: downloads (split, zips of zips) unpacked into mod folders
pub mod haiku_fanpatch; // fanpatch: mods for other releases (Team If), Haiku's text under mods'
pub mod haiku_modlayout; // modfix: mods laid out as players get them (loose RomFS folders, title folders deeper down, a choice of code patches)
pub mod jpn_english; // jpn2: Haiku's lines and menus in English under a translation on the Japanese game
pub mod pregame; // updscreen: work before the game's code loads, shown in the game's window
pub mod haiku_update; // updater: Haiku updating itself from its GitHub releases
mod ed25519; // updater: the releases' signatures
// hfr: high frame rates, display modes and pacing
pub mod display;
pub mod hfr;
mod input;
// input: SDL3 controllers, bindings, glyphs, public input API
pub mod controls;
mod glyphs;
pub mod input_api;
mod input_script;
mod pads;
mod playtest; // playtest
mod sdl;
mod library;
mod menus;
mod offline_msg; // offline-msg: a click closes the not-available message
mod online_ui; // online: the host's code, the code box, the session's news
// cpu: native-only execution, headless checkpoints and frame times
pub mod native_cpu;
mod vowperf_sample; // vowperf: HAIKU_SAMPLE, a sampling profiler for headless runs
mod hitches; // hitches (issue #14): the menus' lists read on threads of their own, slow pictures logged
mod present;
mod recompile;
mod report;
mod settings;
// wide: the top screen's shape from pc.aspect
pub mod qol; // qol: game speed, Skip Story, the SpotPass questions, the boon/bane panel
pub mod qol4; // qol4: Options in the Main Menu, the soft reset, the map menu's items
mod boot3; // boot3: the launch window and the taskbar, hangs at start and at Return to Title
pub mod info_ui; // info-ui: enemy targeting lines, growth rates
pub mod cheats; // cheats: the Options menu's Cheats tab
pub mod pulse; // pulse: Divine Pulse, the rewind (Gameplay)
mod pulse_fx; // pulse2: its effects on the picture
pub mod states; // pulse: save states (Cheats > Save States)
pub mod statecheck; // statecheck: a state remembers the file layer it was made with
mod trailer_dump; // trailer: dev-only frame dump switches for the trailer pipeline
mod wide;
mod zig;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

use zakuro_common::Screen;
use zakuro_core::{loader, Config, FrameOutcome, System};
use zakuro_gpu::{layout, GpuScreen, Overlay, PresentError, RendererKind, ScreenImage};

use gui::Gui;
use input::Keyboard;
use library::Library;
use menus::{Action, Menus};
use present::Backend;
use recompile::{Job, Stage, Toolchain, Wanted};
use settings::{CameraSource, Renderer, Screens, Settings};

pub use zakuro_core::memory::Memory;
pub use zakuro_core::recompiled::Linked;
/// wide: the top screen's aspect the GPU draws at, for a game's camera
/// code to follow (fates-pc hands it to overrides/widescreen.c).
pub use zakuro_gpu::wide::effective_aspect_c as wide_aspect_hook;

use crate::settings::Orientation;

/// what a program built on Zakuro shows over its game, worked out from the
/// game's memory: a line kept in a corner, and a notice shown for a moment.
/// a recompiled game can show what the game keeps hidden this way, its
/// alignment, a timer, without drawing into the game's own screens.
pub trait GameOverlay {
    /// called after each frame the game runs, with its memory.
    fn frame(&mut self, memory: &mut Memory) -> OverlayText;
}

/// what a GameOverlay shows after a frame.
#[derive(Default, Clone, Debug, PartialEq, Eq)]
pub struct OverlayText {
    /// a line in the bottom right corner while it is some.
    pub corner: Option<String>,
    /// a notice at the top for a moment, as a saved state's.
    pub notice: Option<String>,
}

/// runs the emulator as the command line says, on recompiled code linked
/// into the program when there is some.
/// gfx-hfr: the saved `pc.native_graphics` setting (the renderer the game
/// starts with), for the executable to read before it installs the
/// native renderer.
pub fn native_graphics_setting() -> bool {
    // native-final: the pure build has only the native renderer
    if !cfg!(feature = "emulation") {
        return true;
    }
    settings::Settings::load().pc.native_graphics
}

pub fn run(linked: Option<Linked>) {
    run_with(linked, None)
}

/// runs the emulator as run does, with an overlay over the game.
pub fn run_with(linked: Option<Linked>, overlay: Option<Box<dyn GameOverlay>>) {
    run_inner(linked, overlay, None)
}

/// updscreen: opens the window as run does, shows the work there in the
/// game's look while it is done (fates_ui/pregame.rs), then plays the code
/// it gives, in the same window. A window is needed: headless runs stop.
pub fn run_after(work: Box<dyn pregame::Pregame>) {
    run_inner(None, None, Some(work))
}

fn run_inner(linked: Option<Linked>, overlay: Option<Box<dyn GameOverlay>>, pregame: Option<Box<dyn pregame::Pregame>>) {
    report::init();

    let options = match cli::parse() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("zakuro: {message}");
            std::process::exit(2);
        }
    };
    let mut settings = Settings::load();
    fates_ui::ja::set_english(settings.pc.japanese_english_menus); // jpprep
    // input: settings from before the PC control scheme move to it
    if controls::migrate(&mut settings.keys, &mut settings.pad) {
        settings.save();
    }
    cameras::configure(&settings.pc); // cameras
    zakuro_core::native_gfx::set_msaa(settings.pc.msaa); // msaa: the native renderer's samples
    // the console's own number, made the first time and kept, which tells
    // it apart from others over local wireless
    if settings.console_id == 0 {
        use std::hash::{BuildHasher, Hasher};
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |time| time.as_nanos()));
        settings.console_id = hasher.finish() | 1;
        settings.save();
    }

    let data_dir = options.data.clone().map(PathBuf::from).or_else(default_data_dir);
    if let Some(dir) = &data_dir {
        report::to_file(dir);
        bring_saves(dir);
    }

    let mut app = App {
        keyboard: Keyboard::new(settings.keys.clone()),
        bottom: bottom_screen::BottomScreen::new(),
        bottom_check: bottom_check::Script::from_env(),
        gamepads: {
            // input: SDL3 (gilrs if SDL3 is missing), and the public API's settings
            let pads = pads::Pads::new(settings.pad.clone(), settings.pc.button_layout, settings.pc.stick_deadzone);
            input_api::configure(settings.pc.rumble, settings.pc.glyphs, settings.pc.button_layout, &settings.keys, &settings.pad);
            pads
        },
        glyphs: glyphs::Glyphs::default(),
        scale: options.scale.unwrap_or(settings.scale).max(1),
        layout: settings.layout,
        orientation: settings.orientation,
        top_shape: settings.top_shape,
        options,
        active: settings.clone(),
        settings,
        linked,
        overlay,
        overlay_corner: None,
        library: Library::new(data_dir.clone()),
        data_dir,
        game: None,
        menus: {
            let mut menus = Menus::default();
            menus.slot = 1;
            menus
        },
        jobs: Vec::new(),
        waiting: Vec::new(),
        download_compiler: false,
        gui: None,
        backend: None,
        window: None,
        audio: None,
        mouse_down: false,
        offline_click: Default::default(), // offline-msg
        tilting_from: None,
        cursor: (0.0, 0.0),
        last_title_update: Instant::now(),
        next_frame: Instant::now(),
        skipped: 0,
        shown: 0,
        spent: Spent::default(),
        paused: false,
        fast_forward_key: false,
        fast_forwarding: false,
        next_shown: Instant::now(),
        stop: false,
        camera: None,
        room_server: None,
        room: None,
        dlp_guest: None,
        dlp_own: None,
        wireless_reboot: None,
        maximized_before_full_screen: false,
        resize_after_full_screen: false,
        hfr: hfr::Pacer::default(),
        fates: fates_ui::FatesUi::default(),
        qol: Default::default(), // qol
        pregame: pregame.map(fates_ui::pregame::Screen::new), // updscreen
    };

    // native-rt: deterministic or real-time scheduling for the session
    zakuro_core::pcos::rt::choose(app.options.headless.is_some());
    if app.options.headless.is_some() {
        zakuro_core::pcos::threads::background(); // quiet: tests never slow a game being played
    }
    // updscreen: the work before the game needs the window
    if app.pregame.is_some() && app.options.headless.is_some() {
        eprintln!("zakuro: the game's code has to be made first, which a headless run doesn't do");
        std::process::exit(3);
    }
    if let Some(mut frames) = app.options.headless {
        hfr::apply_game_timing(&app.settings.pc); // hfr
        qol::apply(&app.settings.pc); // qol
        zakuro_core::randomizer::set_seed_setting(zakuro_core::randomizer::parse_seed(&app.settings.pc.randomizer_seed)); // randomizer
        zakuro_core::supports::set_settings(app.settings.pc.corrin_s_any, app.settings.pc.kana_gender.core()); // supports
        info_ui::apply(&app.settings.pc); // info-ui
        cheats::apply(&app.settings.pc); // cheats
        pulse::apply(&app.settings.pc); // pulse
        online_ui::configure(&app.settings.pc.online); // online2: the host's rule
        haiku_update::start(app.settings.pc.check_updates, true); // updater: only with HAIKU_UPDATE_CHECK=1
        app.start();
        // mods: FATES_MODS_RESTART_AT=n runs n frames, then restarts the game
        // in place as the Mods page's Apply does, and runs the rest
        if let Some(at) = std::env::var("FATES_MODS_RESTART_AT").ok().and_then(|at| at.parse::<u64>().ok()).filter(|at| *at < frames) {
            if let Some(game) = &mut app.game {
                run_headless(game, at, |_| {});
            }
            // FATES_MODS_RESTART_ENABLE=id,id turns those mods on (the rest
            // off) first, as the player would on the Mods page
            if let Ok(ids) = std::env::var("FATES_MODS_RESTART_ENABLE") {
                app.fates_mods_test_enable(&ids);
            }
            app.fates_mods_restart();
            frames -= at;
        }
        // input: FATES_INPUT_SCRIPT drives the keyboard/controller mapping
        let mut script = input_script::Script::open(&app.settings);
        cameras::fixed_step(); // cameras: a poll a game frame
        let mut camera_script = cameras::HeadlessScript::open(); // cameras: FATES_CAMERAS_SCRIPT
        let mut offline_click = offline_msg::ClickConfirm::default(); // offline-msg
        let mut live = online_ui::Live::from_env(); // online: FATES_LIVE_INPUT
        let (data_dir, packs) = (app.data_dir.clone(), app.active.texture_packs);
        let mut speed_settings = app.settings.pc.clone(); // qol
        // qol4: the game's soft reset starts it again between two stretches
        loop {
        let glyph_set = &mut app.glyphs;
        if let Some(game) = &mut app.game {
            // mods: the mods' texture packs, under the game's own button pictures
            if !glyph_set.mod_textures.is_empty() {
                glyph_set.apply(glyphs::forced().unwrap_or(controls::GlyphSet::Nintendo), &mut game.system, data_dir.as_deref(), packs);
            }
            run_headless(game, frames, |game| {
                // online: the session's news, the test's code, live input
                online_ui::poll();
                if let Some(live) = &mut live {
                    let input = live.frame(&mut game.system, game.frame);
                    game.system.set_input(input);
                }
                // qol: Skip Story's cover, as a window would have it
                qol::headless_frame(&mut game.system.memory, game.frame, &speed_settings);
                cheats::headless_frame(&mut game.system, game.frame); // cheats
                zakuro_core::tones::frame(&mut game.system.memory); // tones: a speaking style chosen in the options
                if qol::read(qol::Word::QuitRequest).unwrap_or(0) != 0 {
                    qol::write(qol::Word::QuitRequest, 0);
                    println!("qol: frame {} Exit to Desktop was confirmed (a window closes here)", game.frame);
                }
                // qol3: the title Main Menu's Exit to Desktop (a window asks here)
                if qol::read(qol::Word::TitleExitRequest).unwrap_or(0) != 0 {
                    qol::write(qol::Word::TitleExitRequest, 0);
                    println!("qol3: frame {} Exit to Desktop was chosen in the Main Menu (a window asks Yes/No here)", game.frame);
                }
                qol4::headless_frame(game.frame); // qol4: FATES_QOL4_RESET_AT, Return to Title at a frame
                // qol4: the title Main Menu's Options (a window opens them here)
                if qol::read(qol::Word::TitleOptionsRequest).unwrap_or(0) != 0 {
                    qol::write(qol::Word::TitleOptionsRequest, 0);
                    println!("qol4: frame {} Options was chosen in the Main Menu (a window opens the options here)", game.frame);
                }
                if let Some(script) = &mut script {
                    let mut input = script.input(game.frame);
                    // offline-msg: the script's clicks, as the window's
                    if script.take_click() {
                        offline_click.click(&game.system);
                    }
                    offline_click.apply(&mut input);
                    cameras::mask_game_input(&mut input); // cameras: the fly camera's moves
                    info_ui::frame_input(&mut game.system.memory, game.frame, &mut input, true); // info-ui
                    game.system.set_input(input);
                    cameras::poll(0.0); // cameras: the script's stick and keys
                    // info-ui: the growth-rates key, as the window takes it
                    if input_api::take_action(controls::PcAction::GrowthRates) {
                        let shown = info_ui::with_state(|state| {
                            state.toggle();
                            state.view_on
                        });
                        println!("info-ui: frame {} the growth key: growth rates {}", game.frame, if shown { "shown" } else { "hidden" });
                    }
                    let set = glyphs::forced().unwrap_or_else(input_api::glyph_set);
                    glyph_set.apply(set, &mut game.system, data_dir.as_deref(), packs);
                    // qol: the Game Speed key, as the window takes it (the
                    // speed itself means nothing without a wall clock)
                    if input_api::take_action(controls::PcAction::FastForward) {
                        let speed = qol::toggle_speed(&mut speed_settings);
                        println!("qol: frame {} the Game Speed key: {}", game.frame, qol::speed_name(speed));
                    }
                }
                if let Some(camera_script) = &mut camera_script {
                    camera_script.frame(game.frame, script.is_some()); // cameras
                }
            });
        }
        // qol4: the run stopped for the game's soft reset: started again,
        // the rest of the frames run on
        if !qol4::restart_due() {
            break;
        }
        app.qol4_restart_headless();
        }
        return;
    }

    if let Some(folder) = app.settings.games.clone() {
        app.library.scan(&folder);
    }
    app.audio = if app.options.mute {
        None
    } else {
        // ui: the output device the options chose
        audio::Audio::open_on(zakuro_core::AUDIO_SAMPLE_RATE, &app.settings.pc.audio_device).inspect_err(|error| log::warn!("no sound, {error}")).ok()
    };
    app.apply_volume();

    let event_loop = EventLoop::new().expect("create an event loop");
    event_loop.set_control_flow(ControlFlow::Poll);
    if let Err(error) = event_loop.run_app(&mut app) {
        eprintln!("zakuro: {error}");
    }
    zakuro_core::netplay::shutdown(); // online: the other player is told at once
    // native-rt: FATES_SCHED_STATS=1, the scheduling counts of the session
    if zakuro_core::pcos::rt::stats_wanted() {
        let frames = app.game.as_ref().map_or(0, |game| game.system.frames);
        for line in zakuro_core::pcos::rt::summary(frames) {
            eprintln!("{line}");
        }
    }
    zakuro_gpu::smooth::flush(); // smooth: the timing log's last lines
}

/// opens a folder in the system's file manager.
fn show_folder(folder: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    match std::process::Command::new(program).arg(folder).spawn() {
        // waited for so it leaves nothing behind when it closes
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(error) => log::warn!("showing {}: {error}", folder.display()),
    }
}

/// the system's place for a program's data.
fn default_data_dir() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    let dir = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Application Support"))
    } else {
        var("XDG_DATA_HOME").or_else(|| var("HOME").map(|home| home.join(".local/share")))
    };
    dir.map(|dir| dir.join("haiku"))
}

/// how long ago a state was saved, as a person would say it.
fn ago(time: std::time::SystemTime) -> String {
    let seconds = time.elapsed().map_or(0, |elapsed| elapsed.as_secs());
    match seconds {
        0..60 => "saved just now".into(),
        60..120 => "saved a minute ago".into(),
        120..3600 => format!("saved {} minutes ago", seconds / 60),
        3600..7200 => "saved an hour ago".into(),
        7200..86400 => format!("saved {} hours ago", seconds / 3600),
        86400..172800 => "saved yesterday".into(),
        _ => format!("saved {} days ago", seconds / 86400),
    }
}

/// saves Zakuro kept in the working directory before it had a place for
/// them, copied there the first time. the old ones stay as they were.
fn bring_saves(data: &Path) {
    let (old, new) = (Path::new("user"), data.join("user"));
    if new.exists() || !old.is_dir() {
        return;
    }
    // a copy cut short is left under another name, and tried again
    let partial = data.join("user.copying");
    let copied = std::fs::remove_dir_all(&partial)
        .or_else(|error| if error.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(error) })
        .and_then(|()| copy_dir(old, &partial))
        .and_then(|()| std::fs::rename(&partial, &new));
    match copied {
        Ok(()) => log::info!("copied the saves in ./user to {}, the old ones stay as they were", new.display()),
        Err(error) => log::warn!("could not copy the saves in ./user to {}, {error}", new.display()),
    }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// builds a System with no title loaded and both screens filled with a
/// solid color, to check the presentation path (framebuffer config -> guest
/// memory -> window) in isolation from any title's boot logic, and that the
/// GPU starts, which a packed release is tried with.
fn paint_test_pattern(config: Config) -> System {
    use zakuro_core::memory::{MemoryRegion, MemoryState, Permission};

    let mut system = System::new(config);
    #[cfg(feature = "emulation")] // native-final
    if system.config.hardware_renderer {
        match system.gpu.enable_hardware_renderer(system.config.resolution, system.config.device.clone(), None) {
            Ok(name) => log::info!("the 3D draws on {name}"),
            Err(error) => log::warn!("the 3D can't draw on the GPU, {error}"),
        }
    }

    // ABGR in memory for format 0 (Rgba8), red = 00 00 00 FF?
    let paint = |system: &mut System, screen: Screen, rgba: [u8; 4]| {
        let width = screen.width();
        let height = screen.height();
        let stride = height * 4;
        let size = stride * width;

        let block = system
            .memory
            .phys
            .allocate(MemoryRegion::Base, size)
            .expect("allocate test pattern framebuffer");

        let vaddr = zakuro_core::services::gsp::physical_to_virtual(system, block.addr);
        system.memory.map(
            vaddr,
            block.addr,
            size,
            Permission::READ | Permission::WRITE,
            MemoryState::Shared,
        );

        let mut pixel = [0u8; 4];
        pixel[3] = rgba[0]; // r
        pixel[2] = rgba[1]; // g
        pixel[1] = rgba[2]; // b
        pixel[0] = rgba[3]; // a
        let mut data = Vec::with_capacity(size as usize);
        for _ in 0..(size / 4) {
            data.extend_from_slice(&pixel);
        }
        system.memory.write_physical(block.addr, &data);

        let index = match screen {
            Screen::Top => 0,
            Screen::Bottom => 1,
        };
        system.set_framebuffer(index, 0, block.addr, block.addr, stride, 0);
    };

    paint(&mut system, Screen::Top, [220, 40, 40, 255]); // red
    paint(&mut system, Screen::Bottom, [40, 90, 220, 255]); // blue
    system
}

/// runs frames without a window. ZAKURO_REPLAY's inputs are fed in when
/// set, and ZAKURO_DUMP_DIR with ZAKURO_DUMP_FRAMES (a comma list of frame
/// numbers, or "every:N") writes each screen as top-NNNNN.png and
/// bottom-NNNNN.png, for checking what the game shows without a display.
fn run_headless(game: &mut Running, frames: u64, mut each_frame: impl FnMut(&mut Running)) {
    let dump_dir = std::env::var_os("ZAKURO_DUMP_DIR").map(PathBuf::from);
    let dump_spec = std::env::var("ZAKURO_DUMP_FRAMES").unwrap_or_default();
    let every = dump_spec.strip_prefix("every:").and_then(|n| n.trim().parse::<u64>().ok());
    let listed: Vec<u64> = dump_spec.split(',').filter_map(|n| n.trim().parse().ok()).collect();
    // trailer: ZAKURO_DUMP_FRAMES=range:A-B dumps every frame from A to B
    let range = trailer_dump::range(&dump_spec);
    if let Some(dir) = &dump_dir {
        let _ = std::fs::create_dir_all(dir);
    }
    // hfr: ZAKURO_HFR_LAYERS=n draws n pictures in between each frame, for
    // measuring what they cost and dumping them (as top-NNNNN-bK.png)
    let layers: usize = std::env::var("ZAKURO_HFR_LAYERS").ok().and_then(|n| n.parse().ok()).unwrap_or(0);
    if layers > 0 {
        let on = game.system.set_interpolation(true);
        let fractions: Vec<f32> = (0..layers).map(|k| k as f32 / layers as f32).collect();
        game.system.set_between(&fractions);
        println!("hfr: {layers} pictures in between a frame, interpolation {on}");
    }
    // cpu: FATES_LOAD goes on from a saved state, frames counted from its
    // (qol4: after the game's soft reset, from where it was)
    let first = if qol4::take_resuming() { game.frame } else { native_cpu::headless_load(&mut game.system).unwrap_or(0) };
    // gfx-models: a loaded state starts the pacing over; the fractions asked
    // for go in again, else only a game at every other refresh got any
    if layers > 0 && first > 0 {
        let fractions: Vec<f32> = (0..layers).map(|k| k as f32 / layers as f32).collect();
        game.system.set_between(&fractions);
    }
    game.frame = first;
    let save_points = native_cpu::save_points();
    let mut proc_log = bottom_scene::ProcLog::from_env(); // bottom2
    let mut playtest = playtest::Playtest::from_env(); // playtest: FATES_PEEK/POKE/UNITS
    let mut times = Vec::with_capacity(frames as usize);
    let start = Instant::now();
    let mut outcome = FrameOutcome::Completed;
    let mut ran = first;
    vowperf_sample::start(game.system.recompiled.as_ref().map(|library| library.code_points()).unwrap_or_default()); // vowperf
    zakuro_core::moflexhd::on(); // moflexhd: FATES_MOVIE_STATS read before any movie
    // native-rt: the real-time schedule, started over after a stall
    let (mut rt_start, mut rt_from) = (start, first);
    // savehang: FATES_HEADLESS_SPEED=1 runs Skip Story's skips at Max in
    // real time, as a window does (qol), for tests of what follows them
    let skip_speed = std::env::var_os("FATES_HEADLESS_SPEED").is_some();
    let mut speed = 1u32;
    let lend_cap = std::env::var("FATES_HEADLESS_LEND_MS").ok().and_then(|v| v.parse::<f64>().ok()).map(|ms| Duration::from_secs_f64(ms / 1000.0));
    for _ in first..frames.max(first) {
        // qol4: the game asked for its soft reset: the caller starts it again
        if qol4::restart_due() {
            break;
        }
        if let Some(replay) = &mut game.replay {
            let mut input = replay.input(game.frame);
            info_ui::frame_input(&mut game.system.memory, game.frame, &mut input, true); // info-ui: the growth button's touches
            game.system.set_input(input);
        }
        if let Some(tools) = &mut playtest {
            tools.before(&mut game.system.memory, ran + 1); // playtest
        }
        // pulse: FATES_PULSE_AT and FATES_STATES_AT; while the rewind
        // screen holds the game, no frame of it runs
        pulse::headless_frame(&mut game.system, ran + 1);
        if states::headless_frame(&mut game.system, ran + 1) {
            game.frame = game.system.frames;
        }
        if pulse::holds() {
            ran += 1;
            continue;
        }
        each_frame(game);
        game.frame += 1;
        zakuro_core::native_gfx::HEADLESS_FRAME.store(ran + 1, std::sync::atomic::Ordering::Relaxed); // wide2: trace key
        // native-rt: in real time (FATES_SCHED=realtime) the threads run
        // until the refresh's time on the wall clock, which run_frame raises
        if zakuro_core::pcos::rt::realtime() {
            // savehang: the speed Skip Story's skip runs at, as qol_poll sets it
            let wanted = if skip_speed && qol::headless_covered() { qol::MAX_SPEED } else { 1 };
            if wanted != speed {
                speed = wanted;
                zakuro_core::pcos::rt::set_speed(speed as f64);
                (rt_start, rt_from) = (Instant::now(), ran);
                println!("savehang: frame {ran}: the game runs at {speed}x");
            }
            let refresh = (ran - rt_from + 1) as u128 * zakuro_core::CYCLES_PER_FRAME as u128 * 1_000_000_000 / zakuro_core::kernel::thread::CPU_CLOCK_HZ as u128 / speed as u128;
            let due = rt_start + Duration::from_nanos(refresh as u64);
            // far behind (a stall), the schedule starts over, as a window's does
            if Instant::now() > due + CATCH_UP_LIMIT {
                (rt_start, rt_from) = (Instant::now(), ran);
            }
            // savehang: FATES_HEADLESS_LEND_MS=n lends the threads at most n ms
            // of each refresh, the rest slept, as a window that falls behind
            // (its pictures taking the time) lends them little
            match lend_cap {
                Some(cap) => {
                    game.system.lend_until(due.min(Instant::now() + cap));
                    if let Some(rest) = due.checked_duration_since(Instant::now()) {
                        std::thread::sleep(rest);
                    }
                }
                None => game.system.lend_until(due),
            }
        }
        let frame_start = Instant::now();
        pulse::before_frame(&mut game.system); // pulse: a moment shown, a rewind made
        outcome = game.system.run_frame();
        times.push(frame_start.elapsed());
        ran += 1;
        vowperf_sample::FRAME.store(ran, std::sync::atomic::Ordering::Relaxed); // vowperf
        // pulse: the moments worth keeping; after a moment was loaded the
        // replay goes on from its frame, so the same inputs play again
        pulse::after_frame(&mut game.system, ran);
        if pulse::take_loaded() && pulse::replays_again(true) {
            game.frame = game.system.frames;
        }
        if let Some(clock) = &mut game.clock {
            clock.frame(&mut game.system.memory); // hfr
        }
        if let Some(log) = &mut proc_log {
            log.frame(&mut game.system.memory, ran); // bottom2
        }
        if let Some(tools) = &mut playtest {
            tools.after(&mut game.system.memory, ran); // playtest
        }
        native_cpu::headless_save(&mut game.system, ran, &save_points);
        let due = every.is_some_and(|n| n > 0 && ran % n == 0) || listed.contains(&ran) || ran == frames || range.as_ref().is_some_and(|r| r.contains(ran)); // trailer: range
        // gfx: with the native renderer on, take the screens every frame as
        // a window would, so each frame is composed
        if !(due && dump_dir.is_some()) && zakuro_core::native_gfx::active() && zakuro_core::native_gfx::every_screen() {
            for screen in [Screen::Top, Screen::Bottom] {
                let _ = game.system.read_screen_scaled(screen);
            }
        }
        if let Some(dir) = &dump_dir {
            if due {
                if layers > 0 {
                    println!("hfr: frame {ran} pictures in between at {:?}, draws matched {:?}, gap {:?}", game.system.between_ready(), game.system.interpolation_matches(), game.system.picture_gap()); // gfx-models: gap
                }
                for layer in 1..=game.system.between_ready().len() {
                    if let Some((pixels, width, height)) = game.system.upright_between(Screen::Top, layer) {
                        let _ = trailer_dump::save(&dir.join(format!("top-{ran:05}-b{layer}.png")), &pixels, width, height); // trailer: fast PNG when asked
                    }
                    // gfx-hfr: the native renderer's lower screen's too
                    if zakuro_core::native_gfx::active() && trailer_dump::wants_bottom() {
                        if let Some((pixels, width, height)) = game.system.upright_between(Screen::Bottom, layer) {
                            let _ = image::save_buffer(dir.join(format!("bottom-{ran:05}-b{layer}.png")), &pixels, width, height, image::ColorType::Rgba8);
                        }
                    }
                }
                // qol: FATES_QOL_SEEN=1: a frame Skip Story covers is written black, as seen
                let covered = std::env::var_os("FATES_QOL_SEEN").is_some() && qol::headless_covered();
                for (screen, name) in [(Screen::Top, "top"), (Screen::Bottom, "bottom")] {
                    if screen == Screen::Bottom && !trailer_dump::wants_bottom() {
                        continue; // trailer: FATES_DUMP_SCREENS=top
                    }
                    let (pixels, width, height) = match layers > 0 {
                        // hfr: the pictures stay on the GPU then
                        true => game.system.upright_screen(screen, false).unwrap_or_else(|| game.system.read_screen_scaled(screen)),
                        false => game.system.read_screen_scaled(screen),
                    };
                    let pixels = if covered { std::sync::Arc::new(pixels.iter().enumerate().map(|(i, v)| if i % 4 == 3 { *v } else { 0 }).collect::<Vec<u8>>()) } else { pixels }; // qol
                    let path = dir.join(format!("{name}-{ran:05}.png"));
                    if let Err(error) = trailer_dump::save(&path, &pixels, width, height) {
                        eprintln!("could not write {}: {error}", path.display());
                    }
                }
            }
        }
        if outcome != FrameOutcome::Completed {
            break;
        }
    }
    zakuro_core::netplay::shutdown(); // online: the other player is told at once
    // ui: screenshots of the interface over the last frame, for checking it
    if let Some(dir) = std::env::var_os("FATES_UI_SNAPSHOT") {
        fates_ui::snapshot::run(std::path::Path::new(&dir), &mut game.system);
    }
    let system = &mut game.system;
    let elapsed = start.elapsed();
    println!("{outcome:?} after {ran} frames in {elapsed:.2?}");
    if let Some(line) = qol::summary() {
        println!("{line}"); // qol
    }
    if let Some(line) = cheats::summary() {
        println!("{line}"); // cheats
    }
    if let Some(line) = info_ui::summary() {
        println!("{line}"); // info-ui
    }
    if let Some(line) = pulse::summary() {
        println!("{line}"); // pulse
    }
    zakuro_core::moflexhd::movie_end(); // moflexdec: a movie still under way, said
    if let Some(line) = zakuro_core::moflexhd::summary() {
        println!("{line}"); // moflexhd
    }
    println!(
        "{:.1} MIPS, {}",
        system.cpu.cycles as f64 / elapsed.as_secs_f64() / 1e6,
        system.status_line()
    );
    if !system.fatal_errors.is_empty() {
        println!("fatal errors: {}", system.fatal_errors.join("; "));
    }
    // hfr: where the GPU's time outside draws went
    let [transfers, copies, fills, syncs, pacing] = system.hfr_spent;
    println!("hfr: display transfers {transfers:.2?}, texture copies {copies:.2?}, fills {fills:.2?}, syncs {syncs:.2?}, pacing {pacing:.2?}");
    println!("{}", zakuro_core::pcos::snd::summary(system)); // audio
    println!("{}", system.host_threads.summary()); // os2
    // native-rt: why runs ended and the baton changed hands, cpu time
    for line in zakuro_core::pcos::rt::summary(ran - first) {
        println!("{line}");
    }
    // gfx-final: what the emulated GPU still did with a native renderer on
    if zakuro_core::native_gfx::active() {
        println!("gfx-emulated: {:?}", zakuro_core::native_gfx::emulated_work());
    }
    // native-final: everything emulated the session reached (all 0: none)
    println!("{}", zakuro_core::pcos::emu::summary(system.interpreted_this_run()));
    for (thread, pc, hits) in system.hot_spots(10) {
        println!("  hot {thread:<8} 0x{pc:08X} {hits}");
    }
    // cpu: frame times, where the interpreter ran, the dispatch's cost
    native_cpu::frame_log(first, &times); // vowperf: HAIKU_FRAME_LOG
    vowperf_sample::report(); // vowperf: HAIKU_SAMPLE
    println!("{}", native_cpu::frame_times(&mut times));
    for line in system.cpu_summary() {
        println!("{line}");
    }
    native_cpu::dump_text(system);
}

/// a game being played.
struct Running {
    system: System,
    path: PathBuf,
    name: String,
    /// the inputs written down, ZAKURO_RECORD=file, for playing the run
    /// back.
    recorder: Option<zakuro_core::replay::Recorder>,
    /// the inputs a recording wrote down, ZAKURO_REPLAY=file, played back in
    /// place of the keyboard's and the controllers', and the frames run.
    replay: Option<zakuro_core::replay::Replay>,
    frame: u64,
    /// frames run since counting_since, for the frame rate.
    frames: u32,
    counting_since: Instant,
    fps: f32,
    // hfr: the game's clock against the wall's, ZAKURO_HFR_LOG
    clock: Option<hfr::ClockLog>,
}

impl Running {
    fn new(system: System, path: PathBuf, name: String) -> Running {
        Running {
            system,
            path,
            name,
            recorder: None,
            replay: None,
            frame: 0,
            frames: 0,
            counting_since: Instant::now(),
            fps: 0.0,
            clock: hfr::ClockLog::from_env(),
        }
    }

    fn count_frame(&mut self) {
        self.frames += 1;
        let elapsed = self.counting_since.elapsed();
        if elapsed >= Duration::from_millis(500) {
            self.fps = self.frames as f32 / elapsed.as_secs_f32();
            self.frames = 0;
            self.counting_since = Instant::now();
        }
    }
}

struct App {
    options: cli::Options,
    /// the settings as the user left them, which get saved.
    settings: Settings,
    /// the settings in force, what the running game has of its own over
    /// the general ones, which is all there is without a game.
    active: Settings,
    linked: Option<Linked>,
    /// what the program built on Zakuro shows over the game, and the line it
    /// keeps in the corner now.
    overlay: Option<Box<dyn GameOverlay>>,
    overlay_corner: Option<String>,
    data_dir: Option<PathBuf>,
    game: Option<Running>,
    library: Library,
    menus: Menus,
    jobs: Vec<Job>,
    /// the games waiting for their turn to recompile, the next one first.
    waiting: Vec<Wanted>,
    /// the player said yes to downloading a compiler for the next recompile.
    download_compiler: bool,
    gui: Option<Gui>,
    // backend must be declared (and therefore dropped) before window, Rust
    // drops struct fields in declaration order, and the GL surface's Drop
    // calls eglDestroySurface, which on Wayland does a protocol round-trip
    // against the window's wl_surface.
    backend: Option<Backend>,
    window: Option<Window>,
    keyboard: Keyboard,
    gamepads: pads::Pads,
    /// input: the controller glyph texture set drawn in the game.
    glyphs: glyphs::Glyphs,
    // bottom: the bottom screen's panel, its touch cursor and its art.
    bottom: bottom_screen::BottomScreen,
    bottom_check: Option<bottom_check::Script>,
    audio: Option<audio::Audio>,
    /// the window's size, times the console's.
    scale: u32,
    /// how the screens are arranged.
    layout: Screens,
    orientation: Orientation,
    /// how wide the top screen shows.
    top_shape: settings::TopShape,
    /// the left button is down, and where the pointer is, in window pixels.
    mouse_down: bool,
    /// offline-msg: a click that confirms the not-available message
    offline_click: offline_msg::ClickConfirm,
    /// where the pointer was when the right button went down, which tilts
    /// the console while held.
    tilting_from: Option<(f32, f32)>,
    cursor: (f32, f32),
    last_title_update: Instant,
    /// when the next frame is due. frames run to a schedule rather than one
    /// after another, so that a slow one is made up by those after it and
    /// the sound, made as the console runs, keeps up.
    next_frame: Instant,
    /// frames in a row not shown while catching up.
    skipped: u32,
    /// frames shown since the title was last updated.
    shown: u32,
    /// how long emulating and showing frames took since the title was last
    /// updated, for the frame rate's log.
    spent: Spent,
    /// stopped with F1, without the menu.
    paused: bool,
    /// Tab is held down, for fast forward.
    fast_forward_key: bool,
    /// the game runs as fast as it can, its sound left out.
    fast_forwarding: bool,
    /// when fast forward shows its next frame, on a grid at the console's
    /// frame rate.
    next_shown: Instant,
    stop: bool,
    /// what the running game's cameras were given to see, to give them
    /// another only when the settings change it.
    camera: Option<(CameraSource, Option<PathBuf>)>,
    /// the room this program holds open, and the one it is in.
    room_server: Option<zakuro_core::room::Server>,
    room: Option<zakuro_core::room::Connection>,
    /// Download Play as a guest, while in a room with no game running.
    dlp_guest: Option<zakuro_core::download_play::Guest>,
    /// the player's own copy of the child joined for, played in place of
    /// one sent.
    dlp_own: Option<PathBuf>,
    /// the network the next game joins as it starts, the host's of a
    /// Download Play child.
    wireless_reboot: Option<[u8; 16]>,
    /// the window was maximized when it went full screen, which leaving
    /// full screen brings back.
    maximized_before_full_screen: bool,
    /// the layout or the scale changed while the window was full screen,
    /// which kept its size, so leaving full screen fits the window to them.
    resize_after_full_screen: bool,
    // hfr: present pacing above the console's rate
    hfr: hfr::Pacer,
    /// ui: the Fates interface, its options menu and notices.
    fates: fates_ui::FatesUi,
    // qol: game speed, Skip Story's skips, the boon/bane panel
    qol: qol::State,
    /// updscreen: the work before the game's code loads, while it shows;
    /// the game starts once it is done
    pregame: Option<fates_ui::pregame::Screen>,
}

/// how long each step of the loop took, its mean and its longest.
#[derive(Default)]
struct Spent {
    emulating: Times,
    showing: Times,
}

#[derive(Default)]
struct Times {
    total: Duration,
    longest: Duration,
    count: u32,
}

impl Times {
    fn add(&mut self, time: Duration) {
        self.total += time;
        self.longest = self.longest.max(time);
        self.count += 1;
    }
}

impl std::fmt::Display for Times {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mean = self.total.as_secs_f64() * 1e3 / self.count.max(1) as f64;
        write!(f, "{mean:.1} ms, at most {:.1}", self.longest.as_secs_f64() * 1e3)
    }
}

/// one 3DS frame, about 16.71 ms.
const FRAME_TIME: Duration =
    Duration::from_nanos(zakuro_core::CYCLES_PER_FRAME * 1_000_000_000 / zakuro_core::kernel::thread::CPU_CLOCK_HZ);
/// how far behind the schedule may fall before it starts over instead of
/// running fast to catch up, after a pause say.
const CATCH_UP_LIMIT: Duration = Duration::from_millis(200);
/// frames that may go unshown in a row while catching up.
const MAX_SKIPPED: u32 = 4;

/// whether a frame goes unshown, the game running behind the schedule with
/// a presenter that waits for the display, and fewer than MAX_SKIPPED
/// unshown before it.
fn skips_showing(playing: bool, behind: bool, waits: bool, skipped: u32) -> bool {
    playing && behind && waits && skipped < MAX_SKIPPED
}

/// what the player is told when the 3D can't be drawn on the GPU.
const GPU_FALLBACK: &str = "The 3D can't be drawn on the GPU, as Vulkan isn't working on this computer, so it's drawn in software. \
Games run much slower that way, and texture packs are left out. Updating the graphics driver usually fixes it; the log says \
what went wrong. To draw in software without this message, turn off \"Draw the 3D on the GPU\" in Settings, Graphics.";

/// port: Haiku's own icon (a PNG the program embeds), for the window and
/// the taskbar; when it is set, the game's 3DS icon is not used.
pub static WINDOW_ICON_PNG: std::sync::OnceLock<&'static [u8]> = std::sync::OnceLock::new();

fn haiku_icon(side: u32) -> Option<winit::window::Icon> {
    let png = WINDOW_ICON_PNG.get()?;
    let image = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?;
    let image = image.resize_exact(side, side, image::imageops::FilterType::Lanczos3).into_rgba8();
    winit::window::Icon::from_rgba(image.into_raw(), side, side).ok()
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let size = self.window_size();
        // port: the game's own name, not the runtime's
        let attributes = Window::default_attributes().with_title(if fates_ui::enabled() { "Haiku" } else { "Zakuro" }).with_inner_size(size)
            .with_window_icon(haiku_icon(32))
            // qol4: made hidden, then shown cloaked until its first picture
            // is presented (qol4::window_created), so the window is never
            // seen unpainted (white) while the renderer starts and the game
            // loads
            .with_visible(!qol4::start_hidden());
        // port: the taskbar and Alt+Tab take the large icon
        #[cfg(windows)]
        let attributes = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attributes.with_taskbar_icon(haiku_icon(256))
        };
        let renderer = self.options.renderer.unwrap_or(match self.settings.renderer {
            Renderer::Vulkan => RendererKind::Vulkan,
            Renderer::OpenGl => RendererKind::OpenGl,
        });

        let backend = Backend::create(event_loop, attributes.clone(), renderer).or_else(|error| {
            if renderer != RendererKind::Vulkan {
                return Err(error);
            }
            // a machine without a working Vulkan driver still gets a window
            log::warn!("could not start the Vulkan backend, {error}, presenting with OpenGL instead");
            Backend::create(event_loop, attributes, RendererKind::OpenGl)
        });
        match backend {
            Ok((window, mut backend)) => {
                log::info!("presenting with the {} backend", backend.name());
                backend.set_layout(self.layout.screens());
                backend.set_orientation(self.orientation.screen_orientation());
                backend.set_top_width(wide::top_width(self.active.top_shape.width()));
                backend.set_scaling(self.active.filter.screens(), self.active.integer_scale);
                backend.set_stereo(self.active.stereo());
                self.gui = Some(Gui::new(&window));
                qol4::window_created(&window); // qol4: cloaked until its first picture
                self.window = Some(window);
                self.backend = Some(backend);
                self.display_apply(); // hfr
                // bottom: a new presenter has none of the panel's textures
                self.bottom.presenter_changed();
                // the game the command line names starts on the device the
                // presenter may lend the renderer, once there is one
                // updscreen: or once the work before it is done
                if self.pregame.is_none() {
                    self.start();
                }
            }
            Err(error) => {
                eprintln!("zakuro: could not start the {renderer:?} backend: {error}");
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        // Tab is fast forward while a game runs, not a key for moving the
        // interface's focus around
        let (tab, repeat) = match &event {
            WindowEvent::KeyboardInput { event, .. } => (event.physical_key == PhysicalKey::Code(KeyCode::Tab), event.repeat),
            _ => (false, false),
        };
        let in_game = self.game.as_ref().is_some_and(|game| game.system.keyboard_request().is_none())
            && !online_ui::typing() // online: the code box takes the keys
            && !self.menus.over_game()
            && !self.fates.takes_keys(); // ui
        // a Tab held down keeps repeating over a message too
        let withheld = tab && (in_game || (repeat && self.game.is_some()));
        let consumed = match (&mut self.gui, &self.window) {
            (Some(gui), Some(window)) if !withheld => gui.event(window, &event),
            _ => false,
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            // keys let go while another window has the keyboard never come
            WindowEvent::Focused(false) => {
                self.fast_forward_key = false;
                self.keyboard.release();
                // ui: muting in the background
                self.fates.focused = false;
                self.apply_volume();
            }
            WindowEvent::Focused(true) => {
                self.fates.focused = true;
                self.apply_volume();
            }
            WindowEvent::Resized(size) => {
                if let Some(backend) = &mut self.backend {
                    backend.resize(size.width, size.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => self.key(event, consumed),
            WindowEvent::MouseInput { state, button, .. } => {
                if button == MouseButton::Left {
                    let pressed = state == ElementState::Pressed && !self.pointer_taken(consumed);
                    // bottom2: the floating bottom screen's frame is taken by
                    // the mouse rather than touched, and placed when let go
                    let took = pressed && self.bottom.press(self.cursor.0, self.cursor.1);
                    if !pressed {
                        if let Some(place) = self.bottom.release() {
                            self.bottom_placed(place);
                        }
                    }
                    self.mouse_down = pressed && !took;
                    // randomizer: a click on a panel of a New Game mode screen
                    if pressed && !took {
                        self.rando_menu_click();
                    }
                    self.touch();
                    // cameras: a drag off the bottom screen moves the free
                    // camera, or turns My Castle's view, while either is live
                    let camera = pressed && !took && self.keyboard.state().touch.is_none() && cameras::mouse_down(cameras::Drag::Orbit, self.cursor);
                    if !pressed && cameras::dragging() {
                        cameras::mouse_up();
                    }
                    // offline-msg: a click off the lower screen confirms the
                    // not-available message while it waits
                    if pressed && !took && !camera && self.keyboard.state().touch.is_none() {
                        if let Some(game) = &self.game {
                            self.offline_click.click(&game.system);
                        }
                    }
                    self.bottom_pointer_icon();
                }
                // randomizer: the right button goes back on a New Game mode screen
                if button == MouseButton::Right && state == ElementState::Pressed && !self.pointer_taken(consumed) {
                    zakuro_core::randomizer::menu::menu_back();
                }
                // cameras: the right or middle button pans the free camera
                let pan = matches!(button, MouseButton::Right | MouseButton::Middle);
                let pressed_here = state == ElementState::Pressed && !self.pointer_taken(consumed);
                let camera = pan && pressed_here && cameras::mouse_down(cameras::Drag::Pan, self.cursor);
                if pan && state == ElementState::Released && cameras::dragging() {
                    cameras::mouse_up();
                }
                if button == MouseButton::Right && !camera {
                    let pressed = state == ElementState::Pressed && !self.pointer_taken(consumed);
                    self.tilting_from = pressed.then_some(self.cursor);
                    self.tilt();
                }
            }
            // cameras: the wheel zooms the free camera
            WindowEvent::MouseWheel { delta, .. } if !self.pointer_taken(consumed) => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y,
                    MouseScrollDelta::PixelDelta(position) => position.y as f32 / 40.0,
                };
                cameras::wheel(lines);
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x as f32, position.y as f32);
                cameras::mouse_moved(self.cursor); // cameras
                self.bottom.drag_to(self.cursor.0, self.cursor.1); // bottom2
                self.touch();
                self.bottom_pointer_icon(); // bottom2
                self.tilt();
            }
            WindowEvent::RedrawRequested => self.step(event_loop),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl App {
    /// the id of the running game, none without one, or with the test
    /// pattern.
    fn program_id(&self) -> Option<u64> {
        Some(self.game.as_ref()?.system.title.as_ref()?.program_id())
    }

    /// the console's settings for a game, from the command line and the
    /// settings it plays with.
    fn config(&self, settings: &Settings) -> Config {
        let interpret = self.options.interpreter || !settings.recompiled;
        // wide: the top screen's shape, asked for before the game draws
        let window = self.window.as_ref().map(|w| (w.inner_size().width, w.inner_size().height));
        let composes = self.backend.as_ref().is_some_and(present::Backend::composes);
        let window = wide::room(window, settings, composes);
        let display = self.window.as_ref().and_then(|w| w.current_monitor()).map(|m| (m.size().width, m.size().height));
        wide::request(settings.pc.aspect, window, display);
        wide::sides(settings.pc.wide_sides); // wide-ui
        Config {
            new3ds: self.options.new3ds,
            data_dir: self.data_dir.clone(),
            // jpprep: the Japanese game is Japanese only, as a Japanese console reports
            language: if zakuro_core::region::is_jpn() { 0 } else { settings.language.code() },
            recompiled: self.options.recompiled.clone().filter(|_| !interpret).map(Into::into),
            linked: self.linked.filter(|_| !interpret),
            find_recompiled: !interpret,
            // native-final: the pure build has no emulated rasterizer
            hardware_renderer: cfg!(feature = "emulation") && self.options.hardware_rasterizer.unwrap_or(settings.hardware_rasterizer),
            texture_packs: settings.texture_packs,
            interpolate: settings.frame_interpolation,
            enhancements: settings
                .enhancements
                .iter()
                .filter_map(|(id, names)| Some((u64::from_str_radix(id, 16).ok()?, names.clone())))
                .collect(),
            resolution: settings.resolution,
            device: self.backend.as_ref().and_then(Backend::shared_device),
            username: self.nickname(),
            console_id: settings.console_id,
            slider_3d: settings.slider_3d(),
            mods: haiku_builtin::with_mods(self.fates.mods.content_dirs()), // mods; sunglasses: Haiku's own layer under them
            ..Config::default()
        }
    }

    /// starts what the command line asks for, the test pattern or a game.
    fn start(&mut self) {
        states::set_dir(self.data_dir.as_deref()); // pulse: userdata/haiku/states
        if self.options.test_pattern {
            // the test pattern has no CPU program to run, paused keeps just
            // presenting what is there
            let system = paint_test_pattern(self.config(&self.settings));
            self.game = Some(Running::new(system, PathBuf::new(), "test pattern".to_owned()));
            self.paused = true;
        } else if let Some(rom) = self.options.rom.clone() {
            if let Err(error) = self.play(Path::new(&rom)) {
                eprintln!("zakuro: {error}");
                std::process::exit(1);
            }
        }
        if self.options.profile {
            if let Some(game) = &mut self.game {
                game.system.enable_profiler();
            }
        }
    }

    /// starts the game at path, in place of any other, with what it has of
    /// its own in the settings.
    fn play(&mut self, path: &Path) -> Result<(), String> {
        // the old game and what it holds on the GPU go first, its sound too,
        // and the overlay's line about it
        self.game = None;
        self.overlay_corner = None;
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        // a Download Play guest goes with the game
        self.dlp_guest = None;
        // one from the command line isn't in the library, or not yet, and
        // a Download Play child listed apart from its game is there with no id
        let known = self.library.games.iter().find(|game| game.path == path && game.problem.is_none()).map(|game| game.program_id);
        let program_id = known.or_else(|| Some(zakuro_fs::Title::load(path).ok()?.program_id()));
        self.active = self.settings.for_game(program_id);
        let started = self.start_game(path);
        if started.is_err() {
            self.active = self.settings.clone();
        }
        self.apply_layout();
        started
    }

    /// loads the game at path, which the settings in force are for.
    fn start_game(&mut self, path: &Path) -> Result<(), String> {
        self.fates_mods_scan(); // mods: the mods folder, read again at each start
        let code_mods = self.fates_code_mods(path); // codemods: before config(), which leaves out what it leaves out
        let mut config = self.config(&self.active);
        config.code_mods = code_mods; // codemods
        haiku_voices::lay(&mut config.mods, &self.active.pc, path); // jpvoice: the voice layer, under the mods
        haiku_voices::started(path); // jpvoice
        haiku_fanpatch::lay_text(&mut config.mods, path); // fanpatch: Haiku's own text kept under the mods'
        // codemods2: a code mod's saves are its own, the game's are left alone
        if let Some(data) = self.data_dir.clone() {
            config.saves_dir = haiku_codemods::saves_dir(&data);
            match &config.saves_dir {
                Some(dir) => log::info!("saves: {} (code mods run: the game's own saves in {} are left alone)", dir.display(), data.join("user").display()),
                None => log::info!("saves: {}", data.join("user").display()),
            }
            states::set_dir(Some(&haiku_codemods::states_base(&data))); // pulse: the save states go with the saves
        }
        config.wireless_reboot = self.wireless_reboot.take();
        // the update, DLC and Download Play child found with the game in
        // the library
        if let Some(game) = self.library.games.iter().find(|game| game.path == path) {
            config.update = game.update.as_ref().map(|update| update.path.clone());
            config.dlc = game.dlc.iter().map(|dlc| dlc.path.clone()).collect();
            config.dlp_child = game.dlp_child.as_ref().map(|child| child.path.clone());
        } else if let Some((update, dlc, dlp_child)) = library::addons_beside(path) {
            // one from the command line starts before the library's scan is
            // in, or isn't in its folder: what's next to it goes with it
            config.update = update.map(|update| update.path);
            config.dlc = dlc.into_iter().map(|dlc| dlc.path).collect();
            config.dlp_child = dlp_child.map(|child| child.path);
        }
        // a recording starts the clock at a known time, for playing it back
        let record = std::env::var_os("ZAKURO_RECORD").map(PathBuf::from);
        let clock = zakuro_core::memory::config::host_clock();
        if record.is_some() {
            config.clock = Some(clock);
        }
        // and plays back from the clock it was made at
        let replay = match std::env::var_os("ZAKURO_REPLAY").map(PathBuf::from) {
            Some(file) => {
                let replay = zakuro_core::replay::Replay::open(&file).map_err(|error| format!("could not read {}, {error}", file.display()))?;
                log::info!("playing back the inputs in {}", file.display());
                config.clock = Some(replay.clock());
                Some(replay)
            }
            None => None,
        };
        let layers = config.mods.clone(); // statecheck: the layer, as the loader gets it
        let system = loader::load(path, config).map_err(|error| format!("could not open {}, {error}", path.display()))?;
        statecheck::started(&system, &layers, &self.fates.mods, path, self.active.language); // statecheck: what states made now carry
        self.fates_code_mods_loaded(); // codemods: what the loader said about them
        // online2: the gameplay mods this game runs with, for online battles
        zakuro_core::netplay::set_compat(zakuro_core::netplay::Compat { gameplay_mods: self.fates.mods.online_stamps(), cheated_units: false });
        let outdated = system.recompiled.as_ref().is_some_and(|code| code.outdated());
        // the settings ask for the 3D on the GPU, but Vulkan didn't start:
        // only the log said so, while the game ran slowly
        let in_software = system.config.hardware_renderer && !system.gpu.draws_on_gpu();
        let name = self
            .library
            .games
            .iter()
            .find(|game| game.path == path)
            .map(|game| game.name.clone())
            .or_else(|| Some(library::Smdh::parse(system.title.as_ref()?.exefs_file("icon")?)?.name).filter(|name| !name.is_empty()))
            .or_else(|| path.file_stem().map(|stem| stem.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let mut game = Running::new(system, path.to_owned(), name);
        game.replay = replay;
        // gfx-models: dev, FATES_LOAD in a window too (a saved state, for
        // reproducing a scene windowed with real time and pictures in
        // between); the replay's frames count from the state's
        if let Some(first) = native_cpu::headless_load(&mut game.system) {
            game.frame = first;
        }
        if let Some(record) = record {
            match zakuro_core::replay::Recorder::create(&record, clock) {
                Ok(recorder) => {
                    log::info!("recording the inputs to {}", record.display());
                    game.recorder = Some(recorder);
                }
                Err(error) => log::warn!("could not record to {}, {error}", record.display()),
            }
        }
        // both can be so, and the GPU's is what explains a slow game
        let notes: Vec<String> = [
            in_software.then(|| GPU_FALLBACK.to_owned()),
            outdated.then(|| {
                format!(
                    "{} was recompiled by an older version of Zakuro. Recompile it again from the library to get the newest improvements.",
                    game.name
                )
            }),
        ]
        .into_iter()
        .flatten()
        .collect();
        if !notes.is_empty() {
            self.menus.message = Some(notes.join("\n\n"));
        }
        self.game = Some(game);
        // input: the button glyphs for the controller in use
        self.glyphs.reset();
        self.apply_glyphs();
        // the new game's cameras see what the settings say, and it plays
        // in the room the player is in
        self.camera = None;
        self.apply_camera();
        self.link_room();
        // nothing left open holds the game back
        self.menus.menu_open = false;
        self.menus.settings_open = false;
        self.menus.game_settings = None;
        self.keyboard.release();
        self.paused = false;
        self.next_frame = Instant::now();
        self.hfr_apply(); // hfr
        qol::apply(&self.settings.pc); // qol: the game-level options, from the start
        zakuro_core::supports::set_settings(self.settings.pc.corrin_s_any, self.settings.pc.kana_gender.core()); // supports
        info_ui::apply(&self.settings.pc); // info-ui
        cheats::apply(&self.settings.pc); // cheats
        pulse::apply(&self.settings.pc); // pulse
        Ok(())
    }

    fn back_to_library(&mut self) {
        self.game = None;
        self.overlay_corner = None;
        if let Some(room) = &self.room {
            room.playing("");
        }
        if let Some(audio) = &self.audio {
            audio.clear();
        }
        self.menus.menu_open = false;
        self.keyboard.release();
        // what the game had of its own goes with it
        self.active = self.settings.clone();
        self.apply_layout();
    }

    /// arranges the screens another way, for the running game alone when it
    /// has an arrangement of its own.
    fn set_layout(&mut self, layout: Screens) {
        let own = self.program_id().and_then(|id| self.settings.per_game.get_mut(&settings::game_key(id)));
        match own.filter(|own| own.layout.is_some()) {
            Some(own) => own.layout = Some(layout),
            None => self.settings.layout = layout,
        }
        self.settings.save();
        self.active = self.settings.for_game(self.program_id());
        self.apply_layout();
    }

    fn key(&mut self, event: KeyEvent, consumed: bool) {
        let pressed = event.state == ElementState::Pressed;
        let PhysicalKey::Code(code) = event.physical_key else { return };
        // Escape leaves a controller binding waiting for a button as it was
        if pressed && code == KeyCode::Escape && self.menus.rebinding_pad.take().is_some() {
            return;
        }
        // a binding waiting for a key takes it, Escape leaves it as it was
        if let (true, Some(index)) = (pressed, self.menus.rebinding) {
            self.menus.rebinding = None;
            // qol4: Escape is a key the options menu's row takes (its
            // default); the other rows keep it as cancel. A key bound to the
            // options menu leaves the other rows, and the other way round
            if code != KeyCode::Escape || self.settings.keys.is_options_row(index) {
                self.settings.keys.assign(index, code);
                self.apply_settings();
            }
            return;
        }
        // qol4: the options menu's key (Escape unless rebound) opens and
        // closes it, and never reaches the game
        if self.qol4_options_key(code, pressed, event.repeat) {
            return;
        }
        if code == KeyCode::Tab && !event.repeat && !self.settings.keys.binds(KeyCode::Tab) {
            // fast forward while it is held
            self.fast_forward_key = pressed && self.game.is_some();
        }
        if pressed && !event.repeat {
            match code {
                KeyCode::Escape if self.game.is_some() => {
                    if std::env::var_os("FATES_ORIGINAL_UI").is_some() {
                        return;
                    }
                    self.menus.menu_open = !self.menus.menu_open;
                    self.keyboard.release();
                    return;
                }
                KeyCode::F1 => {
                    self.paused = !self.paused;
                    log::info!("{}", if self.paused { "paused" } else { "resumed" });
                }
                // bottom: the bottom screen's modes, then the console's layouts
                KeyCode::F9 => {
                    let (mode, layout) = bottom_screen::next_arrangement(self.settings.pc.bottom_screen, self.active.layout);
                    self.settings.pc.bottom_screen = mode;
                    log::info!("bottom screen {mode:?}, layout {layout:?}");
                    self.set_layout(layout);
                }
                KeyCode::F10 if self.active.layout.other_screen() != self.active.layout => self.set_layout(self.active.layout.other_screen()),
                KeyCode::F11 => self.toggle_fullscreen(),
                // savehang: every game thread's state into the log, for hangs
                KeyCode::F12 if self.game.is_some() => zakuro_core::pcos::hang::request(),
                // port: no emulator save states; the game saves for itself
                KeyCode::F5 if self.game.is_some() && !fates_ui::enabled() => self.save_state(),
                KeyCode::F8 if self.game.is_some() && !fates_ui::enabled() => self.load_state(),
                _ => {}
            }
        }
        let to_game = self.game.is_some()
            && !self.menus.over_game()
            && !self.fates.takes_keys() // ui
            && !consumed
            && !self.gui.as_ref().is_some_and(Gui::wants_keyboard);
        // letting go always gets through, so that no button stays held
        if to_game || !pressed {
            self.keyboard.key(event.physical_key, pressed);
        }
    }

    /// takes in the controllers' buttons, a binding waiting for one takes
    /// it, and Home opens and closes the menu over the game.
    fn poll_gamepads(&mut self) {
        // input: SDL3 pads; presses go to a binding waiting for one, the
        // rest of the state to the public input API (input_api.rs)
        for press in self.gamepads.poll() {
            if let Some(index) = self.menus.rebinding_pad.take() {
                // qol4: the options menu's button kept apart from the rest
                self.settings.pad.assign(index, controls::stored(press.button, press.positional));
                self.apply_settings();
            }
        }
        let (pads, pad_used) = self.gamepads.output();
        let last = match (pad_used, self.keyboard.used) {
            (Some((family, pad)), Some(key)) if pad > key => controls::Device::Pad(family),
            (Some((family, _)), None) => controls::Device::Pad(family),
            _ => controls::Device::Keyboard,
        };
        let keyboard = self.keyboard.state();
        let circle = input::combine(keyboard, &pads);
        // qol3: the options menu takes every input while it is open: the
        // game, the cameras, the touch cursor and the PC actions see none of
        // it (the menu reads the pads itself, below); only the Options
        // action still goes through, to close it
        if fates_ui::enabled() && self.fates.modal() { // updater: modal, the update's screen too
            let options = controls::PcAction::ALL.iter().position(|action| *action == controls::PcAction::Options).unwrap_or(0);
            let mut held = controls::PadOutput::default();
            held.actions[options] = pads.actions[options] || self.keyboard.actions()[options];
            input_api::update(zakuro_core::services::hid::PadState::empty(), [false; controls::PcAction::COUNT], [0.0; 2], &held, last, self.gamepads.list());
        } else {
            input_api::update(keyboard.buttons, self.keyboard.actions(), [circle.circle_x, circle.circle_y], &pads, last, self.gamepads.list());
        }
        // cameras: the free camera's stick and buttons; its key flips the
        // setting, which is saved and shown
        cameras::poll(self.window.as_ref().map_or(0.0, |window| window.inner_size().height as f32));
        if let Some(on) = cameras::take_toggled() {
            self.settings.pc.battle_free_camera = on;
            self.settings.save();
            self.active = self.settings.for_game(self.program_id());
            self.menus.toast = Some((format!("Battle Free Camera {}", if on { "On" } else { "Off" }), Instant::now()));
        }
        // qol5: My Castle's free camera (not saved): the toast says how back
        if let Some(change) = cameras::take_castle_toggled() {
            self.menus.toast = Some((fates_ui::options::castle_free_camera_toast(&self.settings, change), Instant::now()));
        }
        // ui: the Fates interface moves with the pads' console buttons
        // (bindings and button layout applied), and the Options action opens it
        if fates_ui::enabled() {
            self.fates.pad_held = pads.buttons;
            self.fates.pad_circle = pads.circle;
            if input_api::take_action(controls::PcAction::Options) {
                self.fates.options_action = true;
            }
        }
        // the Options action (Guide/Home) is the ui agent's; Zakuro's own
        // menu takes it only where the original UI is not kept
        if std::env::var_os("FATES_ORIGINAL_UI").is_none() && self.game.is_some() && input_api::take_action(controls::PcAction::Options) {
            self.menus.menu_open = !self.menus.menu_open;
            self.keyboard.release();
        }
        self.apply_glyphs();
    }

    /// input: draws the button glyphs of the glyph set in use, when it changed.
    fn apply_glyphs(&mut self) {
        let set = glyphs::forced().unwrap_or_else(input_api::glyph_set);
        if let Some(game) = &mut self.game {
            self.glyphs.apply(set, &mut game.system, self.data_dir.as_deref(), self.active.texture_packs);
        }
    }

    /// bottom: whether the window is composed around the bottom screen, which
    /// takes a game and a presenter that composes.
    fn composes(&self) -> bool {
        self.game.is_some() && self.backend.as_ref().is_some_and(present::Backend::composes)
    }

    /// bottom2: the floating bottom screen was put somewhere with the mouse.
    fn bottom_placed(&mut self, place: zakuro_gpu::backend::bottom_panel::Placement) {
        bottom_screen::store_placement(&mut self.settings.pc, place);
        self.settings.save();
        self.active = self.settings.for_game(self.program_id());
    }

    /// bottom2: the pointer's shape over the floating bottom screen's frame.
    fn bottom_pointer_icon(&mut self) {
        if let (Some(icon), Some(window)) = (self.bottom.icon_change(), &self.window) {
            window.set_cursor(icon);
        }
    }

    /// randomizer: a click on the upper screen while a New Game mode screen
    /// (difficulty's mode, the Normal / Randomizer step, the randomizer
    /// screen) is up chooses the panel under it. A 3DS cannot touch the
    /// upper screen, so the game reads only buttons there; the panel is
    /// found here, in the console's 400x240 (the 400-wide picture centred in
    /// a wider one), and the menu is driven by its own buttons
    /// (overrides/randomizer.c, rando_pointer).
    fn rando_menu_click(&self) -> bool {
        if zakuro_core::randomizer::menu::menu_panels() == 0 || self.orientation != settings::Orientation::North {
            return false;
        }
        let (Some(top), Some(window)) = (self.qol.top_screen, &self.window) else { return false };
        let points = window.scale_factor() as f32;
        let (x, y) = (self.cursor.0 / points, self.cursor.1 / points);
        if !top.contains(egui::pos2(x, y)) || top.height() <= 0.0 {
            return false;
        }
        let scale = top.height() / 240.0;
        let wide = top.width() / scale;
        zakuro_core::randomizer::menu::menu_click((x - top.min.x) / scale - (wide - 400.0) / 2.0, (y - top.min.y) / scale)
    }

    /// whether the pointer is busy with something other than the game.
    fn pointer_taken(&self, consumed: bool) -> bool {
        consumed || self.game.is_none() || self.menus.menu_open || self.gui.as_ref().is_some_and(Gui::wants_pointer)
            // ui
            || self.fates.wants_pointer || self.fates.modal() // updater: modal
    }

    /// touches the bottom screen while the button is down over it.
    fn touch(&mut self) {
        let Some(window) = &self.window else { return };
        let size = window.inner_size();
        // bottom: where the panel put the bottom screen, when it composes
        if let Some(bottom) = self.bottom.composed_bottom() {
            let (x, y) = self.cursor;
            self.bottom.mouse_at(x, y, &self.active);
            let point = zakuro_gpu::backend::bottom_panel::touch_point(bottom, (size.width, size.height), x, y);
            self.keyboard.touch(point.filter(|_| self.mouse_down));
            return;
        }
        let (_, bottom) = layout(size.width, size.height, self.layout.screens(), self.orientation.screen_orientation(), wide::top_width(self.top_shape.width()), self.active.integer_scale);
        // with the top screen alone there is nothing to touch
        let Some(bottom) = bottom else {
            self.keyboard.touch(None);
            return;
        };
        let x = self.cursor.0 - bottom.x;
        let y = self.cursor.1 - bottom.y;
        let inside = x >= 0.0 && y >= 0.0 && x < bottom.width && y < bottom.height;
        self.keyboard.touch((self.mouse_down && inside).then(|| {
            let (u, v) = (x / bottom.width, y / bottom.height);
            let (u, v) = match self.orientation {
                Orientation::North => (u, v),
                Orientation::East => (v, 1.0 - u),
                Orientation::South => (1.0 - u, 1.0 - v),
                Orientation::West => (1.0 - v, u),
            };
            (((u * 320.0) as u16).min(319), ((v * 240.0) as u16).min(239))
        }));
    }

    /// tilts the console while the right button is held, toward where the
    /// pointer went and by how far, as Citra's motion emulation does,
    /// which titles were tried with. letting go stands it upright again.
    fn tilt(&mut self) {
        // radians a pixel, and the furthest it goes, a quarter turn
        const PER_PIXEL: f32 = 0.01;
        let tilt = match (self.tilting_from, &self.window) {
            (Some((x, y)), Some(window)) => {
                let scale = window.scale_factor() as f32;
                let (dx, dy) = ((self.cursor.0 - x) / scale, (self.cursor.1 - y) / scale);
                let distance = dx.hypot(dy);
                let angle = (distance * PER_PIXEL).min(std::f32::consts::FRAC_PI_2);
                if distance > 0.0 { [dx / distance * angle, dy / distance * angle] } else { [0.0; 2] }
            }
            _ => [0.0; 2],
        };
        self.keyboard.tilt(tilt);
    }

    fn toggle_fullscreen(&mut self) {
        if let Some(window) = &self.window {
            let full = window.fullscreen().is_some();
            if !full {
                self.maximized_before_full_screen = window.is_maximized();
                self.resize_after_full_screen = false;
            }
            window.set_fullscreen((!full).then_some(Fullscreen::Borderless(None)));
            // the layout may have changed meanwhile, which full screen kept
            // the window's size through. a window maximized before comes
            // back maximized, resizing it would take it out of that
            if full && self.resize_after_full_screen && !self.maximized_before_full_screen {
                let _ = window.request_inner_size(self.window_size());
            }
        }
    }

    fn apply_volume(&self) {
        if let Some(audio) = &self.audio {
            // ui: muted in the background when the options say so
            let background = self.settings.pc.mute_unfocused && !self.fates.focused;
            audio.set_volume(if self.settings.mute || background { 0.0 } else { self.settings.volume });
        }
    }

    /// saves the settings and puts to use what can change right away.
    fn apply_settings(&mut self) {
        self.settings.save();
        fates_ui::ja::set_english(self.settings.pc.japanese_english_menus); // jpprep
        self.active = self.settings.for_game(self.program_id());
        if let Some(game) = &mut self.game {
            // gfx-models: hfr_apply below decides while the frame rate loop
            // runs (above the console's rate it interpolates whatever this
            // setting says); switching it off here and on there started the
            // pictures over at every change in the options (a hitch each)
            let hfr_decides = std::env::var_os("ZAKURO_HFR_OFF").is_none();
            if !hfr_decides && game.system.gpu.interpolates() != self.active.frame_interpolation {
                let on = game.system.set_interpolation(self.active.frame_interpolation);
                log::info!("frame interpolation {}", if on { "on" } else { "off" });
            }
            // games read the 3D slider every frame
            let slider = self.active.slider_3d();
            if game.system.config.slider_3d != slider {
                game.system.set_slider_3d(slider);
            }
        }
        self.apply_camera();
        cameras::configure(&self.settings.pc); // cameras
        self.keyboard.set_keys(self.settings.keys.clone());
        // input: bindings, layout, dead zone, rumble and glyph settings
        let pc = &self.settings.pc;
        self.gamepads.configure(self.settings.pad.clone(), pc.button_layout, pc.stick_deadzone);
        input_api::configure(pc.rumble, pc.glyphs, pc.button_layout, &self.settings.keys, &self.settings.pad);
        self.apply_volume();
        self.apply_layout();
        // hfr: frame rate, display modes, vsync, resolution
        self.hfr_apply();
        qol::apply(&self.settings.pc); // qol
        zakuro_core::randomizer::set_seed_setting(zakuro_core::randomizer::parse_seed(&self.settings.pc.randomizer_seed)); // randomizer
        zakuro_core::supports::set_settings(self.settings.pc.corrin_s_any, self.settings.pc.kana_gender.core()); // supports
        info_ui::apply(&self.settings.pc); // info-ui
        cheats::apply(&self.settings.pc); // cheats
        pulse::apply(&self.settings.pc); // pulse
        self.display_apply();
        self.display_apply_resolution();
    }

    /// gives the running game's cameras what the settings say they see,
    /// when it is not what they have.
    fn apply_camera(&mut self) {
        let wanted = (self.settings.camera, self.settings.camera_picture.clone());
        let Some(game) = &mut self.game else { return };
        if self.camera.as_ref() == Some(&wanted) {
            return;
        }
        game.system.camera = match &wanted {
            (CameraSource::Webcam, _) => Some(Box::new(camera::Webcam::default())),
            (CameraSource::Picture, Some(path)) => match camera::picture(path) {
                Ok(picture) => {
                    log::info!("the cameras see {}", path.display());
                    Some(Box::new(picture))
                }
                Err(error) => {
                    self.menus.message = Some(error);
                    None
                }
            },
            _ => None,
        };
        self.camera = Some(wanted);
    }

    /// puts the screens' arrangement and the window scale to use, sizing
    /// the window to them.
    fn apply_layout(&mut self) {
        if let Some(backend) = &mut self.backend {
            backend.set_top_width(wide::top_width(self.active.top_shape.width()));
            backend.set_scaling(self.active.filter.screens(), self.active.integer_scale);
            backend.set_stereo(self.active.stereo());
        }
        let scale = if self.options.scale.is_none() { self.active.scale.max(1) } else { self.scale };
        if scale != self.scale || self.active.layout != self.layout || self.active.orientation != self.orientation || self.active.top_shape != self.top_shape {
            self.scale = scale;
            self.layout = self.active.layout;
            self.orientation = self.active.orientation;
            self.top_shape = self.active.top_shape;
            if let Some(backend) = &mut self.backend {
                backend.set_layout(self.layout.screens());
                backend.set_orientation(self.orientation.screen_orientation());
            }
            // a full screen or maximized window keeps its size, resizing
            // it would take it out of that, full screen fits it on leaving
            match &self.window {
                Some(window) if window.fullscreen().is_some() => self.resize_after_full_screen = true,
                Some(window) if !window.is_maximized() => {
                    let _ = window.request_inner_size(self.window_size());
                }
                _ => {}
            }
        }
    }

    /// the window size that fits the screens at the chosen scale.
    fn window_size(&self) -> winit::dpi::Size {
        // bottom: room for the docked panel beside the top screen
        let composes = self.backend.as_ref().is_none_or(present::Backend::composes);
        if let Some(mode) = bottom_screen::BottomScreen::mode(&self.active, composes) {
            let (width, height) = bottom_screen::window_size(mode, &self.active, self.scale as f32);
            return winit::dpi::LogicalSize::new(width, height).into();
        }
        let (width, height) = self.layout.screens().size(wide::top_width(self.top_shape.width()));
        let (width, height) = match self.orientation.screen_orientation().is_portrait() {
            true => (height, width),
            false => (width, height)
        };
        // scaled by whole numbers, a desktop scaled by 125% say would leave a
        // border, so the window gets whole multiples of the screens' pixels
        if let (true, Some(window)) = (self.active.integer_scale, &self.window) {
            let times = (self.scale as f64 * window.scale_factor()).round().max(1.0) as u32;
            return winit::dpi::PhysicalSize::new(width * times, height * times).into();
        }
        winit::dpi::LogicalSize::new(width * self.scale, height * self.scale).into()
    }

    /// what to recompile with: the computer's C compiler, or the Zig Zakuro
    /// downloaded, or else, when the player said yes to it, Zig downloaded
    /// first. with none of those there is none, it asks whether to download
    /// Zig for games, or says what is missing.
    fn toolchain(&mut self, games: &[Wanted]) -> Option<Toolchain> {
        let tools = zig::tools_dir(self.data_dir.as_deref());
        let error = match recomp3ds::compile::check(None) {
            Ok(compiler) => return Some(Toolchain::Ready(compiler)),
            Err(error) => error,
        };
        if let Some(compiler) = tools.as_deref().and_then(zig::installed) {
            return Some(Toolchain::Ready(compiler));
        }
        match tools {
            Some(tools) if self.download_compiler && zig::download_size().is_some() && zig::can_download() => return Some(Toolchain::Download(tools)),
            Some(_) if zig::download_size().is_some() && zig::can_download() => self.menus.compiler_offer = Some(games.to_vec()),
            Some(_) if zig::download_size().is_some() => {
                self.menus.message = Some(format!("Recompiling needs a C compiler, gcc or clang, or curl or wget for Zakuro to download one: {error}."));
            }
            _ => self.menus.message = Some(format!("Recompiling needs a C compiler: {error}.")),
        }
        None
    }

    /// puts games in line to recompile, after those there are, each with
    /// its turn once the one before it is done. download says the player
    /// agreed to download a compiler should there be none.
    fn recompile(&mut self, games: Vec<Wanted>, download: bool) {
        self.download_compiler = download;
        if self.toolchain(&games).is_none() {
            return;
        }
        for game in games {
            let running = self.jobs.iter().any(|job| job.program_id == game.program_id && !job.stage().finished());
            if !running && !self.waiting.iter().any(|waiting| waiting.program_id == game.program_id) {
                self.waiting.push(game);
            }
        }
        self.next_recompile();
    }

    /// starts the game next in line, when none is recompiling.
    fn next_recompile(&mut self) {
        if self.waiting.is_empty() || self.jobs.iter().any(|job| !job.stage().finished()) {
            return;
        }
        // a compiler can have stopped being there, its download failing or
        // being cancelled, and it asks again for those left
        let Some(toolchain) = self.toolchain(&self.waiting.clone()) else {
            self.waiting.clear();
            return;
        };
        self.download_compiler = false;
        let wanted = self.waiting.remove(0);
        self.jobs.retain(|job| job.program_id != wanted.program_id);
        // a scan may have taken it out of the library meanwhile
        if let Some(game) = self.library.games.iter().find(|game| game.path == wanted.path) {
            let update = game.update.as_ref().map(|update| update.path.as_path());
            self.jobs.push(Job::start(&game.path, update, game.program_id, &game.name, self.data_dir.as_deref(), toolchain));
        }
    }

    /// tells about recompiles as they finish, and starts the next.
    fn poll_jobs(&mut self) {
        let mut finished = Vec::new();
        for job in &mut self.jobs {
            if !job.announced && job.stage().finished() {
                job.announced = true;
                finished.push((job.program_id, job.name.clone(), job.stage()));
            }
        }
        for (program_id, name, stage) in finished {
            match stage {
                Stage::Done => {
                    self.library.recompiled(program_id);
                    // a game told to run on the interpreter stays on it
                    let playing = self.game.as_ref().is_some_and(|game| {
                        game.system.config.find_recompiled && game.system.title.as_ref().is_some_and(|title| title.program_id() == program_id)
                    });
                    let installed = zakuro_core::recompiled::installed(program_id);
                    if let (true, Some(game), Some(path)) = (playing, self.game.as_mut(), installed) {
                        // from here on the game runs on the new code, no reset needed
                        match zakuro_core::loader::swap_recompiled(&mut game.system, &path) {
                            Ok(()) => self.menus.message = Some(format!("{name} is recompiled, and it runs on the new code now.")),
                            Err(error) => {
                                log::warn!("could not switch {name} to its new code, {error}");
                                self.menus.message = Some(format!("{name} is recompiled. Reset it from the menu, Esc, to run it on the new code."));
                            }
                        }
                    }
                }
                // others can fail before anyone reads this, in a long line
                Stage::Failed(error) if error != "cancelled" => {
                    let failed = format!("Recompiling {name} failed, {error}");
                    self.menus.message = Some(self.menus.message.take().map_or(failed.clone(), |earlier| format!("{earlier}\n\n{failed}")));
                }
                _ => {}
            }
        }
        self.next_recompile();
    }

    fn act(&mut self, action: Action, event_loop: &ActiveEventLoop) {
        match action {
            Action::Play(path) => {
                if let Err(error) = self.play(&path) {
                    self.menus.message = Some(error);
                }
            }
            Action::Recompile(games) => self.recompile(games, false),
            Action::DownloadCompiler(games) => self.recompile(games, true),
            Action::Mods(program_id) => {
                if let Some(data_dir) = &self.data_dir {
                    let folder = zakuro_core::mods::dir(data_dir, program_id);
                    let made = std::fs::create_dir_all(folder.join("romfs")).and_then(|()| std::fs::create_dir_all(folder.join("textures")));
                    match made {
                        Ok(()) => show_folder(&folder),
                        Err(error) => self.menus.message = Some(format!("The mods folder can't be made: {error}")),
                    }
                }
            }
            Action::CancelRecompile(program_id) => {
                self.waiting.retain(|waiting| waiting.program_id != program_id);
                for job in self.jobs.iter().filter(|job| job.program_id == program_id) {
                    job.cancel();
                }
            }
            Action::CancelWaiting => self.waiting.clear(),
            Action::ChooseFolder => {
                if let Some(folder) = rfd::FileDialog::new().set_title("Where your games are").pick_folder() {
                    self.settings.games = Some(folder.clone());
                    self.settings.save();
                    self.library.scan(&folder);
                }
            }
            Action::SaveState => self.save_state(),
            Action::LoadState => self.load_state(),
            Action::HostRoom => self.host_room(),
            Action::JoinRoom => {
                let address = self.settings.room_address.clone();
                self.join_room(&address);
            }
            Action::LeaveRoom => self.leave_room(),
            Action::DlpJoin(index) => self.join_download_play(index),
            Action::DlpLeave => {
                if let Some(guest) = &mut self.dlp_guest {
                    guest.leave();
                }
                self.dlp_own = None;
            }
            Action::ChooseCameraPicture => {
                let picked = rfd::FileDialog::new()
                    .set_title("A picture for the cameras")
                    .add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "bmp"])
                    .pick_file();
                if let Some(file) = picked {
                    self.settings.camera = CameraSource::Picture;
                    self.settings.camera_picture = Some(file);
                    self.apply_settings();
                }
            }
            Action::ChooseBackground => {
                let picked = rfd::FileDialog::new()
                    .set_title("A picture for the library")
                    .add_filter("Pictures", &["png", "jpg", "jpeg", "webp", "bmp"])
                    .pick_file();
                if let Some(file) = picked {
                    self.settings.background = Some(file);
                    self.settings.save();
                }
            }
            Action::Rescan => {
                if let Some(folder) = self.settings.games.clone() {
                    self.library.scan(&folder);
                }
            }
            Action::Resume => {
                self.menus.menu_open = false;
                self.keyboard.release();
            }
            Action::Reset => {
                if let Some(path) = self.game.as_ref().map(|game| game.path.clone()) {
                    if let Err(error) = self.play(&path) {
                        self.menus.message = Some(error);
                    }
                }
            }
            Action::Library => self.back_to_library(),
            Action::Fullscreen => self.toggle_fullscreen(),
            Action::CopyReport => {
                if let (Some(game), Some(gui)) = (&self.game, &self.gui) {
                    gui.ctx.copy_text(report::text(&game.name, &game.system));
                    self.menus.message = Some("The game's details and the end of the log are copied. Paste them in your report and fill in what happens.".into());
                }
            }
            Action::Cheat(index, on) => {
                let Some(game) = &mut self.game else { return };
                let program_id = game.system.title.as_ref().map_or(0, |title| title.program_id());
                let Some(cheat) = game.system.cheats.get_mut(index) else { return };
                cheat.enabled = on;
                log::info!("{} {} {}", if cheat.builtin { "enhancement" } else { "cheat" }, cheat.name, if on { "on" } else { "off" });
                if cheat.builtin {
                    // an enhancement stays on for the game in the settings
                    let names = self.settings.enhancements.entry(format!("{program_id:016X}")).or_default();
                    names.retain(|name| *name != cheat.name);
                    if on {
                        names.push(cheat.name.clone());
                    }
                    self.settings.save();
                } else {
                    self.save_cheats();
                }
            }
            Action::AddCheat(name, code) => {
                let lines: Vec<String> = code.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_owned).collect();
                if lines.iter().any(|line| !zakuro_core::cheats::is_code(line)) {
                    self.menus.message = Some("Some of those lines aren't codes like 00000000 00000000, the cheat skips them.".into());
                }
                if let Some(game) = &mut self.game {
                    let name = match name.trim() {
                        "" => format!("Cheat {}", game.system.cheats.len() + 1),
                        name => name.to_owned(),
                    };
                    let mut cheat = zakuro_core::cheats::Cheat::new(&name, lines, Vec::new());
                    cheat.enabled = true;
                    game.system.cheats.push(cheat);
                }
                self.save_cheats();
            }
            Action::ReloadCheats => {
                if let (Some(path), Some(game)) = (self.cheats_path(), &mut self.game) {
                    let enhancements: Vec<_> = game.system.cheats.drain(..).filter(|cheat| cheat.builtin).collect();
                    game.system.cheats = zakuro_core::cheats::load(&path);
                    game.system.cheats.extend(enhancements);
                }
            }
            Action::CheatsFolder => {
                if let Some(folder) = self.cheats_path().and_then(|path| path.parent().map(Path::to_owned)) {
                    match std::fs::create_dir_all(&folder) {
                        Ok(()) => show_folder(&folder),
                        Err(error) => self.menus.message = Some(format!("The cheats folder can't be made: {error}")),
                    }
                }
            }
            Action::Quit => event_loop.exit(),
            Action::Settings => self.apply_settings(),
            Action::Keyboard(text, button) => {
                if let Some(game) = &mut self.game {
                    game.system.answer_keyboard(&text, button);
                }
            }
        }
    }

    fn step(&mut self, event_loop: &ActiveEventLoop) {
        // hfr: above the console's rate the frame rate loop takes over
        if self.hfr.rate > 0.0 && !self.fast_forwarding && !self.fast_forward_key {
            return self.hfr_step(event_loop);
        }
        if self.stop {
            event_loop.exit();
            return;
        }
        let now = Instant::now();
        if now > self.next_frame + CATCH_UP_LIMIT {
            self.next_frame = now;
        }
        self.library.poll();
        self.poll_jobs();
        self.poll_gamepads();
        self.qol_poll(); // qol: the Game Speed key, the speed, the boon/bane panel
        self.states_poll(); // pulse: save states
        self.info_poll(); // info-ui: the growth key
        // bottom: the panel's button, the touch cursor, the slide; a mouse
        // held still over a sliding panel follows it
        let composes = self.composes();
        // bottom2: shown while its options change
        self.bottom.preview = fates_ui::enabled() && self.fates.options.previewing_bottom();
        self.bottom.poll(&self.active, composes);
        self.bottom_check_step();
        if self.mouse_down {
            self.touch();
        }
        // a room that went away takes the game's local wireless with it
        if self.room.as_ref().is_some_and(|room| !room.open()) {
            self.leave_room();
            self.menus.message = Some("The room closed, or the connection to it was lost.".into());
        }
        self.download_play();

        // a game waiting on its keyboard waits for the user, not running
        let typing = self.game.as_ref().is_some_and(|game| game.system.keyboard_request().is_some());
        let playing = self.game.is_some() && !self.paused && !self.menus.over_game() && !typing
            // ui: the options menu and the game's message boxes stop it
            && !(fates_ui::enabled() && self.fates.pauses(self.menus.message.is_some()))
            && !pulse::holds(); // pulse: the rewind screen stops it
        // Tab or a controller's right trigger, or the right stick's click
        // while ZR has the trigger, whichever no button of the console's is on
        // input: or the fast-forward binding ([keys]/[pad] fast_forward)
        // port: no emulator fast forward; the game runs at its own speed
        let fast = playing && !fates_ui::enabled() && (self.fast_forward_key || input_api::action_held(controls::PcAction::FastForward));
        if fast {
            // the output waits out fast forward rather than run dry
            if let Some(audio) = &self.audio {
                audio.hold();
            }
        }
        if fast != self.fast_forwarding {
            self.fast_forwarding = fast;
            if !fast {
                // back at the console's pace, with no sound piled up
                if let Some(audio) = &self.audio {
                    audio.clear();
                }
                self.next_frame = Instant::now();
            }
        }
        if playing {
            let start = Instant::now();
            self.emulate();
            self.spent.emulating.add(start.elapsed());
        } else if let Some(audio) = &self.audio {
            // the sound stops with the game, which is not falling behind
            audio.hold();
        }

        self.next_frame += qol::frame_period(); // qol: at the game speed
        if fast {
            // nothing to wait for
            self.next_frame = Instant::now();
        }
        // behind the schedule, showing the frame would wait on the display,
        // so it goes unshown, a few at most. a presenter that does not wait,
        // in mailbox mode, shows them all, a game that could not keep up
        // showed a fifth of its frames. fast forward shows them at the
        // console's frame rate, waiting on the display for no more
        if trailer_dump::lockstep() {
            self.next_frame = Instant::now(); // trailer: every frame shown, at whatever pace the captures allow
        }
        let behind = Instant::now() > self.next_frame;
        let waits = self.backend.as_ref().is_some_and(Backend::waits_for_display);
        let unshown = if fast { Instant::now() < self.next_shown } else { skips_showing(playing, behind, waits, self.skipped) };
        if unshown {
            self.skipped += 1;
        } else {
            self.skipped = 0;
            let start = Instant::now();
            self.present(event_loop);
            // the next one a frame later, or right away after falling
            // behind, at the console's rate on average
            self.next_shown = (self.next_shown + FRAME_TIME).max(Instant::now() - FRAME_TIME);
            self.spent.showing.add(start.elapsed());
            self.shown += 1;
        }

        if self.last_title_update.elapsed() >= Duration::from_millis(500) {
            let shown = self.shown as f32 / self.last_title_update.elapsed().as_secs_f32();
            self.shown = 0;
            self.last_title_update = Instant::now();
            if let Some(game) = &self.game {
                let Spent { emulating, showing } = std::mem::take(&mut self.spent);
                log::debug!(
                    target: "zakuro::fps",
                    "{:.1} frames emulated and {shown:.1} shown a second, emulating took {emulating}, showing {showing}",
                    game.fps
                );
            }
            if let Some(window) = &self.window {
                let title = match &self.game {
                    // port: the window says the project's name, Haiku, nothing else
                    _ if fates_ui::enabled() => "Haiku".to_owned(),
                    Some(game) => format!("Zakuro - {} - {}", game.name, game.system.status_line()),
                    None => "Zakuro".to_owned(),
                };
                window.set_title(&title);
                // port: the game's own icon on the window and taskbar, once
                static ICON_SET: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
                if fates_ui::enabled() && WINDOW_ICON_PNG.get().is_none() && !ICON_SET.load(std::sync::atomic::Ordering::Relaxed) {
                    if let Some(smdh) = self.game.as_ref().and_then(|game| library::Smdh::parse(game.system.title.as_ref()?.exefs_file("icon")?)) {
                        let side = library::ICON_SIZE as u32;
                        if let Ok(icon) = winit::window::Icon::from_rgba(smdh.icon.clone(), side, side) {
                            window.set_window_icon(Some(icon));
                        }
                        ICON_SET.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            let underruns = self.audio.as_ref().map_or(0, |audio| audio.take_underruns());
            if underruns > 0 && playing {
                log::warn!("the sound ran dry {underruns} times, the emulation is falling behind");
            }
        }

        // native-rt: in real time the game's threads run in the wait
        if playing {
            if let Some(game) = &mut self.game {
                game.system.lend_until(self.next_frame);
            }
        }
        let now = Instant::now();
        if now < self.next_frame {
            std::thread::sleep(self.next_frame - now);
        }
    }

    /// runs one frame of the game.
    fn emulate(&mut self) {
        let Some(game) = &mut self.game else { return };
        cameras::windowed_script(game.frame); // cameras: FATES_CAMERAS_SCRIPT in a window
        qol4::headless_frame(game.frame); // boot3: FATES_QOL4_RESET_AT (tests) in a window too
        boot3::window_procs(&mut game.system.memory, game.frame); // boot3, dev: FATES_BOTTOM_PROCS in a window
        let input = match &mut game.replay {
            Some(replay) => {
                let mut input = replay.input(game.frame);
                // bottom: a checking script's touches go in over the replay
                if self.bottom_check.is_some() {
                    input.touch = input.touch.or(self.keyboard.state().touch);
                    self.bottom.apply_input(&mut input);
                }
                input
            }
            // input: keyboard and every controller together
            None => {
                let mut input = input::combine(self.keyboard.state(), &self.gamepads.output().0);
                // bottom: the controller's touch cursor
                self.bottom.apply_input(&mut input);
                self.offline_click.apply(&mut input); // offline-msg
                online_ui::apply(&mut input); // online: the code box, Cancel's B
                cameras::mask_game_input(&mut input); // cameras: the fly camera's moves
                input
            }
        };
        let mut input = input;
        // qol3, test: FATES_UI_SCRIPT_GAME=1 has a UI script's `pad` and
        // `dir` reach the game too, as a controller's would
        if let Some(script) = self.fates.script.as_ref().filter(|_| std::env::var("FATES_UI_SCRIPT_GAME").is_ok_and(|v| v == "1")) {
            input.buttons |= script.pad_held();
            if let Some([x, y]) = script.circle {
                (input.circle_x, input.circle_y) = (x, y);
            }
        }
        // qol3: nothing reaches the game while the options menu is open
        // (with the frame rate loop the game runs behind it): its moves
        // were scrolling the Main Menu behind it.
        // FATES_OPTIONS_PASS_INPUT=1 lets it through, as before
        if fates_ui::enabled() && self.fates.modal() && !std::env::var("FATES_OPTIONS_PASS_INPUT").is_ok_and(|v| v == "1") {
            input = zakuro_core::services::hid::InputState { tilt: input.tilt, ..Default::default() };
        }
        // ui: nothing from the button that closed a menu
        let mut input = fates_ui::filter_input(&mut self.fates, input);
        info_ui::frame_input(&mut game.system.memory, game.frame, &mut input, false); // info-ui: the growth button's touches
        cheats::frame(&mut game.system); // cheats: the menu's view and its actions, between two frames
        zakuro_core::tones::frame(&mut game.system.memory); // tones: a speaking style chosen in the options
        game.frame += 1;
        zakuro_core::native_gfx::HEADLESS_FRAME.store(game.frame, std::sync::atomic::Ordering::Relaxed); // gfxmodels3: the frame for dev traces in a window too
        if let Some(recorder) = &mut game.recorder {
            recorder.record(input);
        }
        game.system.set_input(input);
        let run = pulse::before_frame(&mut game.system); // pulse: a moment shown, a rewind made
        let quiet = run != pulse::Run::Normal;
        let muted = run == pulse::Run::Quiet; // pulse2: a rewind's own frame is heard
        let outcome = game.system.run_frame();
        // pulse: the moments worth keeping, what opens the rewind
        pulse::after_frame(&mut game.system, game.frame);
        let pulse_loaded = pulse::take_loaded();
        // pulse2: the sound winds down as the rewind opens, and starts over
        // (the moment's music going on, fading in) as it closes, before
        // this frame's sound goes in
        if let (Some(cue), Some(audio)) = (pulse_fx::take_cue(), &self.audio) {
            match cue {
                pulse_fx::Cue::Stop => audio.time_stop(),
                pulse_fx::Cue::Resume => audio.time_resume(),
            }
        }
        if pulse_loaded && pulse::replays_again(false) {
            game.frame = game.system.frames;
        } else if quiet {
            // the frames the rewind screen runs are not the player's: a
            // replay's next input waits for the game to go on
            game.frame -= 1;
        }
        if let Some(clock) = &mut game.clock {
            clock.frame(&mut game.system.memory); // hfr
        }
        // bottom2: what Fates shows on its lower screen, for Auto
        if self.active.pc.bottom_screen == settings::BottomScreen::Auto && game.system.title.as_ref().is_some_and(|title| zakuro_core::region::Region::of(title.program_id()).is_some()) { // eur: and the EUR build; jpprep: and the Japanese
            self.bottom.read_game(&mut game.system.memory, game.frame);
        }
        haiku_update::game_frame(&mut game.system.memory, game.frame); // updater: the Main Menu, only while an update's notice waits
        // the overlay reads the game the program was built on, not another
        // one its library opened
        let ours = self.linked.as_ref().is_none_or(|linked| game.system.title.as_ref().is_some_and(|title| title.program_id() == linked.program_id()));
        if let Some(overlay) = self.overlay.as_mut().filter(|_| ours) {
            let text = overlay.frame(&mut game.system.memory);
            self.overlay_corner = text.corner;
            if let Some(notice) = text.notice {
                self.menus.toast = Some((notice, Instant::now()));
            }
        }
        game.count_frame();
        let sound = game.system.take_audio();
        // fast forward plays no sound, there is too much of it too soon
        // qol: nor a game speed above what is time-stretched
        let heard = self.qol.heard();
        if let (Some(audio), false) = (&self.audio, self.fast_forwarding || !heard || muted) { // pulse: quiet
            audio.push(&sound);
        } else if let (Some(audio), false) = (&self.audio, heard) {
            audio.hold();
        }
        match outcome {
            FrameOutcome::Completed => {}
            FrameOutcome::Exited => {
                log::info!("the title exited");
                self.back_to_library();
            }
            FrameOutcome::Faulted => {
                log::error!("the title stopped on a fault");
                let errors = game.system.fatal_errors.join("; ");
                if !errors.is_empty() {
                    log::error!("{errors}");
                }
                let message = format!("The game stopped on a fault. {errors}");
                self.menus.report = Some((message.clone(), report::text(&game.name, &game.system)));
                self.menus.message = Some(message);
                self.back_to_library();
            }
        }
        // pulse: a moment was loaded: the pictures' pacing and the sound
        // start over, as after a loaded state
        if pulse_loaded {
            self.hfr_apply();
            self.hfr.reset();
            // pulse2: the sound already started over (time_resume) when the
            // rewind closed; a moment shown keeps it stopped
        }
    }

    /// the name the player goes by, the console's too.
    fn nickname(&self) -> String {
        match self.settings.nickname.trim() {
            "" => "Player".into(),
            name => name.chars().take(10).collect(),
        }
    }

    /// gives the running game the room the player is in, and tells the room
    /// the game.
    fn link_room(&mut self) {
        let Some(game) = &mut self.game else { return };
        game.system.room = self.room.as_ref().map(|room| room.link());
        if let Some(room) = &self.room {
            room.playing(&game.name);
        }
    }

    /// opens a room on the port chosen, and joins it.
    fn host_room(&mut self) {
        let name = match self.settings.room_name.trim() {
            "" => format!("{}'s room", self.nickname()),
            name => name.to_owned(),
        };
        let password = self.menus.room_password.clone();
        match zakuro_core::room::Server::start(self.settings.room_port, &name, &password, 16) {
            Ok(server) => {
                let address = format!("127.0.0.1:{}", server.port());
                self.room_server = Some(server);
                self.join_room(&address);
                if self.room.is_none() {
                    self.room_server = None;
                }
            }
            Err(error) => self.menus.message = Some(format!("The room could not be opened on port {}, {error}", self.settings.room_port)),
        }
    }

    /// joins the room at address.
    fn join_room(&mut self, address: &str) {
        let password = self.menus.room_password.clone();
        match zakuro_core::room::Connection::join(address.trim(), &self.nickname(), &password) {
            Ok(room) => {
                self.room = Some(room);
                self.link_room();
            }
            Err(error) => self.menus.message = Some(format!("Could not join the room, {error}.")),
        }
    }

    /// leaves the room, and closes it when it is this program's.
    fn leave_room(&mut self) {
        self.dlp_guest = None;
        self.room = None;
        self.room_server = None;
        if let Some(game) = &mut self.game {
            game.system.room = None;
        }
    }

    /// Download Play as a guest: hears the offers in the room while no game
    /// runs, and starts the child once the host's game begins.
    fn download_play(&mut self) {
        let Some(room) = self.room.as_ref().filter(|_| self.game.is_none()) else {
            self.dlp_guest = None;
            return;
        };
        let guest = self.dlp_guest.get_or_insert_with(|| zakuro_core::download_play::Guest::new(room.link()));
        guest.poll();
        match guest.end() {
            Some(zakuro_core::download_play::Phase::Begun { child, reboot }) => {
                let own = self.dlp_own.take();
                let path = match child {
                    Some(child) => self.keep_child(&child),
                    None => own.ok_or_else(|| "the game to download went missing".to_owned()),
                };
                let started = path.and_then(|path| {
                    self.wireless_reboot = Some(reboot);
                    self.play(&path)
                });
                if let Err(error) = started {
                    self.wireless_reboot = None;
                    self.menus.message = Some(format!("Download Play could not start the game, {error}."));
                }
            }
            Some(zakuro_core::download_play::Phase::Failed(why)) => {
                self.dlp_own = None;
                self.menus.message = Some(format!("Download Play stopped, {why}."));
            }
            _ => {}
        }
    }

    /// takes part in the Download Play offer at index, with the player's own
    /// copy of the child when the folder has one.
    fn join_download_play(&mut self, index: usize) {
        let node = zakuro_core::download_play::node(self.settings.console_id, &self.nickname());
        let Some(guest) = &mut self.dlp_guest else { return };
        let Some(heard) = guest.offers().get(index) else { return };
        let (host, child) = (heard.host, heard.offer.child);
        self.dlp_own = self.library.children.iter().find(|own| own.title_id == child).map(|own| own.path.clone());
        guest.join(host, node, self.dlp_own.is_some());
    }

    /// keeps a child the host sent, for it to run from.
    fn keep_child(&self, child: &[u8]) -> Result<PathBuf, String> {
        let cia = zakuro_fs::Cia::read(child).map_err(|_| "the host sent something that isn't a game".to_owned())?;
        let folder = self.data_dir.clone().unwrap_or_else(std::env::temp_dir).join("dlp");
        let path = folder.join(format!("{:016X}.cia", cia.title_id));
        std::fs::create_dir_all(&folder).and_then(|()| std::fs::write(&path, child)).map_err(|error| error.to_string())?;
        Ok(path)
    }

    /// the file of a slot of the running game's saved states.
    fn state_path(&self, slot: u8) -> Option<PathBuf> {
        let program_id = self.game.as_ref()?.system.title.as_ref()?.program_id();
        Some(haiku_codemods::states_base(self.data_dir.as_ref()?).join("states").join(format!("{program_id:016X}")).join(format!("{slot}.state"))) // codemods2: beside the saves
    }

    /// saves the running game's state to the slot chosen.
    fn save_state(&mut self) {
        let slot = self.menus.slot;
        let Some(path) = self.state_path(slot) else { return };
        let Some(game) = &mut self.game else { return };
        let started = Instant::now();
        let saved = game.system.save_state().and_then(|bytes| {
            let parent = path.parent().expect("a state's file is in a folder");
            std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
            // written beside it first, a state cut short never takes the
            // place of a whole one
            let partial = path.with_extension("state.new");
            std::fs::write(&partial, &bytes).and_then(|()| std::fs::rename(&partial, &path)).map_err(|error| error.to_string())
        });
        match saved {
            Ok(()) => {
                log::info!("saved the state to {} in {:?}", path.display(), started.elapsed());
                self.menus.toast = Some((format!("Saved to slot {slot}"), Instant::now()));
            }
            Err(error) => self.menus.message = Some(format!("The state could not be saved, {error}")),
        }
        // the time it took is not the game's to make up
        self.next_frame = Instant::now();
    }

    /// loads the running game's state from the slot chosen.
    fn load_state(&mut self) {
        let slot = self.menus.slot;
        let Some(path) = self.state_path(slot) else { return };
        let Some(game) = &mut self.game else { return };
        let loaded = std::fs::read(&path)
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => format!("Slot {slot} has no state saved."),
                _ => format!("The state could not be read, {error}"),
            })
            .and_then(|bytes| statecheck::load_raw(&mut game.system, &bytes, &path.display().to_string()).map(|_| ()).map_err(|error| format!("The state could not be loaded, {error}"))); // statecheck: the layer checked first
        match loaded {
            Ok(()) => {
                log::info!("loaded the state from {}", path.display());
                self.menus.toast = Some((format!("Loaded slot {slot}"), Instant::now()));
                self.menus.menu_open = false;
                // the overlay's line was about the memory before, the next
                // frame gives the one about the state
                self.overlay_corner = None;
                if let Some(audio) = &self.audio {
                    audio.state_loaded(); // statesnd: the state's sound fades in
                }
            }
            Err(error) => self.menus.message = Some(error),
        }
        self.next_frame = Instant::now();
    }

    /// the file of the running game's cheats.
    fn cheats_path(&self) -> Option<PathBuf> {
        let program_id = self.game.as_ref()?.system.title.as_ref()?.program_id();
        Some(zakuro_core::cheats::path(self.data_dir.as_ref()?, program_id))
    }

    /// writes the running game's cheats to their file.
    fn save_cheats(&mut self) {
        let (Some(path), Some(game)) = (self.cheats_path(), &self.game) else { return };
        if let Err(error) = zakuro_core::cheats::save(&path, &game.system.cheats) {
            self.menus.message = Some(format!("The cheats can't be saved: {error}"));
        }
    }

    /// the menus, and the actions they asked for, done.
    fn interface(&mut self, event_loop: &ActiveEventLoop) -> Overlay {
        // ui: this title shows only its own interface, drawn with Fates' art
        if fates_ui::enabled() {
            return self.fates_interface(event_loop);
        }
        // when the slot chosen was saved to, while the menu shows it
        let saved = self
            .menus
            .menu_open
            .then(|| self.state_path(self.menus.slot))
            .flatten()
            .and_then(|path| std::fs::metadata(path).ok()?.modified().ok())
            .map(ago);
        let program_id = self.program_id();
        let (Some(gui), Some(window)) = (&mut self.gui, &self.window) else { return Overlay::default() };
        let show_fps = self.settings.show_fps;
        let game = self.game.as_ref().map(|game| (game.name.clone(), game.fps, game.system.recompiled.is_some()));
        let fast = self.fast_forwarding;
        let keyboard = self.game.as_ref().and_then(|game| game.system.keyboard_request().cloned());
        let room = self.room.as_ref().filter(|_| self.menus.multiplayer_open).map(|connection| {
            let members = connection.members();
            menus::RoomView {
                name: connection.room().to_owned(),
                download_play: self.dlp_guest.as_ref().map(|guest| download_play_view(guest, &members, &self.library)),
                members: members.into_iter().map(|member| (member.nickname, member.game)).collect(),
                port: self.room_server.as_ref().map(|server| server.port()),
            }
        });
        let cheats = self.game.as_ref().filter(|_| self.menus.cheats_open).map(|game| {
            game.system.cheats.iter().map(|cheat| (cheat.name.clone(), cheat.enabled, cheat.notes.join("\n"), cheat.builtin)).collect::<Vec<_>>()
        });
        let corner = self.overlay_corner.clone().filter(|_| game.is_some());
        let (menus, library, settings, jobs, waiting) = (&mut self.menus, &self.library, &mut self.settings, &self.jobs, &self.waiting);
        let mut actions = Vec::new();
        let overlay = gui.frame(window, |ui| {
            match &game {
                Some((name, fps, recompiled)) => {
                    actions.extend(menus.game(ui, name, program_id, show_fps.then_some(*fps), *recompiled, fast, jobs, waiting.len(), saved.as_deref()));
                    if let Some(text) = &corner {
                        menus::corner(ui.ctx(), text);
                    }
                }
                None => actions.extend(menus.library(ui, library, settings, jobs, waiting)),
            }
            let own = program_id.is_some_and(|id| settings.has_own(id));
            actions.extend(menus.settings(ui.ctx(), settings, own));
            actions.extend(menus.game_settings(ui.ctx(), settings));
            if menus.multiplayer_open {
                actions.extend(menus.multiplayer(ui.ctx(), settings, room.as_ref()));
            }
            if let Some(cheats) = &cheats {
                actions.extend(menus.cheats(ui.ctx(), cheats));
            }
            if let Some(request) = &keyboard {
                actions.extend(menus.keyboard(ui.ctx(), request));
            }
            actions.extend(menus.compiler_offer(ui.ctx()));
            menus.message(ui.ctx());
        });
        self.library.changed = false;
        for action in actions {
            self.act(action, event_loop);
        }
        overlay
    }

    fn present(&mut self, event_loop: &ActiveEventLoop) {
        let t0 = Instant::now();
        let overlay = self.interface(event_loop);
        let t1 = Instant::now();
        let blank = |screen: Screen| ((std::sync::Arc::new(Vec::new()), screen.width(), screen.height()), None);
        // the right eye's picture only while the 3D shows
        let stereo = self.active.stereo != settings::Stereo3d::Off;
        let (top, bottom, right) = match &mut self.game {
            Some(game) => (
                shown(&mut game.system, Screen::Top),
                shown(&mut game.system, Screen::Bottom),
                stereo.then(|| shown_right(&mut game.system)).flatten(),
            ),
            None => (blank(Screen::Top), blank(Screen::Bottom), None),
        };
        let right = right.unwrap_or_else(|| blank(Screen::Top));
        // smooth2: a cross-fade (hfr paced_pass): the top screen above is
        // the picture before, the newest goes over it this far
        let crossfade = self.hfr.crossfade.take().and_then(|t| {
            let game = self.game.as_mut()?;
            game.system.show_layer(0);
            let ((_, _, _), newest) = shown(&mut game.system, Screen::Top);
            newest.map(|newest| (newest, t))
        });
        if let Some(backend) = &mut self.backend {
            backend.set_crossfade(crossfade);
            pulse_fx::presented();
            backend.set_pulse_effect(pulse_fx::effect()); // pulse2
            pulse_fx::set_drawn(backend.pulse_effects());
        }
        // pulse2: FATES_OUTPUT_WAV: a presented frame's worth of the output
        if let Some(audio) = self.audio.as_ref().filter(|audio| audio.recording()) {
            static FPS: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
            let fps = *FPS.get_or_init(|| std::env::var("FATES_OUTPUT_WAV_FPS").ok().and_then(|v| v.parse().ok()).unwrap_or(60.0));
            audio.record_frame(1.0 / fps);
        }
        let t2 = Instant::now();
        // wide: the shape the settings and the window ask for, taken by the
        // GPU once it settles
        if self.game.is_some() {
            let window = self.window.as_ref().map(|w| (w.inner_size().width, w.inner_size().height));
            let window = wide::room(window, &self.active, self.composes());
            let display = self.window.as_ref().and_then(|w| w.current_monitor()).map(|m| (m.size().width, m.size().height));
            wide::tick(self.active.pc.aspect, window, display);
            wide::sides(self.active.pc.wide_sides); // wide-ui
            zakuro_core::native_gfx::set_msaa(self.active.pc.msaa); // msaa: a change in the menu takes at once
        }
        // bottom: the frontend composes the window around the bottom screen
        let composes = self.composes();
        if let (Some(window), Some(backend)) = (&self.window, &mut self.backend) {
            let size = window.inner_size();
            let image = ((top.0).1, (top.0).2);
            let classic = layout(size.width, size.height, self.layout.screens(), self.orientation.screen_orientation(), wide::top_width(self.top_shape.width()), self.active.integer_scale).1
                .filter(|_| self.orientation == settings::Orientation::North);
            let composition = self.bottom.compose((size.width, size.height), &self.active, composes, image, classic);
            // qol: where the top screen goes, in points, for the boon/bane panel
            let points = window.scale_factor() as f32;
            let (classic_top, classic_bottom) = layout(size.width, size.height, self.layout.screens(), self.orientation.screen_orientation(), wide::top_width(self.top_shape.width()), self.active.integer_scale); // info-ui: both
            let top = match composition.as_ref().and_then(|c| c.screens) {
                Some((top, _)) => top,
                None => classic_top,
            };
            self.qol.top_screen = top.map(|v| egui::Rect::from_min_size(egui::pos2(v.x / points, v.y / points), egui::vec2(v.width / points, v.height / points)));
            // info-ui: and the lower screen, for the growth button (upright only)
            let bottom = match composition.as_ref().and_then(|c| c.screens) {
                Some((_, bottom)) => bottom,
                None => classic_bottom,
            };
            let upright = self.orientation == settings::Orientation::North;
            info_ui::with_state(|state| state.bottom_screen = bottom.filter(|_| upright).map(|v| egui::Rect::from_min_size(egui::pos2(v.x / points, v.y / points), egui::vec2(v.width / points, v.height / points))));
            backend.set_composition(composition);
            // bottom2: the options menu steps aside from the window it places
            let points = window.scale_factor() as f32;
            self.fates.keep_clear = self.bottom.preview_frame().map(|f| egui::Rect::from_min_size(egui::pos2(f.x / points, f.y / points), egui::vec2(f.width / points, f.height / points)));
        }
        let t3 = Instant::now();
        let Some(backend) = &mut self.backend else { return };
        // wide: the shape the GPU took, known once the game drew
        backend.set_top_width(wide::top_width(self.active.top_shape.width()));
        let (((top, top_width, top_height), top_gpu), ((bottom, bottom_width, bottom_height), bottom_gpu)) = (top, bottom);
        let ((right, right_width, right_height), right_gpu) = right;
        let result = backend.present(
            ScreenImage { width: top_width, height: top_height, pixels: &top, gpu: top_gpu },
            ScreenImage { width: bottom_width, height: bottom_height, pixels: &bottom, gpu: bottom_gpu },
            ScreenImage { width: right_width, height: right_height, pixels: &right, gpu: right_gpu },
            &overlay,
        );
        if std::env::var_os("FATES_DEBUG_PRESENT").is_some() {
            let t4 = Instant::now();
            log::warn!("present-time: ui {:?} screens {:?} layout {:?} backend {:?}", t1 - t0, t2 - t1, t3 - t2, t4 - t3);
        }
        hitches::frame_shown(t0, t1, t3); // hitches: a picture that took long to show says why
        let presented = result.is_ok(); // qol4
        match result {
            Ok(()) | Err(PresentError::OutOfDate) => {}
            Err(error) => {
                log::error!("presentation failed: {error}");
                event_loop.exit();
            }
        }
        // qol4: the window shows once a picture is in it
        if let Some(window) = &self.window {
            qol4::reveal(window, presented);
        }
    }
}

/// what the multiplayer window shows of Download Play: the offers in the
/// room, and where the guest is.
fn download_play_view(guest: &zakuro_core::download_play::Guest, members: &[zakuro_core::room::Member], library: &Library) -> menus::DlpView {
    use zakuro_core::download_play::Phase;
    let offers = guest
        .offers()
        .iter()
        .map(|heard| {
            let offer = &heard.offer;
            let smdh = library::Smdh::parse(&offer.smdh);
            let own = library.children.iter().any(|own| own.title_id == offer.child);
            let blocked = if offer.guests >= offer.most {
                Some("The host's game takes no more guests.".to_owned())
            } else if !offer.sends_child && !own {
                Some(
                    "The game to download is encrypted on the card, so either player needs a decrypted copy of it: \
                     decrypt the .cia in the card's Download Play partition with GodMode9 and put it in the games folder."
                        .to_owned(),
                )
            } else {
                None
            };
            menus::OfferView {
                child: offer.child,
                title: smdh.as_ref().map(|smdh| smdh.name.clone()).filter(|name| !name.is_empty()).unwrap_or_else(|| format!("{:016X}", offer.child)),
                host: members.iter().find(|member| member.mac == heard.host).map(|member| member.nickname.clone()).unwrap_or_default(),
                icon: smdh.map(|smdh| smdh.icon),
                guests: offer.guests,
                most: offer.most,
                blocked,
            }
        })
        .collect();
    let doing = guest.phase().map(|phase| match phase {
        Phase::Asking => ("Asking the host to join.".to_owned(), None),
        Phase::Waiting => ("Joined. The game downloads once the host starts it.".to_owned(), None),
        Phase::Receiving { got, total } => ("Downloading the game.".to_owned(), Some(*got as f32 / (*total).max(1) as f32)),
        Phase::Ready => ("Ready. Waiting for the host to start the game.".to_owned(), None),
        Phase::Begun { .. } => ("Starting the game.".to_owned(), None),
        Phase::Failed(why) => (why.clone(), None),
    });
    menus::DlpView { offers, doing }
}

/// a screen as the presenter takes it, straight from the GPU when it shares
/// the renderer's device and the GPU drew what the screen shows, else its
/// pixels.
fn shown(system: &mut System, screen: Screen) -> (zakuro_core::Screen, Option<GpuScreen>) {
    // gfx: the native renderer composes on the GPU when it shares the
    // presenter's device, else on the CPU path
    if zakuro_core::native_gfx::active() {
        if let Some((gpu, width, height)) = zakuro_core::native_gfx::gpu_screen(system, screen) {
            return ((std::sync::Arc::new(Vec::new()), width, height), Some(gpu));
        }
        return (system.read_screen_scaled(screen), None);
    }
    if let Some(gpu) = system.gpu_screen(screen) {
        let scale = system.gpu.scale();
        // wide: the top screen's picture is wider than the console's
        let width = match screen {
            Screen::Top => (screen.width() as f32 * scale as f32 * zakuro_gpu::wide::factor()).round() as u32,
            _ => screen.width() * scale,
        };
        return ((std::sync::Arc::new(Vec::new()), width, screen.height() * scale), Some(gpu));
    }
    static WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if std::env::var_os("FATES_DEBUG_PRESENT").is_some() && !WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        log::warn!("present: {screen:?} has no GPU picture, reading it back on the CPU (slow)");
    }
    (system.read_screen_scaled(screen), None)
}

/// the top screen's picture for the right eye, as shown gives the screens,
/// when the game draws in 3D.
fn shown_right(system: &mut System) -> Option<(zakuro_core::Screen, Option<GpuScreen>)> {
    if let Some(gpu) = system.gpu_right_eye() {
        let scale = system.gpu.scale();
        return Some(((std::sync::Arc::new(Vec::new()), Screen::Top.width() * scale, Screen::Top.height() * scale), Some(gpu)));
    }
    system.read_right_eye_scaled().map(|screen| (screen, None))
}

/// keeps the renderer kind referenced even when a backend feature is off.
const _: RendererKind = RendererKind::Software;

#[cfg(test)]
mod tests {
    use super::*;

    /// a frame goes unshown only while the game runs behind with a
    /// presenter that waits for the display, a few in a row at most.
    #[test]
    fn frames_go_unshown_only_when_showing_would_wait() {
        assert!(skips_showing(true, true, true, 0));
        assert!(skips_showing(true, true, true, MAX_SKIPPED - 1));
        assert!(!skips_showing(true, true, true, MAX_SKIPPED), "a few in a row at most");
        assert!(!skips_showing(true, true, false, 0), "mailbox shows every frame");
        assert!(!skips_showing(true, false, true, 0), "on time");
        assert!(!skips_showing(false, true, true, 0), "paused");
    }
}
