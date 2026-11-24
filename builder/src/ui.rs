//! Haiku's setup window: a small Win32 window drawn by hand with GDI in
//! the port's colours (the dark teal wallpaper, cream text, gold trim).
//! There is no game art before the game is installed, so it keeps Haiku
//! Setup's look.
//!
//! Screens: the game dump, the DLC, the build (progress and percentage),
//! a failure (what went wrong, the log), and once Haiku is built, Play,
//! Rebuild and Change dump or DLC. Files can be picked or dropped on it.
//! versions2: and Add a version (the same screens, for a dump of another
//! version, which installs beside) and Remove a version (its own screen:
//! which, what is deleted and what is kept, then Windows' Yes/No).
//!
//! oneexe: it is Haiku.exe's own window now. Haiku.exe opens it on first
//! launch (Start::Install), when the game's module or Haiku's files have to
//! be made again (Start::Update: the build starts by itself and the game
//! starts when it is done), and for `Haiku.exe --setup` and the Options'
//! Setup entry (Start::Setup). Play starts Haiku.exe again, which then
//! plays: the game always runs in a process of its own.
//!
//! For tests and evidence it can drive itself and save what it shows:
//! `--shots <dir>` saves each frame it is asked to (its own back buffer,
//! the window's exact pixels), and `--script "<cmd>; <cmd>; ..."` runs
//! commands: game <path>, dlc <path>, nodlc, click <next|back|build|cancel|
//! retry|play|rebuild|change|nodlc|add-version|remove-version|pick-usa|
//! pick-eur|pick-jpn|remove-confirm>, shot <name>, wait-percent <n>,
//! wait-done, wait-ms <n>, exit. (A scripted remove-confirm skips the
//! Yes/No.)

/// oneexe: why the window opens
#[derive(Clone, Debug, PartialEq)]
pub enum Start {
    /// no game installed yet: the first screens
    Install,
    /// the game is installed, but Haiku's files or its module have to be
    /// made again: built at once, then the game starts
    Update(String),
    /// asked for (--setup, the Options' Setup): Rebuild, Change dump or
    /// DLC, the voices
    Setup,
}

#[cfg(not(windows))]
pub struct Options;
#[cfg(not(windows))]
impl Options {
    pub fn from_env() -> Options {
        Options
    }
    pub fn from_args(_: &[String]) -> Options {
        Options
    }
}
#[cfg(not(windows))]
pub fn run(_: Options, _: Start) {
    eprintln!("Haiku's setup window is for Windows; use --build");
}

#[cfg(windows)]
pub use imp::*;

#[cfg(windows)]
mod imp {
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Instant;

    use super::Start;
    use crate::pipeline::{self, Paths, Shared, State, Status};
    use crate::win::*;
    use crate::{dlc, dump, log};

    // jpvoice: the Japanese voices screen and the Play screen's Voices
    #[path = "../../voices_ui.rs"]
    mod voices_ui;

    pub struct Options {
        pub shots: Option<PathBuf>,
        pub script: Vec<String>,
        /// the window is never shown (tests, evidence): a shot is drawn
        /// off screen by the same code
        pub hidden: bool,
    }

    fn split_script(text: &str) -> Vec<String> {
        text.split(';').map(|s| s.trim().to_owned()).filter(|s| !s.is_empty()).collect()
    }

    impl Options {
        pub fn from_env() -> Options {
            Options {
                shots: std::env::var_os("HAIKU_SETUP_SHOTS").map(PathBuf::from),
                script: std::env::var("HAIKU_SETUP_SCRIPT").map(|s| split_script(&s)).unwrap_or_default(),
                hidden: std::env::var("HAIKU_SETUP_HIDDEN").is_ok_and(|v| v == "1"),
            }
        }
        pub fn from_args(args: &[String]) -> Options {
            let mut o = Options::from_env();
            let mut i = 0;
            while i < args.len() {
                match args[i].as_str() {
                    "--shots" => {
                        o.shots = args.get(i + 1).map(PathBuf::from);
                        i += 1;
                    }
                    "--hidden" => o.hidden = true,
                    "--script" => {
                        o.script = args.get(i + 1).map(|s| split_script(s)).unwrap_or_default();
                        i += 1;
                    }
                    _ => {}
                }
                i += 1;
            }
            o
        }
    }

    // ---- look ---------------------------------------------------------------

    const BG_TOP: (u8, u8, u8) = (17, 46, 50);
    const BG_BOTTOM: (u8, u8, u8) = (8, 25, 29);
    const PANEL: u32 = rgb(24, 58, 62);
    const GOLD: u32 = rgb(255, 215, 130);
    const TRIM: u32 = rgb(176, 138, 74);
    const CREAM: u32 = rgb(253, 236, 185);
    const MUTED: u32 = rgb(178, 165, 138);
    const RED_BUTTON: u32 = rgb(150, 36, 24);
    const RED_HOVER: u32 = rgb(178, 48, 30);
    const SECONDARY: u32 = rgb(30, 70, 74);
    const SECONDARY_HOVER: u32 = rgb(40, 88, 92);
    const DISABLED: u32 = rgb(42, 56, 58);
    const OK: u32 = rgb(150, 214, 120);
    const BAD: u32 = rgb(240, 128, 104);

    const WIDTH: i32 = 760;
    const HEIGHT: i32 = 520;

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Screen {
        Game,
        Dlc,
        Build,
        Failed,
        Ready,
        /// jpvoice: the optional Japanese voices
        Voices,
        /// versions2: Remove a version
        Remove,
    }

    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Id {
        PickFile,
        PickFolder,
        Next,
        Back,
        NoDlc,
        Build,
        Cancel,
        Retry,
        OpenLog,
        Play,
        Rebuild,
        Change,
        // jpvoice
        VoicesOpen,
        VoicesEnglish,
        VoicesJapanese,
        VoicesExtract,
        VoicesRemove,
        // jpdlc: the Japanese DLC's pick
        VoicesDlcFile,
        VoicesDlcFolder,
        // versions2
        AddVersion,
        RemoveVersion,
        RemovePick(crate::layout::Version),
        RemoveConfirm,
    }

    struct Button {
        id: Id,
        label: String,
        rect: RECT,
        primary: bool,
        enabled: bool,
    }

    struct Fonts {
        title: HGDIOBJ,
        heading: HGDIOBJ,
        body: HGDIOBJ,
        bold: HGDIOBJ,
        small: HGDIOBJ,
        button: HGDIOBJ,
        big: HGDIOBJ,
    }

    impl Fonts {
        fn new(scale: f32) -> Fonts {
            let f = |size: f32, weight: i32, face: &str| {
                let face = wide(face);
                // SAFETY: plain GDI font creation
                unsafe { CreateFontW(-(size * scale).round() as i32, 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, CLEARTYPE_QUALITY, 0, face.as_ptr()) }
            };
            Fonts {
                title: f(38.0, FW_BOLD, "Georgia"),
                heading: f(21.0, FW_BOLD, "Georgia"),
                body: f(15.0, FW_NORMAL, "Segoe UI"),
                bold: f(15.0, FW_SEMIBOLD, "Segoe UI"),
                small: f(12.5, FW_NORMAL, "Segoe UI"),
                button: f(15.0, FW_SEMIBOLD, "Segoe UI"),
                big: f(44.0, FW_BOLD, "Georgia"),
            }
        }
        fn delete(&self) {
            for font in [self.title, self.heading, self.body, self.bold, self.small, self.button, self.big] {
                // SAFETY: fonts made by new, no longer selected
                unsafe { DeleteObject(font) };
            }
        }
    }

    enum Wait {
        Shot,
        Percent(f32),
        Done,
        Until(Instant),
    }

    struct App {
        hwnd: HWND,
        scale: f32,
        fonts: Fonts,
        paths: Paths,
        state: State,
        screen: Screen,
        game: Option<Result<dump::Game, dump::Problem>>,
        game_name: String,
        dlcs: Vec<dlc::Dlc>,
        dlc_result: Option<Result<String, dump::Problem>>,
        dlc_name: String,
        no_dlc: bool,
        resumable: bool,
        status: Shared,
        cancel: Arc<AtomicBool>,
        job: Option<std::thread::JoinHandle<Result<String, String>>>,
        failure: String,
        built_note: String,
        buttons: Vec<Button>,
        hover: Option<Id>,
        pressed: Option<Id>,
        shots: Option<PathBuf>,
        hidden: bool,
        shot_pending: Option<String>,
        shot_count: usize,
        script: VecDeque<String>,
        wait: Option<Wait>,
        /// jpvoice
        voices: voices_ui::VoicesState,
        /// oneexe: building again by itself (Start::Update): the game
        /// starts when it is done
        updating: Option<String>,
        /// versions2: Add a version's screens (a dump of another version)
        adding: bool,
        /// versions2: a game is built, so the dump screen can go Back to
        /// Play
        can_return: bool,
        /// versions2: Remove a version: the one picked, what removing it
        /// deletes, why it wasn't
        remove_pick: Option<crate::layout::Version>,
        remove_summary: Option<crate::layout::RemoveSummary>,
        remove_error: Option<String>,
        /// versions2: a removed version's folder being deleted
        deleting: Vec<std::thread::JoinHandle<()>>,
        /// versions2: driven by --script (no Yes/No)
        scripted: bool,
    }

    thread_local! {
        static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    }

    fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
        APP.with(|app| app.try_borrow_mut().ok().and_then(|mut a| a.as_mut().map(f)))
    }

    fn file_name(path: &Path) -> String {
        path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string())
    }

    impl App {
        fn px(&self, v: i32) -> i32 {
            (v as f32 * self.scale).round() as i32
        }

        fn set_game(&mut self, path: &Path) {
            log::line(format!("game picked: {}", path.display()));
            self.game_name = file_name(path);
            let result = dump::check_game(path);
            match &result {
                Ok(g) => log::line(format!("  ok: {} ({} bytes)", g.kind, g.size)),
                Err(p) => log::line(format!("  {}: {}", p.kind(), p.message())),
            }
            self.game = Some(result);
            self.screen = Screen::Game;
            self.invalidate();
        }

        fn set_dlc(&mut self, path: &Path) {
            log::line(format!("DLC picked: {}", path.display()));
            self.dlc_name = file_name(path);
            // eurdlc: a DLC must fit the game picked before it (the European
            // DLC isn't the USA game's), said here rather than mid-build
            let region = self.game_ok().map(|g| g.region);
            let fit = |d: dlc::Dlc| match region.map(|r| crate::jpn::dlc_fits(r, d.title_id)) {
                Some(Err(why)) => Err(dump::Problem::WrongGame(why)),
                _ => Ok(d),
            };
            match dlc::check(path).and_then(fit) {
                Ok(d) => {
                    log::line(format!("  ok: {}", d.describe()));
                    self.dlc_result = Some(Ok(d.describe()));
                    self.dlcs = vec![d];
                    self.no_dlc = false;
                }
                Err(p) => {
                    log::line(format!("  {}: {}", p.kind(), p.message()));
                    self.dlc_result = Some(Err(p));
                    self.dlcs.clear();
                }
            }
            self.screen = Screen::Dlc;
            self.invalidate();
        }

        /// versions2: the dump and DLC the playing version was built from,
        /// as the setup's state has them (what the first screens show)
        fn restore_choice(&mut self) {
            self.game = None;
            self.game_name.clear();
            self.dlcs.clear();
            self.dlc_result = None;
            self.dlc_name.clear();
            self.no_dlc = false;
            let state = self.state.clone();
            if let Some(dump_path) = state.get("dump").map(PathBuf::from).filter(|p| p.exists()) {
                self.set_game(&dump_path);
                for dlc_path in state.dlc_paths().into_iter().filter(|p| p.exists()) {
                    self.set_dlc(&dlc_path);
                }
                if state.get("dlc") == Some("") {
                    self.no_dlc = true;
                }
            }
        }

        /// versions2: Add a version: the dump screens, for another version
        fn add_version(&mut self) {
            self.adding = true;
            self.game = None;
            self.game_name.clear();
            self.dlcs.clear();
            self.dlc_result = None;
            self.dlc_name.clear();
            self.no_dlc = false;
            self.resumable = false;
            self.screen = Screen::Game;
        }

        /// versions2: back from the dump screens to Play
        fn back_to_ready(&mut self) {
            self.adding = false;
            self.remove_pick = None;
            self.remove_summary = None;
            self.remove_error = None;
            self.restore_choice();
            self.screen = Screen::Ready;
        }

        fn pick_removal(&mut self, version: crate::layout::Version) {
            self.remove_pick = Some(version);
            self.remove_error = None;
            self.remove_summary = Some(crate::layout::remove_summary(&self.paths.root, version));
        }

        /// versions2: Remove a version, after Windows' Yes/No
        fn remove_version(&mut self) {
            let Some(version) = self.remove_pick else { return };
            let root = self.paths.root.clone();
            // a test can still have the question shown (HAIKU_SETUP_ASK=1)
            if !self.scripted || std::env::var("HAIKU_SETUP_ASK").is_ok_and(|v| v == "1") {
                let text = wide(&format!(
                    "Remove {}?\n\nIts folder, games\\{}\\, is deleted: its game files and the game's code, Haiku's files made from it, its caches and its mods. This can't be undone; adding it again needs its dump and a build.\n\nYour saves and settings (userdata\\haiku) are kept.",
                    version.name(),
                    version.folder()
                ));
                let caption = wide("Haiku Setup");
                // SAFETY: NUL-terminated strings, our window
                let answer = unsafe { MessageBoxW(self.hwnd, text.as_ptr(), caption.as_ptr(), MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2) };
                if answer != IDYES {
                    log::line(format!("remove {}: not confirmed", version.tag()));
                    return;
                }
            }
            log::line(format!("remove {}: confirmed", version.tag()));
            match crate::layout::remove(&root, version) {
                Ok(folder) => {
                    log::line(format!("  {} moved to {}, deleting it; playing {}", crate::layout::dir(&root, version).display(), folder.display(), crate::layout::active(&root).map(|v| v.tag()).unwrap_or("none")));
                    self.deleting.push(std::thread::spawn(move || match crate::layout::delete_removed(&folder) {
                        Ok(()) => log::line(format!("  deleted {}", folder.display())),
                        Err(e) => log::line(format!("  {} isn't all deleted ({e}); the rest goes at the next start", folder.display())),
                    }));
                    self.paths = Paths::new(root.clone());
                    self.state = State::load(&self.paths);
                    self.built_note = format!("{} was removed; your saves are kept.", crate::layout::capital(version.name()));
                    self.back_to_ready();
                }
                Err(e) => {
                    log::line(format!("  not removed: {e}"));
                    self.remove_error = Some(e);
                }
            }
        }

        fn game_ok(&self) -> Option<&dump::Game> {
            self.game.as_ref().and_then(|g| g.as_ref().ok())
        }

        fn start_build(&mut self) {
            let Some(game) = self.game_ok().cloned() else {
                self.screen = Screen::Game;
                self.invalidate();
                return;
            };
            if self.dlcs.is_empty() && !self.no_dlc {
                self.screen = Screen::Dlc;
                self.invalidate();
                return;
            }
            // eurdlc: the game may have been changed after the DLC was picked
            if let Some(why) = self.dlcs.iter().find_map(|d| crate::jpn::dlc_fits(game.region, d.title_id).err()) {
                log::line(format!("  the DLC doesn't fit this game: {why}"));
                self.dlc_result = Some(Err(dump::Problem::WrongGame(why)));
                self.dlcs.clear();
                self.screen = Screen::Dlc;
                self.invalidate();
                return;
            }
            *self.status.lock().unwrap() = Status { step: "Starting".into(), ..Status::default() };
            self.cancel.store(false, Ordering::Relaxed);
            let (paths, dlcs, status, cancel) = (self.paths.clone(), self.dlcs.clone(), self.status.clone(), self.cancel.clone());
            self.job = Some(std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pipeline::run(&paths, &game, &dlcs, &status, &cancel)));
                let result = result.unwrap_or_else(|_| Err("Haiku's setup ran into an internal error. The log has the details.".into()));
                match &result {
                    Ok(said) => log::line(format!("build finished: {said}")),
                    Err(e) => log::line(format!("build failed: {e}")),
                }
                result
            }));
            self.screen = Screen::Build;
            self.invalidate();
        }

        /// oneexe: Haiku's files and the game's module made again from the
        /// game installed in data/ (no dump needed)
        fn start_update(&mut self) {
            *self.status.lock().unwrap() = Status { step: "Starting".into(), ..Status::default() };
            self.cancel.store(false, Ordering::Relaxed);
            let (paths, status, cancel) = (self.paths.clone(), self.status.clone(), self.cancel.clone());
            self.job = Some(std::thread::spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| pipeline::update(&paths, &status, &cancel)));
                let result = result.unwrap_or_else(|_| Err("Haiku's setup ran into an internal error. The log has the details.".into()));
                match &result {
                    Ok(said) => log::line(format!("update finished: {said}")),
                    Err(e) => log::line(format!("update failed: {e}")),
                }
                result
            }));
            self.screen = Screen::Build;
            self.invalidate();
        }

        fn poll(&mut self) {
            let finished = self.job.as_ref().is_some_and(|j| j.is_finished());
            if self.screen == Screen::Build {
                self.invalidate();
            }
            if finished {
                let job = self.job.take().unwrap();
                match job.join().unwrap_or_else(|_| Err("Haiku's setup ran into an internal error.".into())) {
                    Ok(said) if self.updating.is_some() => {
                        // oneexe: built again by itself: the game starts
                        self.built_note = said;
                        self.updating = None;
                        self.screen = Screen::Ready;
                        self.play();
                    }
                    Ok(said) => {
                        self.built_note = said;
                        // versions: the version just set up plays now
                        self.paths = Paths::new(self.paths.root.clone());
                        self.state = State::load(&self.paths);
                        self.resumable = false;
                        self.adding = false; // versions2
                        self.can_return = true;
                        self.screen = Screen::Ready;
                    }
                    Err(e) => {
                        self.failure = e;
                        self.resumable = true;
                        self.screen = Screen::Failed;
                    }
                }
                self.invalidate();
            }
        }

        fn invalidate(&self) {
            // SAFETY: our window
            unsafe { InvalidateRect(self.hwnd, std::ptr::null(), 0) };
        }

        fn click(&mut self, id: Id) {
            log::line(format!("click {id:?} on {:?}", self.screen));
            match (self.screen, id) {
                // jpvoice
                (Screen::Voices, id) => self.voices_click(id),
                (Screen::Ready, Id::VoicesOpen) => self.screen = Screen::Voices,
                (Screen::Ready, Id::VoicesEnglish) => self.voices_choose(false),
                (Screen::Ready, Id::VoicesJapanese) => self.voices_choose(true),
                // versions2: Add a version, Remove a version
                (Screen::Ready, Id::AddVersion) => self.add_version(),
                (Screen::Game, Id::Back) => self.back_to_ready(),
                (Screen::Ready, Id::RemoveVersion) => {
                    self.remove_pick = None;
                    self.remove_summary = None;
                    self.remove_error = None;
                    self.screen = Screen::Remove;
                }
                (Screen::Remove, Id::RemovePick(version)) => self.pick_removal(version),
                (Screen::Remove, Id::RemoveConfirm) => self.remove_version(),
                (Screen::Remove, Id::Back) => self.back_to_ready(),
                (Screen::Game, Id::PickFile) => {
                    let filter = [("3DS game dumps (*.3ds, *.cci, *.cia, *.cxi)", "*.3ds;*.cci;*.cia;*.cxi;*.app"), ("All files", "*.*")];
                    if let Some(p) = pick_file(self.hwnd, "Select your Fire Emblem Fates Special Edition dump", &filter) {
                        self.set_game(&p);
                    }
                }
                (Screen::Game, Id::PickFolder) => {
                    if let Some(p) = pick_folder(self.hwnd, "Select the folder with your Fire Emblem Fates dump (or Citra's or Azahar's sdmc folder)") {
                        self.set_game(&p);
                    }
                }
                (Screen::Game, Id::Next) => {
                    if self.game_ok().is_some() {
                        self.screen = Screen::Dlc;
                    }
                }
                (Screen::Dlc, Id::PickFile) => {
                    if let Some(p) = pick_file(self.hwnd, "Select your DLC", &[("DLC (*.cia)", "*.cia"), ("All files", "*.*")]) {
                        self.set_dlc(&p);
                    }
                }
                (Screen::Dlc, Id::PickFolder) => {
                    if let Some(p) = pick_folder(self.hwnd, "Select the folder with your DLC (Citra's or Azahar's sdmc folder, or an earlier Haiku's data folder)") {
                        self.set_dlc(&p);
                    }
                }
                (Screen::Dlc, Id::NoDlc) => {
                    self.no_dlc = true;
                    self.dlcs.clear();
                    self.dlc_result = None;
                    self.dlc_name.clear();
                }
                (Screen::Dlc, Id::Back) => self.screen = Screen::Game,
                // oneexe: an update that failed is tried again as an update
                (Screen::Failed, Id::Retry) if self.updating.is_some() => self.start_update(),
                (Screen::Dlc, Id::Build) | (Screen::Failed, Id::Retry) => self.start_build(),
                (Screen::Failed, Id::Back) => self.screen = Screen::Dlc,
                (Screen::Failed, Id::OpenLog) => open_path(&self.paths.log_file()),
                (Screen::Build, Id::Cancel) => self.cancel.store(true, Ordering::Relaxed),
                (Screen::Ready, Id::Play) => self.play(),
                (Screen::Ready, Id::Change) => self.screen = Screen::Game,
                (Screen::Ready, Id::Rebuild) => {
                    // oneexe: Haiku's files and the whole module again, from
                    // the game installed in data/ (Change dump or DLC
                    // installs the game again)
                    self.state.values.remove("done.assets");
                    self.state.save(&self.paths);
                    let _ = std::fs::remove_file(self.paths.module_dir().join(crate::module::NOTE));
                    let _ = std::fs::remove_file(self.paths.work.join("module").join("c").join("objects.tsv"));
                    self.start_update();
                }
                _ => {}
            }
            self.invalidate();
        }

        fn play(&mut self) {
            // oneexe: Haiku.exe again, which plays now that everything is
            // made; it waits for this one to close (HAIKU_RESTART_WAIT)
            let exe = self.paths.exe();
            log::line(format!("play: {}", exe.display()));
            match crate::spawn::start_waiting(&exe, &self.paths.root) {
                Ok(_) => {
                    // SAFETY: our window
                    unsafe { DestroyWindow(self.hwnd) };
                }
                Err(e) => {
                    self.failure = format!("Haiku.exe can't be started ({e}).");
                    self.screen = Screen::Failed;
                }
            }
        }

        // ---- the script (tests, evidence) -----------------------------------

        fn step_script(&mut self) {
            if let Some(wait) = &self.wait {
                let done = match wait {
                    Wait::Shot => self.shot_pending.is_none(),
                    Wait::Percent(p) => self.status.lock().unwrap().percent >= *p || self.screen != Screen::Build,
                    Wait::Done => self.job.is_none() && self.screen != Screen::Build && self.voices.job.is_none(), // jpvoice: the voices' job too
                    Wait::Until(t) => Instant::now() >= *t,
                };
                if !done {
                    return;
                }
                self.wait = None;
            }
            let Some(cmd) = self.script.pop_front() else { return };
            log::line(format!("script: {cmd}"));
            let (word, arg) = cmd.split_once(' ').map_or((cmd.as_str(), ""), |(w, a)| (w, a.trim()));
            match word {
                "game" => self.set_game(Path::new(arg)),
                "dlc" => self.set_dlc(Path::new(arg)),
                "voices" => self.set_voices_dump(Path::new(arg)), // jpvoice
                "nodlc" => self.click(Id::NoDlc),
                "click" => {
                    let id = match arg {
                        "next" => Some(Id::Next),
                        "back" => Some(Id::Back),
                        "build" => Some(Id::Build),
                        "cancel" => Some(Id::Cancel),
                        "retry" => Some(Id::Retry),
                        "play" => Some(Id::Play),
                        "rebuild" => Some(Id::Rebuild),
                        "change" => Some(Id::Change),
                        "nodlc" => Some(Id::NoDlc),
                        // jpvoice
                        "voices" => Some(Id::VoicesOpen),
                        "extract" => Some(Id::VoicesExtract),
                        "english" => Some(Id::VoicesEnglish),
                        "japanese" => Some(Id::VoicesJapanese),
                        "remove" => Some(Id::VoicesRemove),
                        // versions2
                        "add-version" => Some(Id::AddVersion),
                        "remove-version" => Some(Id::RemoveVersion),
                        "pick-usa" => Some(Id::RemovePick(crate::layout::Version::Usa)),
                        "pick-eur" => Some(Id::RemovePick(crate::layout::Version::Eur)),
                        "pick-jpn" => Some(Id::RemovePick(crate::layout::Version::Jpn)),
                        "remove-confirm" => Some(Id::RemoveConfirm),
                        _ => None,
                    };
                    if let Some(id) = id {
                        self.click(id);
                    }
                }
                "shot" => {
                    self.shot_pending = Some(arg.to_owned());
                    self.wait = Some(Wait::Shot);
                    if self.hidden {
                        // no window on screen: drawn off screen
                        self.paint(0);
                    } else {
                        self.invalidate();
                    }
                }
                "wait-percent" => self.wait = Some(Wait::Percent(arg.parse().unwrap_or(100.0))),
                "wait-done" => self.wait = Some(Wait::Done),
                "wait-ms" => self.wait = Some(Wait::Until(Instant::now() + std::time::Duration::from_millis(arg.parse().unwrap_or(500)))),
                "exit" => {
                    // SAFETY: our window
                    unsafe { DestroyWindow(self.hwnd) };
                }
                _ => {}
            }
        }

        // ---- layout and drawing -------------------------------------------------

        fn layout(&mut self) {
            let mut buttons = Vec::new();
            let scale = self.scale;
            let s = |v: i32| (v as f32 * scale).round() as i32;
            let bottom = s(HEIGHT - 70);
            let h = s(42);
            let mut add = |id: Id, label: &str, x: i32, y: i32, w: i32, primary: bool, enabled: bool| {
                buttons.push(Button { id, label: label.to_owned(), rect: RECT { left: s(x), top: y, right: s(x + w), bottom: y + h }, primary, enabled });
            };
            match self.screen {
                Screen::Game => {
                    add(Id::PickFile, "Select file\u{2026}", 40, s(214), 170, false, true);
                    add(Id::PickFolder, "Select folder\u{2026}", 222, s(214), 170, false, true);
                    add(Id::Next, "Next  \u{203A}", WIDTH - 40 - 160, bottom, 160, true, self.game_ok().is_some());
                    if self.can_return {
                        add(Id::Back, "\u{2039}  Back", 40, bottom, 130, false, true); // versions2
                    }
                }
                Screen::Dlc => {
                    add(Id::PickFile, "Select file\u{2026}", 40, s(214), 170, false, true);
                    add(Id::PickFolder, "Select folder\u{2026}", 222, s(214), 170, false, true);
                    add(Id::NoDlc, "Continue without DLC", 404, s(214), 210, false, true);
                    add(Id::Back, "\u{2039}  Back", 40, bottom, 130, false, true);
                    add(Id::Build, if self.resumable { "Carry on building" } else { "Build Haiku" }, WIDTH - 40 - 190, bottom, 190, true, !self.dlcs.is_empty() || self.no_dlc);
                }
                Screen::Build => {
                    let stopping = self.cancel.load(Ordering::Relaxed);
                    add(Id::Cancel, if stopping { "Stopping\u{2026}" } else { "Cancel" }, WIDTH - 40 - 150, bottom, 150, false, !stopping);
                }
                Screen::Failed => {
                    add(Id::Back, "\u{2039}  Back", 40, bottom, 130, false, true);
                    add(Id::OpenLog, "Open the log", 182, bottom, 160, false, true);
                    add(Id::Retry, "Try again", WIDTH - 40 - 170, bottom, 170, true, true);
                }
                Screen::Ready => {
                    add(Id::Play, "Play", 40, s(262), 240, true, true);
                    add(Id::Rebuild, "Rebuild", 40, bottom, 120, false, true);
                    add(Id::Change, "Change dump or DLC", 172, bottom, 196, false, true);
                    // versions2
                    let versions = crate::layout::installed(&self.paths.root).len();
                    add(Id::AddVersion, "Add a version\u{2026}", 380, bottom, 160, false, true);
                    add(Id::RemoveVersion, "Remove a version\u{2026}", 552, bottom, 168, false, versions > 1);
                    self.voices_buttons(&mut add); // jpvoice
                }
                Screen::Voices => self.voices_buttons(&mut add), // jpvoice
                // versions2: which version, then Remove
                Screen::Remove => {
                    let installed = crate::layout::installed(&self.paths.root);
                    let playing = crate::layout::active(&self.paths.root);
                    for (i, v) in installed.iter().enumerate() {
                        let label = match v {
                            crate::layout::Version::Usa => "USA",
                            crate::layout::Version::Eur => "Europe",
                            crate::layout::Version::Jpn => "Japan",
                        };
                        let label = if Some(*v) == playing { format!("{label} (playing)") } else { label.to_owned() };
                        add(Id::RemovePick(*v), &label, 40 + 182 * i as i32, s(214), 170, self.remove_pick == Some(*v), true);
                    }
                    add(Id::Back, "\u{2039}  Back", 40, bottom, 130, false, true);
                    let label = match self.remove_pick {
                        Some(crate::layout::Version::Usa) => "Remove USA",
                        Some(crate::layout::Version::Eur) => "Remove Europe",
                        Some(crate::layout::Version::Jpn) => "Remove Japan",
                        None => "Remove",
                    };
                    add(Id::RemoveConfirm, label, WIDTH - 40 - 200, bottom, 200, false, self.remove_pick.is_some() && installed.len() > 1);
                }
            }
            for b in &mut buttons {
                if b.id == Id::Play {
                    b.rect.bottom = b.rect.top + s(56);
                }
            }
            self.buttons = buttons;
        }

        fn paint(&mut self, hdc_window: HDC) {
            self.layout();
            let mut rc = RECT::default();
            // SAFETY: GDI calls on a memory DC and DIB we own
            unsafe {
                GetClientRect(self.hwnd, &mut rc);
                let (w, h) = (rc.right.max(1), rc.bottom.max(1));
                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w,
                        biHeight: -h,
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: 0,
                        biSizeImage: 0,
                        biXPelsPerMeter: 0,
                        biYPelsPerMeter: 0,
                        biClrUsed: 0,
                        biClrImportant: 0,
                    },
                    bmiColors: [0],
                };
                let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
                let dc = CreateCompatibleDC(hdc_window);
                let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, 0, 0);
                let old = SelectObject(dc, bitmap);
                SetBkMode(dc, TRANSPARENT);
                self.draw(dc, w, h);
                GdiFlush();
                if hdc_window != 0 {
                    BitBlt(hdc_window, 0, 0, w, h, dc, 0, 0, SRCCOPY);
                }
                if let Some(name) = self.shot_pending.take() {
                    let pixels = std::slice::from_raw_parts(bits as *const u8, (w * h * 4) as usize);
                    self.save_shot(&name, w as u32, h as u32, pixels);
                }
                SelectObject(dc, old);
                DeleteObject(bitmap);
                DeleteDC(dc);
            }
        }

        fn save_shot(&mut self, name: &str, w: u32, h: u32, bgra: &[u8]) {
            let Some(dir) = &self.shots else { return };
            let _ = std::fs::create_dir_all(dir);
            self.shot_count += 1;
            let path = dir.join(format!("{:02}-{name}.png", self.shot_count));
            let rgba: Vec<u8> = bgra.chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], 255]).collect();
            let picture = crate::derive::Picture { w, h, rgba };
            match crate::derive::write_png(&path, &picture) {
                Ok(()) => log::line(format!("saved the window to {}", path.display())),
                Err(e) => log::line(format!("could not save {}: {e}", path.display())),
            }
        }

        unsafe fn text(&self, dc: HDC, font: HGDIOBJ, colour: u32, rect: RECT, text: &str, flags: u32) -> i32 {
            let mut r = rect;
            let w = wide(text);
            SelectObject(dc, font);
            SetTextColor(dc, colour);
            DrawTextW(dc, w.as_ptr(), -1, &mut r, flags | DT_NOPREFIX)
        }

        /// how tall text is in width.
        unsafe fn measure(&self, dc: HDC, font: HGDIOBJ, width: i32, text: &str) -> i32 {
            let mut r = RECT { left: 0, top: 0, right: width, bottom: 0 };
            let w = wide(text);
            SelectObject(dc, font);
            DrawTextW(dc, w.as_ptr(), -1, &mut r, DT_WORDBREAK | DT_CALCRECT | DT_NOPREFIX);
            r.bottom
        }

        unsafe fn fill(&self, dc: HDC, rect: RECT, colour: u32) {
            let brush = CreateSolidBrush(colour);
            FillRect(dc, &rect, brush);
            DeleteObject(brush);
        }

        unsafe fn round_rect(&self, dc: HDC, rect: RECT, fill: u32, border: u32, radius: i32, width: i32) {
            let brush = CreateSolidBrush(fill);
            let pen = CreatePen(PS_SOLID, width.max(1), border);
            let ob = SelectObject(dc, brush);
            let op = SelectObject(dc, pen);
            RoundRect(dc, rect.left, rect.top, rect.right, rect.bottom, radius, radius);
            SelectObject(dc, ob);
            SelectObject(dc, op);
            DeleteObject(brush);
            DeleteObject(pen);
        }

        unsafe fn line(&self, dc: HDC, x0: i32, y0: i32, x1: i32, y1: i32, colour: u32, width: i32) {
            let pen = CreatePen(PS_SOLID, width.max(1), colour);
            let op = SelectObject(dc, pen);
            MoveToEx(dc, x0, y0, std::ptr::null_mut());
            LineTo(dc, x1, y1);
            SelectObject(dc, op);
            DeleteObject(pen);
        }

        /// a round mark: a tick for good news, a cross for bad.
        unsafe fn mark(&self, dc: HDC, x: i32, y: i32, good: bool) {
            let d = self.px(26);
            let colour = if good { OK } else { BAD };
            let pen = CreatePen(PS_SOLID, self.px(2), colour);
            let op = SelectObject(dc, pen);
            let ob = SelectObject(dc, GetStockObject(NULL_BRUSH));
            Ellipse(dc, x, y, x + d, y + d);
            SelectObject(dc, op);
            SelectObject(dc, ob);
            DeleteObject(pen);
            let t = self.px(3);
            if good {
                self.line(dc, x + d * 27 / 100, y + d * 52 / 100, x + d * 44 / 100, y + d * 69 / 100, colour, t);
                self.line(dc, x + d * 44 / 100, y + d * 69 / 100, x + d * 74 / 100, y + d * 34 / 100, colour, t);
            } else {
                self.line(dc, x + d * 32 / 100, y + d * 32 / 100, x + d * 68 / 100, y + d * 68 / 100, colour, t);
                self.line(dc, x + d * 68 / 100, y + d * 32 / 100, x + d * 32 / 100, y + d * 68 / 100, colour, t);
            }
        }

        /// a panel with a mark, a headline and a message; its height back.
        unsafe fn result_panel(&self, dc: HDC, top: i32, good: Option<bool>, headline: &str, message: &str) -> i32 {
            let (left, right) = (self.px(40), self.px(WIDTH - 40));
            let inner_left = left + self.px(if good.is_some() { 58 } else { 20 });
            let width = right - self.px(20) - inner_left;
            let hh = self.measure(dc, self.fonts.bold, width, headline);
            let mh = if message.is_empty() { 0 } else { self.measure(dc, self.fonts.body, width, message) };
            let height = self.px(18) + hh + if mh > 0 { self.px(6) + mh } else { 0 } + self.px(18);
            let border = match good {
                Some(true) => rgb(92, 140, 76),
                Some(false) => rgb(170, 84, 64),
                None => TRIM,
            };
            self.round_rect(dc, RECT { left, top, right, bottom: top + height }, PANEL, border, self.px(14), self.px(1));
            if let Some(good) = good {
                self.mark(dc, left + self.px(18), top + self.px(16), good);
            }
            let headline_colour = match good {
                Some(true) => OK,
                Some(false) => BAD,
                None => GOLD,
            };
            self.text(dc, self.fonts.bold, headline_colour, RECT { left: inner_left, top: top + self.px(18), right: inner_left + width, bottom: top + self.px(18) + hh }, headline, DT_WORDBREAK);
            if mh > 0 {
                let mt = top + self.px(18) + hh + self.px(6);
                self.text(dc, self.fonts.body, CREAM, RECT { left: inner_left, top: mt, right: inner_left + width, bottom: mt + mh }, message, DT_WORDBREAK);
            }
            height
        }

        unsafe fn draw(&self, dc: HDC, w: i32, h: i32) {
            // the wallpaper: a dark teal gradient
            for y in 0..h {
                let t = y as f32 / h as f32;
                let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
                let colour = rgb(mix(BG_TOP.0, BG_BOTTOM.0), mix(BG_TOP.1, BG_BOTTOM.1), mix(BG_TOP.2, BG_BOTTOM.2));
                self.fill(dc, RECT { left: 0, top: y, right: w, bottom: y + 1 }, colour);
            }
            // the header: the name, and a gold rule with a diamond
            let s = |v: i32| self.px(v);
            self.text(dc, self.fonts.title, GOLD, RECT { left: s(40), top: s(20), right: s(400), bottom: s(70) }, "Haiku", DT_SINGLELINE);
            self.text(dc, self.fonts.small, MUTED, RECT { left: s(40), top: s(66), right: s(500), bottom: s(86) }, "Fire Emblem Fates, recompiled for PC", DT_SINGLELINE);
            let rule = s(96);
            self.line(dc, s(40), rule, w / 2 - s(10), rule, TRIM, 1);
            self.line(dc, w / 2 + s(10), rule, w - s(40), rule, TRIM, 1);
            let c = w / 2;
            for (x0, y0, x1, y1) in [(c - s(6), rule, c, rule - s(6)), (c, rule - s(6), c + s(6), rule), (c + s(6), rule, c, rule + s(6)), (c, rule + s(6), c - s(6), rule)] {
                self.line(dc, x0, y0, x1, y1, GOLD, 1);
            }
            // the steps, top right
            let steps = [("1", "Game", Screen::Game), ("2", "DLC", Screen::Dlc), ("3", "Build", Screen::Build), ("4", "Play", Screen::Ready)];
            let current = match self.screen {
                Screen::Failed => Screen::Build,
                Screen::Voices | Screen::Remove => Screen::Ready, // jpvoice; versions2
                other => other,
            };
            let mut x = w - s(40);
            for (n, name, screen) in steps.iter().rev() {
                let label = format!("{n} {name}");
                let width = s(14 + 9 * label.len() as i32);
                x -= width;
                let colour = if *screen == current { GOLD } else { MUTED };
                let font = if *screen == current { self.fonts.bold } else { self.fonts.small };
                self.text(dc, font, colour, RECT { left: x, top: s(40), right: x + width, bottom: s(62) }, &label, DT_SINGLELINE | DT_RIGHT);
            }
            let content = RECT { left: s(40), top: s(116), right: w - s(40), bottom: h - s(80) };
            match self.screen {
                Screen::Game => self.draw_game(dc, content),
                Screen::Dlc => self.draw_dlc(dc, content),
                Screen::Build => self.draw_build(dc, content),
                Screen::Failed => self.draw_failed(dc, content),
                Screen::Ready => {
                    self.draw_ready(dc, content);
                    self.draw_ready_voices(dc); // jpvoice
                }
                Screen::Voices => self.draw_voices(dc, content), // jpvoice
                Screen::Remove => self.draw_remove(dc, content), // versions2
            }
            for b in &self.buttons {
                let hover = self.hover == Some(b.id) && b.enabled;
                let (fill, border, colour) = match (b.enabled, b.primary) {
                    (false, _) => (DISABLED, rgb(70, 84, 84), rgb(120, 126, 120)),
                    (true, true) => (if hover { RED_HOVER } else { RED_BUTTON }, GOLD, CREAM),
                    (true, false) => (if hover { SECONDARY_HOVER } else { SECONDARY }, TRIM, CREAM),
                };
                self.round_rect(dc, b.rect, fill, border, s(12), s(if b.primary { 2 } else { 1 }));
                let font = if b.id == Id::Play { self.fonts.heading } else { self.fonts.button };
                self.text(dc, font, colour, b.rect, &b.label, DT_SINGLELINE | DT_CENTER | DT_VCENTER);
            }
        }

        unsafe fn heading(&self, dc: HDC, r: RECT, title: &str, sub: &str) {
            let s = |v: i32| self.px(v);
            self.text(dc, self.fonts.heading, CREAM, RECT { left: r.left, top: r.top, right: r.right, bottom: r.top + s(34) }, title, DT_SINGLELINE);
            self.text(dc, self.fonts.body, MUTED, RECT { left: r.left, top: r.top + s(38), right: r.right, bottom: r.top + s(90) }, sub, DT_WORDBREAK);
        }

        unsafe fn draw_game(&self, dc: HDC, r: RECT) {
            // versions2: Add a version
            let installed = crate::layout::installed(&self.paths.root);
            if self.adding {
                let have = installed.iter().map(|v| v.folder()).collect::<Vec<_>>().join(", ");
                self.heading(
                    dc,
                    r,
                    "Add a version: select its dump.",
                    &format!("A decrypted dump of another release (US, European or Japanese). It installs beside what is here ({have}) with its own mods, and changes nothing installed."),
                );
            } else {
                self.heading(
                    dc,
                    r,
                    "Select your Fire Emblem Fates Special Edition dump.",
                    "The US, European or Japanese release, decrypted: a .3ds (.cci), .cia or .cxi of the cartridge, or Citra's or Azahar's sdmc folder with it installed. You can also drop it here. It is only read.",
                );
            }
            let top = self.px(274);
            match &self.game {
                None => {}
                Some(Ok(g)) => {
                    // eur: the EUR Special Edition says so
                    let eur = crate::eur::game_is_eur(g);
                    let message = format!(
                        "{}: {}, {} MB.\n{}, and every hash checks out.",
                        self.game_name,
                        g.kind,
                        g.size / 1_000_000,
                        if eur {
                            "Title 000400000017A800 (CTR-P-BFZP), the EUR code Haiku maps its addresses to (English, French, German, Spanish and Italian text)"
                        } else {
                            "Title 0004000000179800 (CTR-P-BFZE), the code Haiku is made for"
                        }
                    );
                    let headline = if eur { "Fire Emblem Fates Special Edition (Europe)" } else { "Fire Emblem Fates Special Edition (USA)" };
                    // jpprep: the Japanese game says so
                    let (headline, message) = if crate::jpn::game_is_jpn(g) {
                        ("Fire Emblem if (Japan)", format!("{}: {}, {} MB.\nThe Japanese game, which Haiku builds with its Japanese address map (Japanese text and voices, the petting minigame, the Japanese DLC), and every hash checks out.", self.game_name, g.kind, g.size / 1_000_000))
                    } else {
                        (headline, message)
                    };
                    // versions2: a version installed already is built again
                    let version = crate::layout::Version::of_region(g.region);
                    let message = if self.adding && installed.contains(&version) {
                        format!("{message}\n{} is installed already: building it again replaces its game files (its mods and saves are kept).", crate::layout::capital(version.name()))
                    } else {
                        message
                    };
                    self.result_panel(dc, top, Some(true), headline, &message);
                }
                Some(Err(p)) => {
                    let headline = match p {
                        dump::Problem::Encrypted(_) => "This dump is encrypted",
                        dump::Problem::WrongRegion(_) => "Another region's Fire Emblem Fates",
                        dump::Problem::SingleRoute(_) => "Not the Special Edition",
                        dump::Problem::WrongGame(_) => "A different game",
                        dump::Problem::WrongVersion(_) => "A different version of the game",
                        dump::Problem::WrongKind(_) => "Not the game",
                        dump::Problem::Corrupt(_) => "This dump is damaged",
                        dump::Problem::NotADump(_) | dump::Problem::Unreadable(_) => "Not a game dump",
                    };
                    let message = if self.game_name.is_empty() { p.message().to_owned() } else { format!("{}: {}", self.game_name, p.message()) };
                    self.result_panel(dc, top, Some(false), headline, &message);
                }
            }
        }

        unsafe fn draw_dlc(&self, dc: HDC, r: RECT) {
            self.heading(
                dc,
                r,
                "Select your DLC.",
                "A decrypted .cia of your game's Fire Emblem Fates DLC (the USA DLC, or the European one for the European game), Citra's or Azahar's sdmc folder with it installed, or the data folder of an earlier Haiku.",
            );
            let top = self.px(274);
            if self.no_dlc {
                self.result_panel(
                    dc,
                    top,
                    None,
                    "Continuing without DLC",
                    "The DLC maps (Before Awakening, the Hidden Truths, the Boo Camp and the rest) won't be available. The story and everything on the cartridge are. You can add the DLC later: Change dump or DLC, then build again.",
                );
                return;
            }
            match &self.dlc_result {
                None => {
                    if self.resumable {
                        self.result_panel(dc, top, None, "A build was stopped before it finished", "Building again carries on where it stopped.");
                    }
                }
                Some(Ok(describe)) => {
                    // eurdlc: the DLC's own region
                    let headline = self.dlcs.first().map_or("Fire Emblem Fates DLC", |d| d.headline());
                    self.result_panel(dc, top, Some(true), headline, &format!("{}: {describe}", self.dlc_name));
                }
                Some(Err(p)) => {
                    let headline = match p {
                        dump::Problem::Encrypted(_) => "This DLC is encrypted",
                        dump::Problem::Corrupt(_) => "This DLC is damaged",
                        dump::Problem::WrongKind(_) => "Not the DLC",
                        dump::Problem::WrongGame(_) => "Not the DLC for this game", // eurdlc
                        _ => "Not Fire Emblem Fates' DLC",
                    };
                    self.result_panel(dc, top, Some(false), headline, &format!("{}: {}", self.dlc_name, p.message()));
                }
            }
        }

        unsafe fn draw_build(&self, dc: HDC, r: RECT) {
            let s = |v: i32| self.px(v);
            let status = self.status.lock().unwrap().clone();
            self.text(dc, self.fonts.heading, CREAM, RECT { left: r.left, top: r.top, right: r.right, bottom: r.top + s(34) }, &format!("{}\u{2026}", status.step.trim_end_matches('.')), DT_SINGLELINE);
            self.text(dc, self.fonts.big, GOLD, RECT { left: r.left, top: r.top + s(44), right: r.right, bottom: r.top + s(104) }, &format!("{:.0}%", status.percent.floor()), DT_SINGLELINE);
            let bar = RECT { left: r.left, top: r.top + s(114), right: r.right, bottom: r.top + s(136) };
            self.round_rect(dc, bar, rgb(12, 32, 36), TRIM, s(10), 1);
            let fill_w = ((bar.right - bar.left - s(6)) as f32 * (status.percent / 100.0).clamp(0.0, 1.0)) as i32;
            if fill_w > s(4) {
                self.round_rect(dc, RECT { left: bar.left + s(3), top: bar.top + s(3), right: bar.left + s(3) + fill_w, bottom: bar.bottom - s(3) }, rgb(196, 150, 72), rgb(232, 190, 110), s(8), 1);
            }
            self.text(dc, self.fonts.body, CREAM, RECT { left: r.left, top: r.top + s(146), right: r.right, bottom: r.top + s(170) }, &status.detail, DT_SINGLELINE | DT_END_ELLIPSIS);
            let note = if self.updating.is_some() {
                "This version of Haiku needs the game's code built again, on your PC, from your own game: only what changed is compiled. The game starts by itself when it's done. It runs at low priority, so you can keep using the computer."
            } else {
                "The game's code is built once, on your PC, from your own dump: it is recompiled and compiled here. That takes from about ten minutes to half an hour, depending on the PC. It runs at low priority, so you can keep using the computer."
            };
            self.text(dc, self.fonts.body, MUTED, RECT { left: r.left, top: r.top + s(186), right: r.right, bottom: r.top + s(260) }, note, DT_WORDBREAK);
            self.text(dc, self.fonts.small, MUTED, RECT { left: r.left, top: r.top + s(266), right: r.right - s(170), bottom: r.top + s(290) }, "The log is setup-work\\setup.log, beside Haiku.exe.", DT_SINGLELINE | DT_END_ELLIPSIS);
        }

        unsafe fn draw_failed(&self, dc: HDC, r: RECT) {
            let s = |v: i32| self.px(v);
            let stopped = self.failure.starts_with("Stopped");
            let title = if stopped { "The build is stopped." } else { "The build didn't finish." };
            self.text(dc, self.fonts.heading, if stopped { CREAM } else { BAD }, RECT { left: r.left, top: r.top, right: r.right, bottom: r.top + s(34) }, title, DT_SINGLELINE);
            let percent = self.status.lock().unwrap().percent;
            let sub = format!("It stopped at {:.0}%. What is done is kept: building again carries on from there.", percent.floor());
            self.text(dc, self.fonts.body, MUTED, RECT { left: r.left, top: r.top + s(38), right: r.right, bottom: r.top + s(64) }, &sub, DT_WORDBREAK);
            if stopped {
                self.result_panel(dc, r.top + s(80), None, "Stopped as you asked", "Try again now, or start Haiku again later, to carry on.");
            } else {
                self.result_panel(dc, r.top + s(80), Some(false), "What went wrong", &format!("{}\n\nThe log is setup-work\\setup.log, beside Haiku.exe.", self.failure));
            }
        }

        unsafe fn draw_ready(&self, dc: HDC, r: RECT) {
            let s = |v: i32| self.px(v);
            self.text(dc, self.fonts.heading, CREAM, RECT { left: r.left, top: r.top, right: r.right, bottom: r.top + s(34) }, "Haiku is ready.", DT_SINGLELINE);
            let dlc_dir = self.paths.data().join("dlc");
            let dlc_count = std::fs::read_dir(&dlc_dir).map(|e| e.flatten().count()).unwrap_or(0);
            let dlc_line = if dlc_count > 0 { "The DLC is installed." } else { "No DLC is installed: the DLC maps aren't available. Change dump or DLC to add it." };
            let built = if self.built_note.is_empty() { String::new() } else { format!(" {}", self.built_note) };
            // versions: which are installed, and which plays
            let installed = crate::layout::installed(&self.paths.root);
            let versions = if installed.len() > 1 {
                let playing = crate::layout::active(&self.paths.root);
                // versions2: one line (the version's long name wrapped it)
                format!("\nInstalled: {}; playing {}. Options, Interface, Game Version switches.", installed.iter().map(|v| v.folder()).collect::<Vec<_>>().join(", "), playing.map(|v| v.folder()).unwrap_or("none"))
            } else {
                String::new()
            };
            // versions2: Add a version said while there is one; with more,
            // the line above says how they switch
            let add = if installed.len() > 1 { "" } else { " Add a version installs another (US, European, Japanese) beside this one." };
            let text = format!("Fire Emblem Fates, built from your dump.{built}\n{dlc_line}{versions}\nSaves and settings are kept in userdata\\haiku, beside Haiku.exe.{add}");
            self.text(dc, self.fonts.body, MUTED, RECT { left: r.left, top: r.top + s(40), right: r.right, bottom: r.top + s(150) }, &text, DT_WORDBREAK);
            self.text(dc, self.fonts.small, MUTED, RECT { left: s(300), top: s(272), right: r.right, bottom: s(340) }, "Starts the game. From now on Haiku.exe goes straight to the game; this screen is Options, Interface, Haiku Setup.", DT_WORDBREAK);
        }

        /// versions2: Remove a version: which, what goes and what stays
        unsafe fn draw_remove(&self, dc: HDC, r: RECT) {
            self.heading(dc, r, "Remove a version of the game.", "Choose the version to remove. The others, and every version's saves, aren't touched.");
            let top = self.px(274);
            let installed = crate::layout::installed(&self.paths.root);
            if installed.len() < 2 {
                self.result_panel(dc, top, Some(false), "The only version installed", "Haiku needs one version of the game, so this one can't be removed. Add another version first.");
                return;
            }
            if let Some(error) = &self.remove_error {
                self.result_panel(dc, top, Some(false), "Not removed", error);
                return;
            }
            let (Some(version), Some(summary)) = (self.remove_pick, &self.remove_summary) else { return };
            let size = if summary.bytes >= 1 << 30 { format!("{:.1} GB", summary.bytes as f64 / (1u64 << 30) as f64) } else { format!("{} MB", summary.bytes >> 20) };
            let mods = match summary.mods {
                0 => "its mods folder (empty)".to_owned(),
                1 => "its mods (1 mod)".to_owned(),
                n => format!("its mods ({n} mods)"),
            };
            let voices = if summary.voices { ", with its Japanese voices" } else { "" };
            let mut message = format!(
                "Deleted: games\\{}\\ ({size}, {} files): its game files and code (data\\{voices}), Haiku's files made from it (assets\\), its caches (cache\\) and {mods}.\nKept: every version's saves and your settings (userdata\\haiku), the other versions, and your dump.",
                version.folder(),
                summary.files
            );
            if summary.playing {
                if let Some(next) = summary.next {
                    message.push_str(&format!("\nIt's playing now: {} plays from now on.", next.name()));
                }
            }
            self.result_panel(dc, top, None, &format!("Remove {}?", version.name()), &message);
        }

        fn hit(&self, x: i32, y: i32) -> Option<Id> {
            self.buttons.iter().find(|b| b.enabled && x >= b.rect.left && x < b.rect.right && y >= b.rect.top && y < b.rect.bottom).map(|b| b.id)
        }

        fn primary(&self) -> Option<Id> {
            self.buttons.iter().find(|b| b.primary && b.enabled).map(|b| b.id)
        }
    }

    fn open_path(path: &Path) {
        let file = wide_os(path.as_os_str());
        let op = wide("open");
        // SAFETY: NUL-terminated strings
        unsafe { ShellExecuteW(0, op.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), SW_SHOW) };
    }

    fn lparam_xy(l: LPARAM) -> (i32, i32) {
        ((l & 0xFFFF) as i16 as i32, ((l >> 16) & 0xFFFF) as i16 as i32)
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        match msg {
            WM_PAINT => {
                let mut ps: PAINTSTRUCT = std::mem::zeroed();
                let hdc = BeginPaint(hwnd, &mut ps);
                with_app(|app| app.paint(hdc));
                EndPaint(hwnd, &ps);
                0
            }
            WM_ERASEBKGND => 1,
            WM_TIMER => {
                with_app(|app| {
                    app.poll();
                    app.poll_voices(); // jpvoice
                    app.step_script();
                });
                0
            }
            WM_MOUSEMOVE => {
                let (x, y) = lparam_xy(l);
                with_app(|app| {
                    let hover = app.hit(x, y);
                    if hover != app.hover {
                        app.hover = hover;
                        app.invalidate();
                    }
                });
                let mut tme = TRACKMOUSEEVENT { cbSize: std::mem::size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: hwnd, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
                0
            }
            WM_MOUSELEAVE => {
                with_app(|app| {
                    if app.hover.take().is_some() {
                        app.invalidate();
                    }
                });
                0
            }
            WM_SETCURSOR => {
                if (l & 0xFFFF) == 1 {
                    let over = with_app(|app| app.hover.is_some()).unwrap_or(false);
                    let busy = with_app(|app| app.job.is_some()).unwrap_or(false);
                    SetCursor(LoadCursorW(0, if over { IDC_HAND } else if busy { 32650 } else { IDC_ARROW }));
                    return 1;
                }
                DefWindowProcW(hwnd, msg, w, l)
            }
            WM_LBUTTONDOWN => {
                let (x, y) = lparam_xy(l);
                with_app(|app| app.pressed = app.hit(x, y));
                0
            }
            WM_LBUTTONUP => {
                let (x, y) = lparam_xy(l);
                let clicked = with_app(|app| {
                    let id = app.hit(x, y);
                    let pressed = app.pressed.take();
                    id.filter(|id| Some(*id) == pressed)
                })
                .flatten();
                if let Some(id) = clicked {
                    with_app(|app| app.click(id));
                }
                0
            }
            WM_KEYDOWN => {
                if w == VK_RETURN {
                    if let Some(id) = with_app(|app| app.primary()).flatten() {
                        with_app(|app| app.click(id));
                    }
                }
                0
            }
            WM_DROPFILES => {
                let mut buf = vec![0u16; 4096];
                let n = DragQueryFileW(w, 0, buf.as_mut_ptr(), buf.len() as u32);
                DragFinish(w);
                if n > 0 {
                    let path = from_wide(&buf);
                    with_app(|app| match app.screen {
                        Screen::Game | Screen::Ready => app.set_game(&path),
                        Screen::Dlc => app.set_dlc(&path),
                        Screen::Voices => app.set_voices_dump(&path), // jpvoice
                        _ => {}
                    });
                }
                0
            }
            WM_DPICHANGED => {
                let dpi = (w & 0xFFFF) as u32;
                let rect = *(l as *const RECT);
                with_app(|app| {
                    app.fonts.delete();
                    app.scale = dpi as f32 / 96.0;
                    app.fonts = Fonts::new(app.scale);
                });
                SetWindowPos(hwnd, 0, rect.left, rect.top, rect.right - rect.left, rect.bottom - rect.top, 0x0004 | 0x0010);
                0
            }
            WM_CLOSE => {
                let building = with_app(|app| app.job.is_some() || app.voices.job.is_some()).unwrap_or(false); // jpvoice: the voices' job too
                if building {
                    let text = wide("Haiku is still being built. Stop and close? What is done is kept, and the next start carries on from there.");
                    let caption = wide("Haiku Setup");
                    if MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_YESNO | MB_ICONQUESTION) != IDYES {
                        return 0;
                    }
                    with_app(|app| app.cancel.store(true, Ordering::Relaxed));
                }
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, w, l),
        }
    }

    pub fn run(options: Options, start: Start) {
        let paths = Paths::new(crate::root());
        let _ = std::fs::create_dir_all(&paths.work);
        log::open(&paths.log_file(), false);
        log::line(format!("Haiku's setup started ({start:?})"));
        if let Some(wine) = crate::cc::wine() {
            log::line(format!("running under Wine/Proton ({wine})")); // buildslow (issue #17)
        }
        for line in crate::layout::sweep(&paths.root) {
            log::line(line); // versions2: what a removed version left
        }
        // SAFETY: the window's whole life is in here, on this thread
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
            let instance = GetModuleHandleW(std::ptr::null());
            let class = wide("HaikuSetup");
            let big_icon = LoadImageW(instance, 1, IMAGE_ICON, 256, 256, LR_DEFAULTCOLOR);
            let small_icon = LoadImageW(instance, 1, IMAGE_ICON, GetSystemMetrics(49), GetSystemMetrics(50), LR_DEFAULTCOLOR);
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: 0x0003,
                lpfnWndProc: wndproc,
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: instance,
                hIcon: big_icon,
                hCursor: LoadCursorW(0, IDC_ARROW),
                hbrBackground: 0,
                lpszMenuName: std::ptr::null(),
                lpszClassName: class.as_ptr(),
                hIconSm: small_icon,
            };
            RegisterClassExW(&wc);
            let style = WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX;
            let title = wide("Haiku Setup");
            let hwnd = CreateWindowExW(WS_EX_ACCEPTFILES, class.as_ptr(), title.as_ptr(), style, CW_USEDEFAULT, CW_USEDEFAULT, WIDTH, HEIGHT, 0, 0, instance, std::ptr::null_mut());
            let dpi = GetDpiForWindow(hwnd).max(96);
            let scale = dpi as f32 / 96.0;
            let mut rect = RECT { left: 0, top: 0, right: (WIDTH as f32 * scale) as i32, bottom: (HEIGHT as f32 * scale) as i32 };
            AdjustWindowRectExForDpi(&mut rect, style, 0, WS_EX_ACCEPTFILES, dpi);
            SetWindowPos(hwnd, 0, 0, 0, rect.right - rect.left, rect.bottom - rect.top, 0x0002 | 0x0004 | 0x0010);
            SendMessageW(hwnd, WM_SETICON, ICON_BIG, big_icon);
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL, small_icon);
            DragAcceptFiles(hwnd, 1);

            let state = State::load(&paths);
            let status: Shared = Arc::new(Mutex::new(Status::default()));
            let scripted = !options.script.is_empty(); // versions2
            let mut app = App {
                hwnd,
                scale,
                fonts: Fonts::new(scale),
                paths: paths.clone(),
                state: state.clone(),
                screen: Screen::Game,
                game: None,
                game_name: String::new(),
                dlcs: Vec::new(),
                dlc_result: None,
                dlc_name: String::new(),
                no_dlc: false,
                resumable: false,
                status,
                cancel: Arc::new(AtomicBool::new(false)),
                job: None,
                failure: String::new(),
                built_note: String::new(),
                buttons: Vec::new(),
                hover: None,
                pressed: None,
                shots: options.shots,
                hidden: options.hidden,
                shot_pending: None,
                shot_count: 0,
                script: options.script.into(),
                wait: None,
                voices: voices_ui::VoicesState::new(), // jpvoice
                updating: None,
                // versions2
                adding: false,
                can_return: false,
                remove_pick: None,
                remove_summary: None,
                remove_error: None,
                deleting: Vec::new(),
                scripted,
            };
            // what an earlier run left: a built Haiku, or a choice and steps
            app.restore_choice(); // versions2: as Back to Play does
            // oneexe: built is the game installed and its module made for
            // this Haiku.exe
            let ready = pipeline::ready(&paths);
            let built = ready == pipeline::Ready::Yes;
            app.resumable = !built && state.values.iter().any(|(k, v)| k.starts_with("done.") && !v.is_empty());
            app.screen = if built { Screen::Ready } else { Screen::Game };
            app.can_return = built; // versions2
            if let (Start::Update(why), pipeline::Ready::Update(_)) = (&start, &ready) {
                log::line(format!("update: {why}"));
                app.updating = Some(why.clone());
            }
            let update = app.updating.is_some();
            APP.with(|a| *a.borrow_mut() = Some(app));
            if update {
                with_app(|app| app.start_update());
            }
            if !options.hidden {
                ShowWindow(hwnd, SW_SHOW);
                UpdateWindow(hwnd);
            }
            SetTimer(hwnd, 1, 100, std::ptr::null());
            let mut msg: MSG = std::mem::zeroed();
            while GetMessageW(&mut msg, 0, 0, 0) > 0 {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            // a build still running stops at its next step
            let job = APP.with(|a| {
                a.borrow_mut().as_mut().and_then(|app| {
                    app.cancel.store(true, Ordering::Relaxed);
                    app.job.take()
                })
            });
            if let Some(job) = job {
                let _ = job.join();
            }
            // versions2: a removed version's folder, deleted to the end
            let deleting = APP.with(|a| a.borrow_mut().as_mut().map(|app| std::mem::take(&mut app.deleting)).unwrap_or_default());
            for delete in deleting {
                let _ = delete.join();
            }
        }
        log::line("Haiku's setup closed");
    }
}
