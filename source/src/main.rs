//! Fire Emblem Fates Special Edition USA, recompiled ahead of time by 3dsrecomp and run by Zakuro. it
//! still reads the game for everything but the code, so it takes the path
//! to a copy of it the way Zakuro does.
//!
//! oneexe: Haiku.exe is the one program a player runs. It holds the engine,
//! Haiku's overrides (compiled in by build.rs) and its setup (haiku-setup,
//! once Haiku Setup.exe). The game's code is a module the setup recompiles
//! on the player's PC from their own game (data/code/game.dll), which this
//! loads and binds at start (module.rs). The first launch, and any launch
//! that finds the module or Haiku's game-derived files out of date for this
//! Haiku.exe, opens the setup's window instead, which builds what is
//! needed and starts Haiku.exe again to play.

// qol3: a Windows program, so no console window opens beside the game;
// console.rs keeps redirected and console output working
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::ffi::c_void;


mod console; // qol3
mod install;
mod module; // oneexe
mod updscreen; // updscreen: the game's code built in the game's window

// codemods: Haiku's overrides by address and file (build.rs)
include!(concat!(env!("OUT_DIR"), "/override_names.rs"));

extern "C" {
    // oneexe: the game's code and its tables (recomp_entries, the tails
    // registry...) are in the game's module now, bound at start (module.rs)
    // pulsefix: the overrides' own variables that are part of the console's
    // state (overrides/keep.c, in this exe)
    fn haiku_keep_regions(out: *mut *const zakuro_core::state::keep::Region) -> u32;
    // wide: overrides/widescreen.c asks the host for the top screen's aspect
    static mut fates_wide_aspect_hook: Option<extern "C" fn() -> f32>;
    // wide2: and tells the native renderer of the wallpapers it stretched
    static mut fates_wide2_rect_hook: Option<extern "C" fn(u32, u32, f32, f32)>;
    // gfx-2d: overrides/gfx_units.c, map sprites snapped at the render scale
    static mut fates_units_snap_hook: Option<extern "C" fn() -> i32>;
    // hfr: overrides/timing.c
    static mut fates_hfr_max_step: u32;
    static fates_hfr_scene_step: u32;
    // os: the native OS bridge, defined by overrides/os_fs.c
    static mut os_native_hook: *const c_void;
    static os_native_abi: u32;
    // gfx: the hook table overrides/gfx_native.c calls through
    static mut fates_gfx_hooks: fates_gfx::Hooks;
    static mut fates_gfx_hooks_3d: fates_gfx::Hooks3D;
    // gfx-shadow: overrides/gfx_shadow.c
    static mut fates_gfx_hooks_shadow: fates_gfx::shadow::Hooks;
    // gfx-post: overrides/gfx_post.c
    static mut fates_gfx_hooks_post: fates_gfx::post::Hooks;
    // gfx-eft: overrides/gfx_eft.c
    static mut fates_gfx_hooks_eft: fates_gfx::HooksEft;
    // gfx-terrain: overrides/gfx_scene2d.c
    static mut fates_gfx_hooks_scene2d: fates_gfx::scene2d::Hooks;
    // gfx-gaps: overrides/gfx_unit_icon.c
    static mut fates_gfx_hooks_unit_icon: fates_gfx::unit_icon::Hooks;
    // gfx-final: overrides/gfx_capture.c
    static mut fates_gfx_hooks_capture: fates_gfx::capture::Hooks;
    // jpn: overrides/gfx_live2d.c, Live2D models (My Room) drawn natively
    static mut fates_gfx_hooks_live2d: fates_gfx::live2d::Hooks;
    // mods: overrides/mods.c, the Main Menu's Mods entry
    static mut haiku_mods_hook: Option<unsafe extern "C" fn(*mut c_void, u32)>;
    // randomizer: overrides/randomizer.c, the New Game randomizer
    static mut haiku_rando_hook: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u32) -> u32>;
    static mut haiku_rando_active: u32;
    // supports: overrides/supports.c, Corrin S-supports anyone, Kana's gender
    static mut haiku_supports_hook: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u32) -> u32>;
    static mut haiku_supports_text: u32;
    // tones: overrides/tones.c, Corrin's speaking style with a translation
    static mut haiku_tones_hook: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u32) -> u32>;
    static mut haiku_tones_active: u32;
    // jpn2: overrides/jpn_text.c, Haiku's menu entries in English under a translation on the Japanese game
    static mut haiku_jpn_english: u32;
    // cameras: overrides/cameras.c, the battle free camera and castle walking
    static mut fates_cameras_hook: Option<unsafe extern "C" fn(*mut zakuro::cameras::CamFrame)>;
    // qol: overrides/qol.c, Skip Story, the SpotPass questions, the boon/bane panel
    static mut qol_skip_story: u32;
    static mut qol_skip_spotpass: u32;
    static mut qol_show_choices: u32;
    static mut qol_skip_cutscenes: u32;
    static mut qol_choice_mode: u32;
    static mut qol_skips_started: u32;
    static mut qol_cutscenes_skipped: u32;
    static mut qol_choices_shown: u32;
    static mut qol_spotpass_answered: u32;
    static mut qol_chara_draws: u32;
    static mut qol_chara_selected: u32;
    static mut qol_chara_selected_at: u32;
    static mut qol_chara_boon: u32;
    static mut qol_chara_bane: u32;
    static mut qol_chara_boon_text: u32;
    static mut qol_chara_bane_text: u32;
    static mut qol_movies_skipped: u32;
    static mut qol_skip_tutorial_maps: u32;
    static mut qol_skip_supports: u32;
    // info-ui: overrides/info_ui.c's words (targeting lines, growth rates)
    static mut info_ui_words: [u32; 512];
    // cheats: overrides/cheats.c's words (the Cheats tab)
    static mut cheats_words: [u32; 1024];
    // pulse: overrides/pulse.c's words (Divine Pulse)
    static mut pulse_words: [u32; 64];
    static mut qol_story_types: u32;
    static mut qol_castle_items: u32;
    static mut qol_dialog_open: u32;
    static mut qol_skip_ours: u32;
    static mut qol_quit_request: u32;
    // qol3: the map's opening talk, the title menu's Exit to Desktop
    static mut qol_opening_types: u32;
    static mut qol_title_exit: u32;
    static mut qol_title_exit_request: u32;
    static mut qol_title_plain: u32;
    static mut qol_title_plain_shown: u32;
    // qol4: Options in the title menu, the map menu's items, the soft reset,
    // the extra data's message
    static mut qol_title_options: u32;
    static mut qol_title_options_request: u32;
    static mut qol_map_items: u32;
    static mut qol_soft_reset: u32;
    static mut qol_restart_request: u32;
    static mut qol_restarts: u32;
    static mut qol_ext_read_quiet: u32;
    static mut qol_ext_read_hidden: u32;
    static mut boot3_procthread_guard: u32; // boot3 (overrides/boot3.c)
    static mut boot3_procthread_held: u32;
    static mut boot3_procthread_sleeps: u32;
    static mut boot3_procthread_gave_up: u32;
    // transblur: overrides/transblur.c, the battle transitions' zoom blur
    static mut haiku_transition_blur: u32;
    static mut haiku_transition_unblurred: u32;
    // drawdist: overrides/drawdist.c, Full Draw Distance
    static mut drawdist_full: u32;
    // castlegifts: overrides/castlegifts.c, Online Rewards
    static mut castlegifts_on: u32;
    static mut castlegifts_unlocked: u32;
    static mut castlegifts_presents: u32;
    // bossmusic: overrides/bossmusic.c, Boss Music Stays, and its counts
    static mut bossmusic_on: u32;
    static mut bossmusic_engaged: u32;
    static mut bossmusic_kept: u32;
    static mut bossmusic_played: u32;
    static mut bossmusic_held: u32;
    static mut bossmusic_back: u32;
    static mut drawdist_batch: fates_gfx::drawdist::Batch;
}

// qol: where each of overrides/qol.c's words is
fn qol_word(word: zakuro::qol::Word) -> *mut u32 {
    use std::ptr::addr_of_mut;
    use zakuro::qol::Word;
    // SAFETY: the addresses of plain words of the linked game build
    unsafe {
        match word {
            Word::SkipStory => addr_of_mut!(qol_skip_story),
            Word::SkipSpotpass => addr_of_mut!(qol_skip_spotpass),
            Word::ShowChoices => addr_of_mut!(qol_show_choices),
            Word::SkipCutscenes => addr_of_mut!(qol_skip_cutscenes),
            Word::ChoiceMode => addr_of_mut!(qol_choice_mode),
            Word::SkipsStarted => addr_of_mut!(qol_skips_started),
            Word::CutscenesSkipped => addr_of_mut!(qol_cutscenes_skipped),
            Word::ChoicesShown => addr_of_mut!(qol_choices_shown),
            Word::SpotpassAnswered => addr_of_mut!(qol_spotpass_answered),
            Word::CharaDraws => addr_of_mut!(qol_chara_draws),
            Word::CharaSelected => addr_of_mut!(qol_chara_selected),
            Word::CharaSelectedAt => addr_of_mut!(qol_chara_selected_at),
            Word::CharaBoon => addr_of_mut!(qol_chara_boon),
            Word::CharaBane => addr_of_mut!(qol_chara_bane),
            Word::CharaBoonText => addr_of_mut!(qol_chara_boon_text),
            Word::CharaBaneText => addr_of_mut!(qol_chara_bane_text),
            Word::MoviesSkipped => addr_of_mut!(qol_movies_skipped),
            Word::SkipTutorialMaps => addr_of_mut!(qol_skip_tutorial_maps),
            Word::SkipSupports => addr_of_mut!(qol_skip_supports),
            Word::StoryTypes => addr_of_mut!(qol_story_types),
            Word::CastleItems => addr_of_mut!(qol_castle_items),
            Word::DialogOpen => addr_of_mut!(qol_dialog_open),
            Word::SkipOurs => addr_of_mut!(qol_skip_ours),
            Word::QuitRequest => addr_of_mut!(qol_quit_request),
            Word::OpeningTypes => addr_of_mut!(qol_opening_types), // qol3
            Word::TitleExit => addr_of_mut!(qol_title_exit),
            Word::TitleExitRequest => addr_of_mut!(qol_title_exit_request),
            Word::TitlePlain => addr_of_mut!(qol_title_plain),
            Word::TitlePlainShown => addr_of_mut!(qol_title_plain_shown),
            Word::TitleOptions => addr_of_mut!(qol_title_options), // qol4
            Word::TitleOptionsRequest => addr_of_mut!(qol_title_options_request),
            Word::MapItems => addr_of_mut!(qol_map_items),
            Word::SoftReset => addr_of_mut!(qol_soft_reset),
            Word::RestartRequest => addr_of_mut!(qol_restart_request),
            Word::Restarts => addr_of_mut!(qol_restarts),
            Word::ExtReadQuiet => addr_of_mut!(qol_ext_read_quiet),
            Word::ExtReadHidden => addr_of_mut!(qol_ext_read_hidden),
            Word::ProcThreadGuard => addr_of_mut!(boot3_procthread_guard), // boot3
            Word::ProcThreadHeld => addr_of_mut!(boot3_procthread_held),
            Word::ProcThreadSleeps => addr_of_mut!(boot3_procthread_sleeps),
            Word::ProcThreadGaveUp => addr_of_mut!(boot3_procthread_gave_up),
            Word::TransitionBlur => addr_of_mut!(haiku_transition_blur), // transblur
            Word::TransitionUnblurred => addr_of_mut!(haiku_transition_unblurred),
            Word::FullDrawDistance => addr_of_mut!(drawdist_full), // drawdist
            Word::OnlineRewards => addr_of_mut!(castlegifts_on), // castlegifts
            Word::RewardsUnlocked => addr_of_mut!(castlegifts_unlocked),
            Word::PresentsGiven => addr_of_mut!(castlegifts_presents),
            Word::BossMusic => addr_of_mut!(bossmusic_on), // bossmusic
            Word::BossEngaged => addr_of_mut!(bossmusic_engaged),
            Word::BossKept => addr_of_mut!(bossmusic_kept),
            Word::BossPlayed => addr_of_mut!(bossmusic_played),
            Word::BossHeld => addr_of_mut!(bossmusic_held),
            Word::BossBack => addr_of_mut!(bossmusic_back),
        }
    }
}

// info-ui: where overrides/info_ui.c's words are
fn info_ui_words_ptr() -> *mut u32 {
    // SAFETY: the address of a static array of the game build
    std::ptr::addr_of_mut!(info_ui_words).cast::<u32>()
}

// pulse: where overrides/pulse.c's words are
fn pulse_words_ptr() -> *mut u32 {
    // SAFETY: the address of a static array of the game build
    std::ptr::addr_of_mut!(pulse_words).cast::<u32>()
}

// cheats: where overrides/cheats.c's words are
fn cheats_words_ptr() -> *mut u32 {
    // SAFETY: the address of a static array of the game build
    std::ptr::addr_of_mut!(cheats_words).cast::<u32>()
}

// hfr: the game's timing overrides, for the frontend's frame rate settings
fn hfr_set_max_step(step: u32) {
    // SAFETY: a plain word the game's code reads on the same thread
    unsafe { std::ptr::write_volatile(std::ptr::addr_of_mut!(fates_hfr_max_step), step) }
}
fn hfr_scene_step() -> u32 {
    // SAFETY: as above
    unsafe { std::ptr::read_volatile(std::ptr::addr_of!(fates_hfr_scene_step)) }
}

// native-final: the shipped build is pure; emulation is for development
// comparisons only
#[cfg(all(feature = "pure", feature = "emulation"))]
compile_error!("`pure` (the default) and `emulation` exclude each other: build with --no-default-features --features emulation");

fn main() {
    console::attach(); // qol3: before anything is printed
    console::wait_for_restart(); // qol3: the Mods screen's restart
    // Portable, process-local storage. This runs before the runtime creates
    // threads and does not change the user's Windows environment.
    // FATES_ROOT overrides the portable folder, for development runs
    // versions: HAIKU_HOME first, which this program sets for the
    // processes it starts (a restart, the setup) when FATES_ROOT is a
    // version's folder (below)
    let root = match std::env::var_os("HAIKU_HOME").or_else(|| std::env::var_os("FATES_ROOT")) {
        Some(root) => std::path::PathBuf::from(root),
        None => std::env::current_exe().expect("executable location")
            .parent().expect("executable directory").to_path_buf(),
    };
    // oneexe: what the game's module has to be built for (build.rs)
    haiku_setup::module::set_spec(module::spec());
    // oneexe: the setup's command line (it was Haiku Setup.exe's):
    // --build, --check-game, --check-dlc, --voices, --voices-dlc,
    // --apply-derived, --rebuild, --setup...
    let args: Vec<String> = std::env::args().skip(1).collect();
    // updater: Haiku's version (source/VERSION), and its update commands
    // (--verify-update, --update-check)
    zakuro::haiku_update::set_version(env!("HAIKU_RELEASE_VERSION"));
    if let Some(code) = zakuro::haiku_update::cli(&args) {
        std::process::exit(code);
    }
    if let Some(code) = haiku_setup::cli(&args) {
        std::process::exit(code);
    }
    // os: `Haiku --install <dump> [dlc.cia ...]` extracts the game into
    // data/ once; `--verify-data` checks data/ against the dump
    if let Some(code) = install::run(&root) {
        std::process::exit(code);
    }
    // oneexe: an earlier Haiku's folder extracted over (upgrade.rs), then
    // the setup when something has to be made, then the game's module
    // updscreen: or the game's code to build first, in the game's window
    let start = setup_and_bind(&root, &args);
    let game_dir = match &start {
        Start::Play(_, dir) | Start::Build(_, dir) => dir.clone(),
    };
    // the project is called Haiku: settings, saves and caches moved from
    // userdata/zakuro and cache/zakuro, once
    for (base, old, new) in [("userdata", "zakuro", "haiku"), ("cache", "zakuro", "haiku")] {
        let (old, new) = (root.join(base).join(old), root.join(base).join(new));
        if old.is_dir() && !new.exists() {
            if let Err(error) = std::fs::rename(&old, &new) {
                eprintln!("could not move {} to {}: {error}", old.display(), new.display());
            }
        }
    }
    std::env::set_var("APPDATA", root.join("userdata"));
    // versions: what belongs to one version of the game (its data, Haiku's
    // files made from it, its mods and their list, its caches) is in its
    // folder, games\<version>\ (haiku_setup::layout); the game finds them
    // through FATES_ROOT, the shared settings and saves through APPDATA
    std::env::set_var("LOCALAPPDATA", game_dir.join("cache"));
    if game_dir != root {
        std::env::set_var("HAIKU_HOME", &root);
        std::env::set_var("FATES_ROOT", &game_dir);
        std::env::set_var("HAIKU_MODS_LIST", game_dir.join(haiku_setup::layout::MODS_LIST));
        let installed = haiku_setup::layout::installed(&root);
        std::env::set_var("HAIKU_VERSIONS", installed.iter().map(|v| v.tag()).collect::<Vec<_>>().join(","));
        if let Some(active) = haiku_setup::layout::active(&root) {
            std::env::set_var("HAIKU_VERSION", active.tag());
            eprintln!("versions: playing {} from {} (installed: {})", active.tag(), game_dir.display(), installed.iter().map(|v| v.tag()).collect::<Vec<_>>().join(", "));
        }
    }
    // os: the game is read from data/ as plain files; the ROM image is only
    // a fallback for a setup that hasn't been installed yet
    let data = game_dir.join("data");
    let game = if zakuro_fs::datadir::is_game_dir(&data) {
        data
    } else {
        eprintln!("no game in {}, run `Haiku --install <your dump>`; trying game/Fates-USA.cxi", data.display());
        root.join("game/Fates-USA.cxi")
    };
    std::env::set_var("FATES_DEFAULT_ROM", game);
    std::env::set_var("FATES_ORIGINAL_UI", "1");
    // Haiku's icon on the window and the taskbar (the exe's own icon is made
    // from the same picture by build.rs; builder: assets/icon/haiku.png)
    let _ = zakuro::WINDOW_ICON_PNG.set(include_bytes!("../../assets/icon/haiku.png"));
    // gfx-hfr: the native renderer is the default (pc.native_graphics, the
    // options' Renderer: Native / Compatibility); FATES_NATIVE_GFX and
    // FATES_GFX_NATIVE3D still override it, =0 the emulated GPU
    // native-final: the pure build has no emulated GPU, so the native
    // renderer is the only one, whatever the environment says
    if !cfg!(feature = "emulation") {
        std::env::set_var("FATES_NATIVE_GFX", "1");
        std::env::set_var("FATES_GFX_NATIVE3D", "1");
    }
    if std::env::var_os("FATES_NATIVE_GFX").is_none() {
        let native = zakuro::native_graphics_setting();
        std::env::set_var("FATES_NATIVE_GFX", if native { "1" } else { "0" });
        if native && std::env::var_os("FATES_GFX_NATIVE3D").is_none() {
            std::env::set_var("FATES_GFX_NATIVE3D", "1");
        }
    }
    match start {
        Start::Play(bound, _) => {
            let code = wire(bound);
            zakuro::run(Some(code));
        }
        // updscreen: the window opens as for the game, shows the game's code
        // being built in the game's look, and the game starts in it after
        // (wire, once the module is built and bound). The region's art and
        // words on the screen from the game installed
        Start::Build(work, _) => {
            if let Some(title) = work.title() {
                zakuro_core::region::set(title);
            }
            zakuro::run_after(Box::new(work));
        }
    }
}

/// what binding the game's module sets before its code runs: the tables and
/// hooks the frontend and the overrides reach each other through. updscreen:
/// a start that builds the module first does this once it is built, on the
/// window's thread, before the game starts.
fn wire(bound: module::Bound) -> zakuro::Linked {
    // os: the overrides that replace SDK calls into the OS reach the
    // native platform layer through this hook
    // SAFETY: set once, before any recompiled code runs
    unsafe {
        if let Some(hook) = zakuro_core::pcos::bridge::hook(os_native_abi) {
            os_native_hook = hook as *const c_void;
        }
    }
    // eur: USA or EUR code; the Rust side maps its USA addresses by it.
    // oneexe: the module's title (its recomp_hosted_title)
    let code_title = bound.title;
    zakuro_core::region::set(code_title);
    // the tables 3dsrecomp wrote in the module, which stays loaded
    // (so that functions a mod or an update changed run in the interpreter)
    let code = bound.linked;
    // hooks: a loaded state or an ended thread drops the overrides' tails
    // waiting in it; FATES_HOOKS_STATS=1 counts how each tail ran, printed
    // at exit. SAFETY: set before the game's code runs
    zakuro_core::recompiled::set_tails_forget(bound.tails_forget);
    // pulsefix: what of the overrides goes with Divine Pulse's moments and
    // saved states, besides the console's memory. Set before the game's
    // code runs, so the variables' first values are what they start from
    zakuro_core::state::keep::set_hooks(zakuro_core::state::keep::Hooks { regions: haiku_keep_regions, tails_keep: bound.tails_keep, tails_put: bound.tails_put });
    zakuro::haiku_codemods::set_override_names(OVERRIDE_NAMES); // codemods
    // codemods: the code's own registry (recomp.h), in the module
    zakuro_core::codemods::set_imports(bound.imports);
    if std::env::var("FATES_HOOKS_STATS").is_ok_and(|value| value == "1") {
        // SAFETY: the registry's own flag, before the code runs
        unsafe { *bound.imports.tails_counting = 1 };
    }
    // mods: the Main Menu's Mods entry (FATES_MODS_TITLE=0 leaves it out),
    // and no mod's code changes: the code is recompiled ahead of time, so
    // code a mod changed could not run; the Mods screen says so instead
    if std::env::var("FATES_MODS_TITLE").map_or(true, |value| value != "0") {
        // SAFETY: set once, before any recompiled code runs
        unsafe { haiku_mods_hook = Some(zakuro::haiku_mods::title_hook) };
    }
    zakuro_core::mods::allow_code_patches(false);
    // randomizer: the New Game randomizer's hooks (FATES_RANDOMIZER=0 leaves
    // them out). SAFETY: set once, before any recompiled code runs
    if std::env::var("FATES_RANDOMIZER").map_or(true, |value| value != "0") {
        unsafe {
            zakuro_core::randomizer::set_active_word(std::ptr::addr_of_mut!(haiku_rando_active));
            haiku_rando_hook = Some(zakuro_core::randomizer::hook);
        }
    }
    // supports: Corrin S-supports anyone and Kana's gender (FATES_SUPPORTS=0
    // leaves the hooks out). SAFETY: set once, before any recompiled code runs
    if std::env::var("FATES_SUPPORTS").map_or(true, |value| value != "0") {
        unsafe {
            zakuro_core::supports::set_text_word(std::ptr::addr_of_mut!(haiku_supports_text));
            haiku_supports_hook = Some(zakuro_core::supports::hook);
        }
    }
    // tones: Corrin's speaking style; acts only while a translation with the
    // Japanese game's talk tones is on (FATES_TONES=0 leaves the hooks out).
    // SAFETY: set once, before any recompiled code runs
    if std::env::var("FATES_TONES").map_or(true, |value| value != "0") {
        unsafe {
            zakuro_core::tones::set_active_word(std::ptr::addr_of_mut!(haiku_tones_active));
            haiku_tones_hook = Some(zakuro_core::tones::hook);
        }
    }
    // jpn2: Haiku's entries in the game's menus follow the script's language
    // on the Japanese game. SAFETY: set once, before any recompiled code runs
    unsafe { zakuro::jpn_english::set_word(std::ptr::addr_of_mut!(haiku_jpn_english)) };
    zakuro::hfr::set_game_timing(zakuro::hfr::GameTiming { set_max_step: hfr_set_max_step, scene_step: hfr_scene_step });
    zakuro::qol::set_hooks(zakuro::qol::Hooks { word: qol_word }); // qol
    zakuro::info_ui::set_hooks(zakuro::info_ui::Hooks { words: info_ui_words_ptr }); // info-ui
    zakuro::cheats::set_hooks(zakuro::cheats::Hooks { words: cheats_words_ptr }); // cheats
    zakuro::pulse::set_hooks(zakuro::pulse::Hooks { words: pulse_words_ptr }); // pulse
    zakuro::states::set_haiku_version(env!("CARGO_PKG_VERSION")); // pulse: save states' header
    // cpu: the game's code runs only as recompiled native code. reaching
    // anything that would need Zakuro's ARM interpreter stops with the
    // address and a guest callstack; FATES_ALLOW_INTERPRETER=1 lets the
    // interpreter run it instead, for diagnostics only
    let allow = std::env::var("FATES_ALLOW_INTERPRETER").is_ok_and(|value| value == "1");
    zakuro::native_cpu::enable(!allow);
    // wide: the game's cameras follow the shape the GPU draws the top
    // screen at. SAFETY: set before the game's code runs, read only by it
    unsafe { fates_wide_aspect_hook = Some(zakuro::wide_aspect_hook) };
    // cameras: the free camera's and the castle walk's input, from the
    // frontend. SAFETY: set before the game's code runs
    unsafe { fates_cameras_hook = Some(zakuro::cameras::game_hook) };
    // gfx: the native renderer, when FATES_NATIVE_GFX asks for it
    // SAFETY: nothing runs game code yet, so nothing reads the table
    fates_gfx::install(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks) });
    fates_gfx::install_3d(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks_3d) });
    // gfx-shadow
    fates_gfx::shadow::install(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks_shadow) });
    fates_gfx::install_post(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks_post) }); // gfx-post
    // gfx-eft: the effect hook
    fates_gfx::install_eft(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks_eft) });
    // gfx-terrain: 2D inside scene passes (map sprites, gauges, cursor)
    fates_gfx::scene2d::install(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks_scene2d) });
    // gfx-gaps: map unit sprites and HP gauges
    fates_gfx::install_unit_icon(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks_unit_icon) });
    // wide2: wallpapers drawn at their own size, their edges extended
    fates_gfx::install_wide2(unsafe { &mut *std::ptr::addr_of_mut!(fates_wide2_rect_hook) });
    unsafe { fates_units_snap_hook = Some(fates_gfx::units_snap) }; // gfx-2d
    // gfx-final: unit-icon textures rendered natively
    fates_gfx::install_capture(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks_capture) });
    // jpn: Live2D models drawn natively
    fates_gfx::install_live2d(unsafe { &mut *std::ptr::addr_of_mut!(fates_gfx_hooks_live2d) });
    // drawdist: far grass in batches, the batch overrides/drawdist.c fills
    fates_gfx::drawdist::set_batch(unsafe { std::ptr::addr_of_mut!(drawdist_batch) });
    code
}

/// oneexe: how the game starts. updscreen: Build, the game's module to build
/// first (an update, a version switched to), in the game's window
enum Start {
    Play(module::Bound, std::path::PathBuf),
    Build(updscreen::Work, std::path::PathBuf),
}

/// oneexe: what has to happen before the game can start, then the game's
/// module loaded and bound. A release install (setup/ beside Haiku.exe)
/// that has nothing installed yet gets the setup's window, which installs
/// and starts Haiku.exe again; this process ends there. updscreen: one whose
/// module or Haiku's files are out of date for this Haiku.exe gets
/// Start::Build, built in the game's own window before the game starts. A
/// development or test folder (no setup/, or HAIKU_GAME_MODULE given) and a
/// headless run never build anything: they say why and stop (exit 3).
fn setup_and_bind(root: &std::path::Path, args: &[String]) -> Start {
    use haiku_setup::pipeline::{self, Paths, Ready};
    let headless = args.iter().any(|arg| arg == "--headless");
    let forced = std::env::var_os("HAIKU_GAME_MODULE").is_some();
    let just_built = std::env::var_os("HAIKU_SETUP_DONE").is_some();
    std::env::remove_var("HAIKU_SETUP_DONE");
    if Paths::new(root.to_path_buf()).managed() && !forced {
        // versions: tidy also moves a flat install into games\<version>\
        for line in haiku_setup::upgrade::tidy(root) {
            eprintln!("oneexe: {line}");
        }
    }
    // versions: the version that plays (the setting, else the first
    // installed), or the folder itself when it is laid out flat
    let paths = Paths::new(root.to_path_buf());
    let game = paths.game.clone();
    // a development or test folder (no setup/) plays from data/ or game/ as
    // it is, with the module it was given: only a release install is set up
    let ready = if forced || !paths.managed() { Ready::Yes } else { pipeline::ready(&paths) };
    let needed = match ready {
        Ready::Yes => None,
        Ready::Install => Some(haiku_setup::ui::Start::Install),
        Ready::Update(why) => Some(haiku_setup::ui::Start::Update(why)),
    };
    let stop = |why: &str| -> ! {
        eprintln!("oneexe: the game can't start: {why}");
        if !headless {
            message(&format!("Haiku can't start the game: {why}.\n\nsetup-work\\setup.log, beside Haiku.exe, has the details."));
        }
        std::process::exit(3);
    };
    if let Some(start) = needed {
        if headless || !paths.managed() {
            stop(&match start {
                haiku_setup::ui::Start::Install => "no game is installed (run Haiku.exe without --headless, or `Haiku.exe --build <dump>`)".to_owned(),
                haiku_setup::ui::Start::Update(why) => format!("{why} (`Haiku.exe --rebuild` builds it; a development folder: dev/recomp.sh, or HAIKU_GAME_MODULE)"),
                haiku_setup::ui::Start::Setup => String::new(),
            });
        }
        if just_built {
            stop(&format!("the setup just ran, and still finds something to make ({start:?})"));
        }
        // updscreen: an installed game whose module or Haiku's files are out
        // of date: built in the game's window, in the game's look, and the
        // game starts there after (HAIKU_UPDATE_SETUP=1: the setup's own
        // window, as before). Nothing installed: the setup's first screens
        if let haiku_setup::ui::Start::Update(why) = &start {
            if std::env::var_os("HAIKU_UPDATE_SETUP").is_none() {
                eprintln!("updscreen: {why}: building the game's code in the game's window");
                return Start::Build(updscreen::Work::new(root, &paths, why), game);
            }
        }
        haiku_setup::ui::run(haiku_setup::ui::Options::from_env(), start);
        std::process::exit(0);
    }
    let path = module::find(&game);
    let title = haiku_setup::pipeline::installed_title(&game.join("data"));
    match module::load(&path, if forced { None } else { title }) {
        Ok(bound) => Start::Play(bound, game),
        Err(why) => {
            // a module the checks passed that still can't be bound (damaged,
            // half written): built again, once (updscreen: in the game's
            // window)
            if !headless && paths.managed() && !forced && !just_built {
                eprintln!("oneexe: {why}: building the game's code again");
                let _ = std::fs::remove_file(module::dir(&game).join(haiku_setup::module::NOTE));
                if std::env::var_os("HAIKU_UPDATE_SETUP").is_none() {
                    return Start::Build(updscreen::Work::new(root, &paths, &why), game);
                }
                haiku_setup::ui::run(haiku_setup::ui::Options::from_env(), haiku_setup::ui::Start::Update(why));
                std::process::exit(0);
            }
            stop(&why)
        }
    }
}

/// oneexe: a message box, for a windowed start that can't go on
fn message(text: &str) {
    #[cfg(windows)]
    {
        #[link(name = "user32")]
        extern "system" {
            fn MessageBoxW(hwnd: *mut c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
        }
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (text, caption) = (wide(text), wide("Haiku"));
        // SAFETY: NUL-terminated strings; no owner window
        unsafe { MessageBoxW(std::ptr::null_mut(), text.as_ptr(), caption.as_ptr(), 0x10) };
    }
    #[cfg(not(windows))]
    let _ = text;
}
