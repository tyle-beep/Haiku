//! updater: Haiku updating itself from its GitHub releases, in the game.
//!
//! - **Check.** At most once a day, on a thread of its own after the game
//!   starts (never the game's thread, never delaying it), Haiku asks
//!   GitHub's API for the latest release
//!   (`GET https://api.github.com/repos/tyle-beep/Haiku/releases/latest`),
//!   sending nothing but `User-Agent: Haiku/<version>` and the API's Accept
//!   header. What it found, when, and a version the player skipped are kept
//!   in `userdata\haiku\update.json`. Options > Interface > Check for Updates
//!   turns it off (no network at all then). Development and test folders (no
//!   `setup\payload.txt` beside Haiku.exe, or `HAIKU_GAME_MODULE` set) and
//!   headless runs never check or update. `HAIKU_UPDATE_CHECK=0` turns it
//!   off, `=1` checks now (past the daily limit, in a headless run too).
//! - **Notice.** A newer release (semantic versions; Haiku's own is built in,
//!   source/VERSION) not skipped puts a notice on the game's Main Menu
//!   (fates_ui/update.rs), which opens its notes, with Install, Later and
//!   Skip This Version.
//! - **Install.** The release's `Haiku.zip` is downloaded (curl, as Zig is,
//!   zig.rs) into `setup-work\update\<version>\`, checked against the
//!   SHA-256 GitHub's API gives for it (`"digest": "sha256:..."`) and against
//!   `Haiku.zip.sig`, its Ed25519 signature by Haiku's release key, whose
//!   public half is below (ed25519.rs). A release without a valid signature
//!   is not installed: the screen offers its download page instead. The zip
//!   is unpacked there and has to be a Haiku release of that version
//!   (`setup\payload.txt` naming it, Haiku.exe). Then the release's files,
//!   and only those, go in: the running Haiku.exe is renamed Haiku.exe.old
//!   (Windows lets a running program be renamed), every other file it
//!   replaces is moved into the update's `old\` folder, and the new ones are
//!   moved in; any failure moves everything back. games\, userdata\, mods,
//!   and anything the release doesn't hold are never touched. Then the new
//!   Haiku.exe starts (it waits for this one to close) and this one exits.
//! - **The next start** (builder/src/upgrade.rs) deletes Haiku.exe.old and
//!   the update's folder, and goes on as after a zip extracted by hand:
//!   upgrade::tidy, layout::migrate, and the game's code built again in the
//!   game's window when the update needs it (updscreen).
//!
//! Any failure (no internet, GitHub's rate limit, a bad hash or signature, a
//! full drive, a file in use) changes nothing and is said in one line.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// the latest release, from GitHub's API.
pub const API: &str = "https://api.github.com/repos/tyle-beep/Haiku/releases/latest";
/// the releases' page, for a release Haiku won't install itself.
pub const PAGE: &str = "https://github.com/tyle-beep/Haiku/releases/latest";
/// the release's assets Haiku reads, by name.
pub const ZIP: &str = "Haiku.zip";
pub const SIG: &str = "Haiku.zip.sig";
/// Haiku's release key, its public half (Ed25519, 32 bytes). The private
/// half is kept outside the repository; dev/release/sign.sh signs a
/// release's Haiku.zip with it.
pub const PUBLIC_KEY: &str = "d1b8f3e0cefb1342722c9fe2eba8d781fe2e58248d8b71788c05dd83ee650160";
/// how long a check lasts, and how soon a failed one is tried again.
const DAY: u64 = 24 * 60 * 60;
const RETRY: u64 = 60 * 60;
/// what a release may never hold: the player's own folders.
const NEVER: [&str; 10] = ["games", "userdata", "mods", "cache", "data", "assets", "setup-work", "tools", "game", "Haiku.exe.old"];

static VERSION: OnceLock<&'static str> = OnceLock::new();

/// Haiku's version (source/VERSION, built in by source/build.rs).
pub fn set_version(version: &'static str) {
    let _ = VERSION.set(version);
}

pub fn version() -> Option<&'static str> {
    VERSION.get().copied()
}

// ---------------------------------------------------------------- versions

/// a version as Haiku's releases are numbered: 1.4, 1.4.1, 1.4.1-test
/// (semantic versioning; missing numbers are 0, so 1.4 is 1.4.0).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    numbers: Vec<u64>,
    pre: Vec<String>,
}

impl Version {
    /// a version from a tag or a file, "v" in front allowed; none when it
    /// isn't one.
    pub fn parse(text: &str) -> Option<Version> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let text = text.split('+').next()?; // build metadata doesn't order
        let (core, pre) = match text.split_once('-') {
            Some((core, pre)) => (core, Some(pre)),
            None => (text, None),
        };
        let numbers: Vec<u64> = core.split('.').map(|n| n.parse().ok()).collect::<Option<_>>()?;
        if numbers.is_empty() || numbers.len() > 4 {
            return None;
        }
        let pre: Vec<String> = match pre {
            Some(pre) if pre.is_empty() || pre.split('.').any(|p| p.is_empty() || !p.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')) => return None,
            Some(pre) => pre.split('.').map(str::to_owned).collect(),
            None => Vec::new(),
        };
        Some(Version { numbers, pre })
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        use std::cmp::Ordering::*;
        let n = self.numbers.len().max(other.numbers.len());
        for i in 0..n {
            let (a, b) = (self.numbers.get(i).copied().unwrap_or(0), other.numbers.get(i).copied().unwrap_or(0));
            if a != b {
                return a.cmp(&b);
            }
        }
        // a pre-release comes before its release
        match (self.pre.is_empty(), other.pre.is_empty()) {
            (true, true) => return Equal,
            (true, false) => return Greater,
            (false, true) => return Less,
            _ => {}
        }
        for (a, b) in self.pre.iter().zip(&other.pre) {
            let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                (Ok(a), Ok(b)) => a.cmp(&b),
                (Ok(_), Err(_)) => Less,
                (Err(_), Ok(_)) => Greater,
                _ => a.cmp(b),
            };
            if order != Equal {
                return order;
            }
        }
        self.pre.len().cmp(&other.pre.len())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// whether theirs is a newer version than ours.
pub fn newer(theirs: &str, ours: &str) -> bool {
    match (Version::parse(theirs), Version::parse(ours)) {
        (Some(theirs), Some(ours)) => theirs > ours,
        _ => false,
    }
}

// ---------------------------------------------------------------- a release

/// what Haiku needs of a release.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Release {
    /// "1.4", from the tag "v1.4"
    pub version: String,
    pub tag: String,
    /// its title ("Haiku v1.4") and notes (GitHub's Markdown)
    pub title: String,
    pub notes: String,
    /// its page on GitHub
    pub page: String,
    /// Haiku.zip: where, how big, its SHA-256 as GitHub gives it (none from
    /// a GitHub that doesn't)
    pub zip_url: String,
    pub zip_size: u64,
    pub zip_sha256: String,
    /// Haiku.zip.sig, none when the release has no signature
    pub sig_url: String,
    pub sig_size: u64,
}

/// the release in GitHub's answer to /releases/latest. local: the test
/// server's answer (HAIKU_UPDATE_URL), whose files may be on plain http
/// on this PC; GitHub's must be https.
pub fn parse_release(json: &str, local: bool) -> Result<Release, String> {
    let value: serde_json::Value = serde_json::from_str(json).map_err(|error| format!("GitHub's answer isn't readable ({error})"))?;
    let text = |key: &str| value.get(key).and_then(|v| v.as_str()).unwrap_or("").to_owned();
    let tag = text("tag_name");
    let version = Version::parse(&tag).map(|_| tag.trim().trim_start_matches(['v', 'V']).to_owned()).ok_or_else(|| format!("the latest release's tag, {tag:?}, isn't a version"))?;
    if !version.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)) {
        return Err(format!("the latest release's tag, {tag:?}, isn't a version"));
    }
    if value.get("draft").and_then(|v| v.as_bool()) == Some(true) {
        return Err("the latest release is a draft".to_owned());
    }
    let mut release = Release { version, tag, title: text("name"), notes: text("body"), page: text("html_url"), ..Release::default() };
    if release.title.trim().is_empty() {
        release.title = format!("Haiku {}", release.version);
    }
    if !url_allowed(&release.page, local) {
        release.page = PAGE.to_owned();
    }
    for asset in value.get("assets").and_then(|v| v.as_array()).map(Vec::as_slice).unwrap_or(&[]) {
        let name = asset.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let url = asset.get("browser_download_url").and_then(|v| v.as_str()).unwrap_or("").to_owned();
        let size = asset.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        if !url_allowed(&url, local) {
            continue;
        }
        match name {
            ZIP => {
                release.zip_url = url;
                release.zip_size = size;
                release.zip_sha256 = asset
                    .get("digest")
                    .and_then(|v| v.as_str())
                    .and_then(|digest| digest.strip_prefix("sha256:"))
                    .filter(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
                    .map(str::to_ascii_lowercase)
                    .unwrap_or_default();
            }
            SIG => {
                release.sig_url = url;
                release.sig_size = size;
            }
            _ => {}
        }
    }
    Ok(release)
}

/// whether Haiku may download from url: https, or the test server on this
/// PC.
fn url_allowed(url: &str, local: bool) -> bool {
    url.starts_with("https://") || (local && is_local(url))
}

fn is_local(url: &str) -> bool {
    url.starts_with("http://127.0.0.1:") || url.starts_with("http://localhost:")
}

// ---------------------------------------------------------------- where

/// the Haiku folder this Haiku.exe runs from, when it is a release install
/// (setup\payload.txt beside it); why not, when it isn't.
pub fn release_root() -> Result<PathBuf, String> {
    if std::env::var_os("HAIKU_GAME_MODULE").is_some() {
        return Err("a test folder (HAIKU_GAME_MODULE is set)".to_owned());
    }
    let exe = std::env::current_exe().map_err(|error| format!("Haiku.exe's folder is unknown ({error})"))?;
    let dir = exe.parent().ok_or("Haiku.exe's folder is unknown")?.to_path_buf();
    if !dir.join("setup").join("payload.txt").is_file() {
        return Err(format!("a development folder ({} has no setup\\payload.txt)", dir.display()));
    }
    Ok(dir)
}

/// HAIKU_UPDATE_CHECK: Some(false) off, Some(true) now.
fn forced() -> Option<bool> {
    match std::env::var("HAIKU_UPDATE_CHECK").ok()?.trim() {
        "0" => Some(false),
        "1" => Some(true),
        _ => None,
    }
}

/// the API's address, and whether it is the test server on this PC
/// (HAIKU_UPDATE_URL, only ever an http address on 127.0.0.1 or localhost).
fn api() -> (String, bool) {
    if let Ok(url) = std::env::var("HAIKU_UPDATE_URL") {
        if is_local(&url) {
            return (url, true);
        }
        log::warn!("updater: HAIKU_UPDATE_URL is not on this PC, so it is not used: {url}");
    }
    (API.to_owned(), false)
}

fn user_agent() -> String {
    format!("Haiku/{}", version().unwrap_or("0"))
}

// ---------------------------------------------------------------- remembered

/// userdata\haiku\update.json
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    /// when GitHub was last asked, and when next (Unix seconds)
    last_check: u64,
    next_check: u64,
    /// a version the player chose to skip
    skipped: String,
    /// the newest release seen, when newer than the Haiku that saw it
    release: Option<Release>,
}

fn saved_path() -> Option<PathBuf> {
    let dir = std::env::var_os("APPDATA").filter(|dir| !dir.is_empty())?;
    Some(PathBuf::from(dir).join("haiku").join("update.json"))
}

impl Saved {
    fn load() -> Saved {
        saved_path().and_then(|path| std::fs::read_to_string(path).ok()).and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    fn save(&self) {
        let Some(path) = saved_path() else { return };
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let Ok(text) = serde_json::to_string_pretty(self) else { return };
        // written whole and then renamed, so a crash never leaves half a file
        let part = path.with_extension("json.part");
        if std::fs::write(&part, text).is_ok() && std::fs::rename(&part, &path).is_err() {
            let _ = std::fs::remove_file(&part);
        }
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

// ---------------------------------------------------------------- the check

/// the release the notice is for, while there is one.
static FOUND: Mutex<Option<Release>> = Mutex::new(None);
/// the notice shows: a newer release, not skipped, not put off.
static AVAILABLE: AtomicBool = AtomicBool::new(false);
/// whether the Main Menu is up (game_frame, while there is a notice).
static ON_MENU: AtomicBool = AtomicBool::new(false);
static STARTED: AtomicBool = AtomicBool::new(false);
/// the setting, as last given
static SETTING: AtomicBool = AtomicBool::new(true);

/// the release whose notice shows on the Main Menu, while one does.
pub fn notice() -> Option<Release> {
    if !AVAILABLE.load(Ordering::Relaxed) {
        return None;
    }
    FOUND.lock().ok()?.clone()
}

/// whether the game shows its Main Menu now (read only while a notice is
/// waiting).
pub fn on_main_menu() -> bool {
    ON_MENU.load(Ordering::Relaxed)
}

/// starts the check, once, when the game's window opens (or a headless run
/// starts): setting is Options > Interface > Check for Updates.
pub fn start(setting: bool, headless: bool) {
    SETTING.store(setting, Ordering::Relaxed);
    let Some(ours) = version() else { return };
    let forced = forced();
    if forced == Some(false) {
        log::info!("updater: Haiku {ours}; update checks off (HAIKU_UPDATE_CHECK=0)");
        return;
    }
    if let Err(why) = release_root() {
        log::info!("updater: Haiku {ours}; no update checks in {why}");
        return;
    }
    if headless && forced != Some(true) {
        return;
    }
    if !setting {
        log::info!("updater: Haiku {ours}; Check for Updates is off");
        return;
    }
    if STARTED.swap(true, Ordering::Relaxed) {
        return;
    }
    let spawned = std::thread::Builder::new().name("haiku-update-check".to_owned()).spawn(move || check(ours, forced == Some(true)));
    if let Err(error) = spawned {
        log::warn!("updater: the check couldn't start: {error}");
    }
}

/// Check for Updates changed in the options: on, a check starts (if none
/// ran yet); off, the notice goes.
pub fn setting_changed(on: bool) {
    SETTING.store(on, Ordering::Relaxed);
    if on {
        start(true, false);
    } else {
        AVAILABLE.store(false, Ordering::Relaxed);
    }
}

fn check(ours: &'static str, forced: bool) {
    let mut saved = Saved::load();
    // what an earlier check found shows at once, with no network
    publish(&saved, ours);
    let at = now();
    if !forced && at < saved.next_check && saved.next_check <= at + DAY {
        log::info!("updater: Haiku {ours}; GitHub was asked {} min ago, not again until tomorrow", at.saturating_sub(saved.last_check) / 60);
        return;
    }
    // not while the game is starting
    std::thread::sleep(Duration::from_secs(if forced { 1 } else { 8 }));
    let (url, local) = api();
    match latest(&url, local) {
        Ok(release) => {
            log::info!("updater: Haiku {ours}; the latest release is {} ({}): {}", release.version, release.title, if newer(&release.version, ours) { "newer" } else { "not newer" });
            saved.last_check = at;
            saved.next_check = at + DAY;
            saved.release = newer(&release.version, ours).then_some(release);
        }
        Err(error) => {
            log::warn!("updater: the update check failed: {error}; trying again in an hour");
            saved.next_check = at + RETRY;
        }
    }
    // a version skipped meanwhile (the notice from the last check showed
    // while this one ran) is kept
    saved.skipped = Saved::load().skipped;
    saved.save();
    publish(&saved, ours);
}

/// the notice for what saved holds, if anything newer and not skipped.
fn publish(saved: &Saved, ours: &str) {
    let release = saved.release.clone().filter(|release| newer(&release.version, ours) && release.version != saved.skipped);
    let show = release.is_some() && SETTING.load(Ordering::Relaxed);
    if let Ok(mut found) = FOUND.lock() {
        *found = release;
    }
    // where the game is gets read again before the notice shows
    if show && !AVAILABLE.load(Ordering::Relaxed) {
        ON_MENU.store(false, Ordering::Relaxed);
    }
    AVAILABLE.store(show, Ordering::Relaxed);
}

/// GitHub's latest release.
fn latest(url: &str, local: bool) -> Result<Release, String> {
    let fetcher = crate::zig::Fetcher::find().ok_or("this PC has neither curl nor wget to ask GitHub with")?;
    let output = fetcher
        .command_as(url, None, 30, &user_agent(), &["Accept: application/vnd.github+json", "X-GitHub-Api-Version: 2022-11-28"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|error| format!("{} could not start ({error})", fetcher.program()))?;
    if !output.status.success() {
        return Err(network_error(&String::from_utf8_lossy(&output.stderr)));
    }
    parse_release(&String::from_utf8_lossy(&output.stdout), local)
}

/// what curl (or wget) said, in a few words for the player.
fn network_error(said: &str) -> String {
    let said = said.trim();
    let first = said.lines().next().unwrap_or("no message").trim();
    if said.contains("403") || said.contains("429") {
        "GitHub is limiting requests right now; try again later".to_owned()
    } else if said.contains("404") {
        "GitHub has no release there".to_owned()
    } else if said.contains("(6)") || said.contains("resolve host") || said.contains("(7)") || said.contains("connect to") {
        "there is no internet connection, or GitHub can't be reached".to_owned()
    } else if said.contains("(28)") || said.contains("timed out") {
        "the connection timed out".to_owned()
    } else if said.contains("(18)") || said.contains("(56)") || said.contains("(52)") || said.contains("(92)") || said.contains("closed") {
        "the connection was dropped".to_owned()
    } else {
        first.trim_start_matches("curl: ").to_owned()
    }
}

/// Later: the notice goes until Haiku starts again.
pub fn later() {
    AVAILABLE.store(false, Ordering::Relaxed);
}

/// Skip This Version: no notice for it again (a newer one gets one).
pub fn skip(version: &str) {
    let mut saved = Saved::load();
    saved.skipped = version.to_owned();
    saved.save();
    AVAILABLE.store(false, Ordering::Relaxed);
    log::info!("updater: the player skipped Haiku {version}");
}

/// a frame of the game: while a notice waits, whether the Main Menu is up
/// (the game's TopMenu proc alive), read every 8th frame. Nothing at all
/// otherwise.
#[inline]
pub fn game_frame(memory: &mut zakuro_core::memory::Memory, frame: u64) {
    if AVAILABLE.load(Ordering::Relaxed) && frame % 8 == 0 {
        watch(memory);
    }
}

fn watch(memory: &mut zakuro_core::memory::Memory) {
    const TOP_MENU: u32 = crate::bottom_scene::name_hash("TopMenu");
    thread_local! {
        static PROCS: std::cell::RefCell<Vec<(u32, u32)>> = const { std::cell::RefCell::new(Vec::new()) };
    }
    let on = PROCS.with(|procs| {
        let mut procs = procs.borrow_mut();
        crate::bottom_scene::live_procs(memory, &mut procs);
        procs.iter().any(|(_, hash)| *hash == TOP_MENU)
    });
    if ON_MENU.swap(on, Ordering::Relaxed) != on {
        log::info!("updater: the Main Menu is {}", if on { "up: the update notice shows" } else { "gone" });
    }
}

// ---------------------------------------------------------------- install

/// where an install has got to.
#[derive(Clone, Debug, PartialEq)]
pub enum Stage {
    Downloading,
    Verifying,
    Unpacking,
    Installing,
    /// installed: the new Haiku.exe, to start
    Ready(PathBuf),
    /// nothing changed; why, in one line, and whether to offer the
    /// download page instead
    Failed { line: String, page: bool },
    Cancelled,
}

pub struct Install {
    pub release: Release,
    stage: Mutex<Stage>,
    arrived: AtomicU64,
    cancel: AtomicBool,
}

impl Install {
    pub fn stage(&self) -> Stage {
        self.stage.lock().map(|stage| stage.clone()).unwrap_or(Stage::Cancelled)
    }

    /// how much of the zip has arrived, 0 to 1.
    pub fn fraction(&self) -> f32 {
        let size = self.release.zip_size.max(1);
        (self.arrived.load(Ordering::Relaxed) as f64 / size as f64).clamp(0.0, 1.0) as f32
    }

    /// stops it, unless the files are already going in (a moment).
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    fn set(&self, stage: Stage) {
        if let Ok(mut now) = self.stage.lock() {
            *now = stage;
        }
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// downloads, checks and installs release on a thread of its own; its
/// stage says how far it got.
pub fn install(release: Release) -> Arc<Install> {
    let job = Arc::new(Install { release, stage: Mutex::new(Stage::Downloading), arrived: AtomicU64::new(0), cancel: AtomicBool::new(false) });
    let worker = job.clone();
    let spawned = std::thread::Builder::new().name("haiku-update-install".to_owned()).spawn(move || {
        let stage = match release_root().map_err(|why| Failure::plain(format!("Haiku can't update itself in {why}"))).and_then(|root| run_install(&worker, &root)) {
            Ok(exe) => Stage::Ready(exe),
            Err(_) if worker.cancelled() => {
                log::info!("updater: the update to {} was cancelled; nothing was changed", worker.release.version);
                Stage::Cancelled
            }
            Err(failure) => {
                log::warn!("updater: the update to {} failed: {}{}", worker.release.version, failure.line, failure.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default());
                Stage::Failed { line: failure.line, page: failure.page }
            }
        };
        worker.set(stage);
    });
    if let Err(error) = spawned {
        job.set(Stage::Failed { line: format!("The update couldn't start ({error}). Nothing was changed."), page: false });
    }
    job
}

/// why an install stopped: a line for the player, more for the log, and
/// whether the download page is the way instead.
#[derive(Debug)]
struct Failure {
    line: String,
    detail: Option<String>,
    page: bool,
}

impl Failure {
    fn plain(line: String) -> Failure {
        Failure { line, detail: None, page: false }
    }

    fn page(line: &str) -> Failure {
        Failure { line: line.to_owned(), detail: None, page: true }
    }

    fn io(what: &str, error: &std::io::Error) -> Failure {
        let line = match error.raw_os_error() {
            Some(39 | 112) => "The drive is full. Nothing was changed.".to_owned(),
            Some(32 | 33) => "A file of Haiku's is in use by another program. Nothing was changed; try again later.".to_owned(),
            Some(5) => "Haiku isn't allowed to change the files in its folder. Nothing was changed.".to_owned(),
            _ => format!("{what} failed ({error}). Nothing was changed."),
        };
        Failure { line, detail: Some(format!("{what}: {error}")), page: false }
    }
}

/// the update's folder: setup-work\update\<version>
pub fn staging(root: &Path, version: &str) -> PathBuf {
    root.join("setup-work").join("update").join(version)
}

fn run_install(job: &Install, root: &Path) -> Result<PathBuf, Failure> {
    let release = &job.release;
    let (_, local) = api();
    log::info!("updater: installing Haiku {} over {} in {}", release.version, version().unwrap_or("?"), root.display());
    if release.sig_url.is_empty() {
        return Err(Failure::page("This release has no signature, so Haiku won't install it itself. Download it from its page instead."));
    }
    if release.zip_url.is_empty() || release.zip_sha256.is_empty() || release.zip_size == 0 {
        return Err(Failure::page("GitHub doesn't list this release's Haiku.zip with its SHA-256, so Haiku won't install it itself. Download it from its page instead."));
    }
    if !url_allowed(&release.zip_url, local) || !url_allowed(&release.sig_url, local) {
        return Err(Failure::page("This release's files aren't on https, so Haiku won't install it itself."));
    }
    let key = crate::ed25519::hex::<32>(PUBLIC_KEY).expect("the release key is 32 bytes of hex");
    // the zip, its unpacked files and the old ones' room: about three times
    // the zip
    if let Some(free) = free_space(root) {
        let needed = release.zip_size.saturating_mul(4);
        if free < needed {
            return Err(Failure::plain(format!("The drive is full: the update needs about {} MB free. Nothing was changed.", needed / 1_000_000 + 1)));
        }
    }
    let dir = staging(root, &release.version);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|error| Failure::io("Making the update's folder", &error))?;
    let result = download_and_install(job, root, &dir, &key);
    if result.is_err() {
        // nothing of a failed update stays, the old files are back in place
        let _ = std::fs::remove_dir_all(&dir);
    }
    result
}

fn download_and_install(job: &Install, root: &Path, dir: &Path, key: &[u8; 32]) -> Result<PathBuf, Failure> {
    let release = &job.release;
    let fetcher = crate::zig::Fetcher::find().ok_or_else(|| Failure::plain("This PC has neither curl nor wget to download with. Nothing was changed.".to_owned()))?;
    let agent = user_agent();
    let cancel = &job.cancel;
    // the signature first: no signature, no download
    let sig_path = dir.join(SIG);
    let sig_size = if release.sig_size > 0 { release.sig_size } else { 64 };
    crate::zig::fetch_as(fetcher, &release.sig_url, &sig_path, sig_size, &|_| {}, cancel, Some(&agent)).map_err(|error| download_failure("Haiku.zip.sig", &error))?;
    let signature = read_signature(&sig_path).ok_or_else(|| Failure::page("This release's signature file isn't a signature, so Haiku won't install it. Download it from its page instead."))?;
    // the zip
    let part = dir.join(format!("{ZIP}.part"));
    let arrived = &job.arrived;
    let size = release.zip_size;
    crate::zig::fetch_as(fetcher, &release.zip_url, &part, size, &|fraction| arrived.store((fraction as f64 * size as f64) as u64, Ordering::Relaxed), cancel, Some(&agent))
        .map_err(|error| download_failure("Haiku.zip", &error))?;
    job.arrived.store(size, Ordering::Relaxed);
    if job.cancelled() {
        return Err(Failure::plain("Cancelled.".to_owned()));
    }
    job.set(Stage::Verifying);
    let zip = dir.join(ZIP);
    std::fs::rename(&part, &zip).map_err(|error| Failure::io("Keeping the download", &error))?;
    verify_zip(&zip, size, &release.zip_sha256, key, &signature)?;
    log::info!("updater: Haiku.zip ({size} bytes) matches GitHub's SHA-256 and its signature by Haiku's release key");
    if job.cancelled() {
        return Err(Failure::plain("Cancelled.".to_owned()));
    }
    job.set(Stage::Unpacking);
    let new = dir.join("new");
    let bytes = std::fs::read(&zip).map_err(|error| Failure::io("Reading the download", &error))?;
    let count = crate::haiku_mods::zip_extract(&bytes, &new).map_err(|error| Failure { line: "The download couldn't be unpacked. Nothing was changed.".to_owned(), detail: Some(error), page: false })?;
    drop(bytes);
    let files = new.join("Haiku");
    check_release(&files, &release.version).map_err(|why| Failure { line: "The download isn't a Haiku release of that version, so it wasn't installed. Nothing was changed.".to_owned(), detail: Some(why), page: true })?;
    log::info!("updater: {count} files unpacked into {}", new.display());
    if job.cancelled() {
        return Err(Failure::plain("Cancelled.".to_owned()));
    }
    // from here on it is not cancelled: the files go in, or all go back
    job.set(Stage::Installing);
    let moved = swap(root, &files, &dir.join("old")).map_err(|(what, error)| {
        let mut failure = Failure::io(&what, &error);
        failure.line = match error.raw_os_error() {
            Some(32 | 33) => "A file of Haiku's is in use by another program, so the update was undone. Nothing was changed; try again later.".to_owned(),
            Some(5) => "Haiku isn't allowed to change a file in its folder, so the update was undone. Nothing was changed.".to_owned(),
            Some(39 | 112) => "The drive is full, so the update was undone. Nothing was changed.".to_owned(),
            _ => format!("The update was undone ({what} failed: {error}). Nothing was changed."),
        };
        failure
    })?;
    log::info!("updater: Haiku {} installed: {moved} files in place, Haiku.exe.old and {} are deleted at the next start", release.version, dir.display());
    Ok(root.join("Haiku.exe"))
}

fn download_failure(what: &str, error: &str) -> Failure {
    if error == "cancelled" {
        return Failure::plain("Cancelled.".to_owned());
    }
    let why = network_error(error);
    let mut line = why[..1].to_uppercase() + &why[1..];
    line.push_str(". Nothing was changed.");
    Failure { line, detail: Some(format!("{what}: {error}")), page: false }
}

/// a signature file: 64 bytes, or their 128 hex digits.
fn read_signature(path: &Path) -> Option<[u8; 64]> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() == 64 {
        return bytes.try_into().ok();
    }
    crate::ed25519::hex::<64>(std::str::from_utf8(&bytes).ok()?)
}

/// the zip is what GitHub lists (its size and SHA-256) and what Haiku's
/// release key signed.
fn verify_zip(zip: &Path, size: u64, sha256: &str, key: &[u8; 32], signature: &[u8; 64]) -> Result<(), Failure> {
    use sha2::{Digest, Sha256};
    let length = std::fs::metadata(zip).map_err(|error| Failure::io("Reading the download", &error))?.len();
    if length != size {
        return Err(Failure { line: "The download was cut short or is damaged. Nothing was changed.".to_owned(), detail: Some(format!("{length} bytes arrived, not {size}")), page: false });
    }
    let mut file = std::fs::File::open(zip).map_err(|error| Failure::io("Reading the download", &error))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|error| Failure::io("Reading the download", &error))?;
    let hash: String = hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect();
    if hash != sha256 {
        return Err(Failure { line: "The download doesn't match what GitHub lists for it (its SHA-256 differs), so it wasn't installed. Nothing was changed.".to_owned(), detail: Some(format!("SHA-256 {hash}, GitHub lists {sha256}")), page: false });
    }
    let mut file = std::io::BufReader::with_capacity(1 << 20, std::fs::File::open(zip).map_err(|error| Failure::io("Reading the download", &error))?);
    let good = crate::ed25519::verify_reader(key, signature, &mut file).map_err(|error| Failure::io("Reading the download", &error))?;
    if !good {
        return Err(Failure { line: "Haiku.zip isn't signed by Haiku's release key, so it wasn't installed. Nothing was changed.".to_owned(), detail: Some("the Ed25519 signature doesn't verify".to_owned()), page: true });
    }
    Ok(())
}

/// the unpacked release is Haiku's, of version: Haiku.exe, and
/// setup\payload.txt naming the version; nothing of the player's.
pub fn check_release(files: &Path, version: &str) -> Result<(), String> {
    if !files.join("Haiku.exe").is_file() {
        return Err("it has no Haiku\\Haiku.exe".to_owned());
    }
    let payload = std::fs::read_to_string(files.join("setup").join("payload.txt")).map_err(|_| "it has no Haiku\\setup\\payload.txt".to_owned())?;
    let named = payload.split_whitespace().nth(2).unwrap_or("");
    if named != version {
        return Err(format!("its setup\\payload.txt names version {named:?}, not {version}"));
    }
    let parent = files.parent().ok_or("no folder")?;
    for entry in std::fs::read_dir(parent).map_err(|error| error.to_string())?.flatten() {
        if entry.file_name() != "Haiku" {
            return Err(format!("it holds {} beside the Haiku folder", entry.file_name().to_string_lossy()));
        }
    }
    for entry in std::fs::read_dir(files).map_err(|error| error.to_string())?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if NEVER.iter().any(|never| never.eq_ignore_ascii_case(&name)) {
            return Err(format!("it holds {name}, which is the player's"));
        }
    }
    Ok(())
}

/// every file under dir, as paths relative to it, in order.
fn files_under(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        for entry in std::fs::read_dir(dir.join(&rel))? {
            let entry = entry?;
            let path = rel.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                stack.push(path);
            } else {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// puts the release's files (new, unpacked) into root: the running
/// Haiku.exe renamed Haiku.exe.old, each file replaced moved into old, the
/// new one moved in. Any failure moves every file back where it was. How
/// many files went in; what failed and why.
pub fn swap(root: &Path, new: &Path, old: &Path) -> Result<usize, (String, std::io::Error)> {
    let files = files_under(new).map_err(|error| ("Listing the update's files".to_owned(), error))?;
    // the moves made, to undo: (from, to)
    let mut done: Vec<(PathBuf, PathBuf)> = Vec::new();
    let result = (|| -> Result<(), (String, std::io::Error)> {
        let exe = root.join("Haiku.exe");
        let exe_old = root.join("Haiku.exe.old");
        // an earlier update's, if its next start couldn't delete it
        if exe_old.exists() {
            std::fs::remove_file(&exe_old).map_err(|error| ("Deleting an earlier update's Haiku.exe.old".to_owned(), error))?;
        }
        // Haiku.exe first: if it can't be replaced, nothing else is
        let order = files.iter().filter(|rel| rel.as_os_str() == "Haiku.exe").chain(files.iter().filter(|rel| rel.as_os_str() != "Haiku.exe"));
        for rel in order {
            let target = root.join(rel);
            let keep = if rel.as_os_str() == "Haiku.exe" { exe_old.clone() } else { old.join(rel) };
            test_fail(rel)?;
            if target.exists() {
                if let Some(parent) = keep.parent() {
                    std::fs::create_dir_all(parent).map_err(|error| (format!("Making {}", parent.display()), error))?;
                }
                std::fs::rename(&target, &keep).map_err(|error| (format!("Moving {} aside", rel.display()), error))?;
                done.push((target.clone(), keep));
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| (format!("Making {}", parent.display()), error))?;
            }
            std::fs::rename(new.join(rel), &target).map_err(|error| (format!("Putting {} in place", rel.display()), error))?;
            done.push((new.join(rel), target));
        }
        let _ = exe;
        Ok(())
    })();
    match result {
        Ok(()) => Ok(files.len()),
        Err(failure) => {
            log::warn!("updater: {} failed ({}): undoing {} moves", failure.0, failure.1, done.len());
            for (from, to) in done.iter().rev() {
                if let Err(error) = std::fs::rename(to, from) {
                    log::error!("updater: {} couldn't be moved back to {}: {error}", to.display(), from.display());
                }
            }
            Err(failure)
        }
    }
}

/// tests: HAIKU_UPDATE_TEST_FAIL=<file> makes putting that release file in
/// place fail as a file in use does (os error 32), so the undo can be seen.
fn test_fail(rel: &Path) -> Result<(), (String, std::io::Error)> {
    match std::env::var("HAIKU_UPDATE_TEST_FAIL") {
        Ok(name) if !name.is_empty() && rel.to_string_lossy().replace('\\', "/") == name.replace('\\', "/") => {
            Err((format!("Moving {} aside", rel.display()), std::io::Error::from_raw_os_error(32)))
        }
        _ => Ok(()),
    }
}

/// free bytes on root's drive, none when unknown.
fn free_space(root: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        #[link(name = "kernel32")]
        extern "system" {
            fn GetDiskFreeSpaceExW(dir: *const u16, free: *mut u64, total: *mut u64, total_free: *mut u64) -> i32;
        }
        let wide: Vec<u16> = root.as_os_str().encode_wide().chain([0]).collect();
        let mut free = 0u64;
        // SAFETY: a NUL-terminated path and a u64 to write
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
        (ok != 0).then_some(free)
    }
    #[cfg(not(windows))]
    {
        let _ = root;
        None
    }
}

/// opens a release's page in the player's browser.
pub fn open_page(url: &str) {
    let url = if url.starts_with("https://") { url } else { PAGE };
    log::info!("updater: opening {url}");
    #[cfg(windows)]
    {
        #[link(name = "shell32")]
        extern "system" {
            fn ShellExecuteW(hwnd: *mut std::ffi::c_void, verb: *const u16, file: *const u16, params: *const u16, dir: *const u16, show: i32) -> isize;
        }
        let wide = |s: &str| s.encode_utf16().chain([0]).collect::<Vec<u16>>();
        let (verb, file) = (wide("open"), wide(url));
        // SAFETY: NUL-terminated strings; no owner window
        unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), file.as_ptr(), std::ptr::null(), std::ptr::null(), 1) };
    }
}

/// starts the new Haiku.exe (it waits for this process to close,
/// HAIKU_RESTART_WAIT) with this one's arguments and folder.
pub fn relaunch(exe: &Path) -> std::io::Result<u32> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    let mut command = std::process::Command::new(exe);
    command.args(&args).env("HAIKU_RESTART_WAIT", std::process::id().to_string());
    if let Ok(folder) = std::env::current_dir() {
        command.current_dir(folder);
    }
    // a test's script would run again
    for test in ["FATES_UI_SCRIPT", "HAIKU_UPDATE_TEST_FAIL"] {
        command.env_remove(test);
    }
    if let Ok(child_env) = std::env::var("HAIKU_RESTART_CHILD_ENV") {
        for pair in child_env.split(';').filter_map(|pair| pair.split_once('=')) {
            command.env(pair.0.trim(), pair.1.trim());
        }
        command.env_remove("HAIKU_RESTART_CHILD_ENV");
    }
    command.spawn().map(|child| child.id())
}

// ---------------------------------------------------------------- the notes

/// a line of the notes as the screen draws it.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Heading(String),
    Text(String),
    Bullet(String),
    Blank,
}

/// GitHub's Markdown as plain lines: headings, bullets (a table's rows
/// too), paragraphs; links as their text, emphasis and code marks gone,
/// code blocks as plain lines.
pub fn notes_lines(markdown: &str) -> Vec<Line> {
    let mut out = Vec::new();
    let mut fenced = false;
    let mut header: Option<Vec<String>> = None;
    for raw in markdown.lines() {
        let line = raw.trim_end();
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            if !trimmed.is_empty() {
                out.push(Line::Text(trimmed.to_owned()));
            }
            continue;
        }
        if trimmed.is_empty() {
            header = None;
            if out.last().is_some_and(|last| *last != Line::Blank) {
                out.push(Line::Blank);
            }
            continue;
        }
        // a table: its header row, the rule under it, then each row as a
        // bullet, its cells joined
        if trimmed.starts_with('|') {
            let cells: Vec<String> = trimmed.trim_matches('|').split('|').map(|cell| inline(cell.trim())).collect();
            if cells.iter().all(|cell| !cell.is_empty() && cell.chars().all(|c| matches!(c, '-' | ':' | ' '))) {
                continue;
            }
            if header.is_none() {
                header = Some(cells);
                continue;
            }
            out.push(Line::Bullet(cells.into_iter().filter(|cell| !cell.is_empty()).collect::<Vec<_>>().join(": ")));
            continue;
        }
        if trimmed.chars().all(|c| matches!(c, '-' | '*' | '_' | ' ')) && trimmed.len() >= 3 {
            continue; // a rule
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            let text = inline(rest.trim_start_matches('#').trim());
            if !text.is_empty() {
                if out.last().is_some_and(|last| *last != Line::Blank) {
                    out.push(Line::Blank);
                }
                out.push(Line::Heading(text));
            }
            continue;
        }
        let bullet = ["- ", "* ", "+ "].iter().find_map(|mark| trimmed.strip_prefix(mark)).or_else(|| {
            let digits = trimmed.chars().take_while(char::is_ascii_digit).count();
            (digits > 0).then(|| trimmed[digits..].strip_prefix(". ")).flatten()
        });
        match bullet {
            Some(text) => {
                let indent = line.len() - trimmed.len();
                let text = inline(text);
                out.push(Line::Bullet(if indent >= 2 { format!("  {text}") } else { text }));
            }
            None => {
                let text = inline(trimmed.trim_start_matches('>').trim());
                // a paragraph's lines run on
                match out.last_mut() {
                    Some(Line::Text(last)) if !raw.starts_with(' ') => {
                        last.push(' ');
                        last.push_str(&text);
                    }
                    Some(Line::Bullet(last)) if raw.starts_with(' ') => {
                        last.push(' ');
                        last.push_str(&text);
                    }
                    _ => out.push(Line::Text(text)),
                }
            }
        }
    }
    while out.last() == Some(&Line::Blank) {
        out.pop();
    }
    out
}

/// a line's Markdown marks taken out: [text](url) as text, `code`, **bold**,
/// *em*, _em_, ~~gone~~, <tags>, \escapes.
fn inline(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() => {
                out.push(chars[i + 1]);
                i += 2;
            }
            '!' if chars.get(i + 1) == Some(&'[') => i += 1,
            '[' => {
                // [text](url): the text
                if let Some(close) = chars[i + 1..].iter().position(|&c| c == ']').map(|p| p + i + 1) {
                    if chars.get(close + 1) == Some(&'(') {
                        if let Some(end) = chars[close + 2..].iter().position(|&c| c == ')').map(|p| p + close + 2) {
                            out.extend(&chars[i + 1..close]);
                            i = end + 1;
                            continue;
                        }
                    }
                }
                out.push(c);
                i += 1;
            }
            '*' | '`' | '~' => i += 1,
            '_' if (i == 0 || !chars[i - 1].is_alphanumeric()) || (i + 1 >= chars.len() || !chars[i + 1].is_alphanumeric()) => i += 1,
            '<' => match chars[i..].iter().position(|&c| c == '>') {
                Some(end) if chars[i + 1..i + end].iter().all(|c| c.is_ascii_alphanumeric() || "/ =\"'-:.".contains(*c)) => i += end + 1,
                _ => {
                    out.push(c);
                    i += 1;
                }
            },
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    let out = out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&nbsp;", " ");
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

// ---------------------------------------------------------------- snapshots

/// snapshot (fates_ui/snapshot.rs, FATES_UI_SNAPSHOT): a release as a check
/// finds one, its notice up on the Main Menu (none: no notice).
pub(crate) fn snapshot_notice(release: Option<Release>) {
    let show = release.is_some();
    if let Ok(mut found) = FOUND.lock() {
        *found = release;
    }
    ON_MENU.store(show, Ordering::Relaxed);
    AVAILABLE.store(show, Ordering::Relaxed);
}

/// snapshot: a release as GitHub gives one, its notes like Haiku's.
pub(crate) fn snapshot_release() -> Release {
    Release {
        version: "1.4".into(),
        tag: "v1.4".into(),
        title: "Haiku v1.4".into(),
        notes: "# Haiku 1.4

Haiku updates itself now: a notice on the **Main Menu** when a newer version is out, its notes, and *Install*.

## What's new

- Updates from the game: the download is checked against GitHub's SHA-256 and Haiku's own signature before anything changes.
- Options > Interface > `Check for Updates` turns the daily check off.
- Your saves, settings, mods and installed versions stay as they are.

## Fixes

1. A long line, to show how the notes wrap: the quick brown fox jumps over the lazy dog, again and again, until it needs two lines on the panel.
2. See the [README](https://github.com/tyle-beep/Haiku#readme).

## Downloads

| File | Size |
|---|---|
| `Haiku.zip` | 48.3 MB |
| `Haiku.zip.sig` | 64 bytes |

## Known issues

- None known.
".into(),
        page: "https://github.com/tyle-beep/Haiku/releases/tag/v1.4".into(),
        zip_url: "https://github.com/tyle-beep/Haiku/releases/download/v1.4/Haiku.zip".into(),
        zip_size: 48_256_131,
        zip_sha256: "0".repeat(64),
        sig_url: "https://github.com/tyle-beep/Haiku/releases/download/v1.4/Haiku.zip.sig".into(),
        sig_size: 64,
    }
}

impl Install {
    /// snapshot: an install at stage, fraction of the zip arrived.
    pub(crate) fn snapshot(release: Release, stage: Stage, fraction: f64) -> Arc<Install> {
        let arrived = (release.zip_size as f64 * fraction) as u64;
        Arc::new(Install { release, stage: Mutex::new(stage), arrived: AtomicU64::new(arrived), cancel: AtomicBool::new(false) })
    }
}

// ---------------------------------------------------------------- the CLI

/// `Haiku.exe --verify-update <Haiku.zip> <Haiku.zip.sig>`: whether the
/// signature is Haiku's release key's (dev/release/sign.sh runs it on the
/// release's own Haiku.exe). `Haiku.exe --update-check`: asks GitHub (or
/// HAIKU_UPDATE_URL) now and prints what it found, saving nothing.
/// `Haiku.exe --update-install`: the same, then, when it is newer, installs
/// it as the screen's Install does (in a release install only), printing
/// each stage, without starting it (the tests' way to try every failure).
pub fn cli(args: &[String]) -> Option<i32> {
    match args.first().map(String::as_str) {
        Some("--version") => {
            println!("Haiku {}", version().unwrap_or("?"));
            Some(0)
        }
        Some("--verify-update") => {
            let (Some(zip), Some(sig)) = (args.get(1), args.get(2)) else {
                eprintln!("usage: Haiku.exe --verify-update <Haiku.zip> <Haiku.zip.sig>");
                return Some(2);
            };
            let key = crate::ed25519::hex::<32>(PUBLIC_KEY).expect("32 bytes");
            let Some(signature) = read_signature(Path::new(sig)) else {
                eprintln!("{sig} isn't a signature (64 bytes)");
                return Some(1);
            };
            let good = std::fs::File::open(zip).and_then(|file| crate::ed25519::verify_reader(&key, &signature, &mut std::io::BufReader::with_capacity(1 << 20, file)));
            match good {
                Ok(true) => {
                    println!("{zip}: signed by Haiku's release key ({PUBLIC_KEY})");
                    Some(0)
                }
                Ok(false) => {
                    println!("{zip}: NOT signed by Haiku's release key ({PUBLIC_KEY})");
                    Some(1)
                }
                Err(error) => {
                    eprintln!("{zip}: {error}");
                    Some(1)
                }
            }
        }
        Some("--update-check") | Some("--update-install") => {
            let installing = args[0] == "--update-install";
            let (url, local) = api();
            println!("Haiku {}; asking {url}", version().unwrap_or("?"));
            match latest(&url, local) {
                Ok(release) => {
                    let ours = version().unwrap_or("0");
                    println!("latest: {} (tag {}), {}", release.version, release.tag, if newer(&release.version, ours) { "newer" } else { "not newer" });
                    println!("title: {}", release.title);
                    println!("page: {}", release.page);
                    println!("zip: {} ({} bytes, sha256 {})", release.zip_url, release.zip_size, if release.zip_sha256.is_empty() { "none" } else { &release.zip_sha256 });
                    println!("sig: {}", if release.sig_url.is_empty() { "none: it won't be installed by Haiku itself" } else { &release.sig_url });
                    println!("notes: {} lines", notes_lines(&release.notes).len());
                    if !installing {
                        return Some(0);
                    }
                    if !newer(&release.version, ours) {
                        println!("install: not newer, nothing to install");
                        return Some(0);
                    }
                    let job = install(release);
                    let mut last = String::new();
                    loop {
                        let stage = job.stage();
                        let now = format!("{stage:?}");
                        if now != last {
                            println!("install: {now}");
                            last = now;
                        }
                        match stage {
                            Stage::Ready(_) => return Some(0),
                            Stage::Failed { .. } | Stage::Cancelled => return Some(1),
                            _ => std::thread::sleep(Duration::from_millis(20)),
                        }
                    }
                }
                Err(error) => {
                    println!("the check failed: {error}");
                    Some(1)
                }
            }
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_order_as_semantic_versions() {
        assert!(newer("1.4", "1.3"));
        assert!(newer("v1.4", "1.3.9"));
        assert!(!newer("1.4", "1.4"));
        assert!(!newer("1.4.0", "1.4"));
        assert!(newer("1.4.1-test", "1.4"));
        assert!(!newer("1.4.1-test", "1.4.1"));
        assert!(newer("1.4.1", "1.4.1-test"));
        // release141: 1.4.1 is offered to 1.4, and not to itself
        assert!(newer("1.4.1", "1.4"));
        assert!(newer("v1.4.1", "1.4"));
        assert!(!newer("1.4.1", "1.4.1"));
        assert!(!newer("v1.4.1", "1.4.1"));
        assert!(!newer("1.4", "1.4.1"));
        // release142: 1.4.2 is offered to 1.4.1 and 1.4, and not to itself
        assert!(newer("1.4.2", "1.4.1"));
        assert!(newer("v1.4.2", "1.4.1"));
        assert!(newer("1.4.2", "1.4"));
        assert!(newer("1.4.1", "1.4"));
        assert!(!newer("1.4.2", "1.4.2"));
        assert!(!newer("v1.4.2", "1.4.2"));
        assert!(!newer("1.4.1", "1.4.2"));
        assert!(newer("1.10", "1.9"));
        assert!(newer("1.4.1-beta.11", "1.4.1-beta.2"));
        assert!(newer("1.4.1-rc.1", "1.4.1-beta.2"));
        assert!(!newer("1.3", "1.4"));
        assert!(!newer("nonsense", "1.4"));
        assert!(!newer("1.4", "nonsense"));
        assert_eq!(Version::parse("1.4+build.5"), Version::parse("1.4"));
        assert!(Version::parse("1..4").is_none());
        assert!(Version::parse("1.4-").is_none());
    }

    const SAMPLE: &str = r#"{
        "tag_name": "v1.4", "name": "Haiku v1.4", "draft": false, "prerelease": false,
        "html_url": "https://github.com/tyle-beep/Haiku/releases/tag/v1.4",
        "body": "Haiku 1.4\r\n\r\n## What's new\r\n- **Updates** in the game\r\n",
        "assets": [
            {"name": "Haiku-toolchain-source-w64devkit-2.11.0.zip", "size": 152193966, "digest": "sha256:2c9acf323c86b948c6fec9cba11158ce011d8edd2ce8f55b357f1d730080a557",
             "browser_download_url": "https://github.com/tyle-beep/Haiku/releases/download/v1.4/Haiku-toolchain-source-w64devkit-2.11.0.zip"},
            {"name": "Haiku.zip", "size": 48087603, "digest": "sha256:5D3A04E8C6905C380B361EE62CC146849C6EFDFED4270A4162B17EEC893E49BF",
             "browser_download_url": "https://github.com/tyle-beep/Haiku/releases/download/v1.4/Haiku.zip"},
            {"name": "Haiku.zip.sig", "size": 64,
             "browser_download_url": "https://github.com/tyle-beep/Haiku/releases/download/v1.4/Haiku.zip.sig"}
        ]
    }"#;

    #[test]
    fn a_release_is_read_from_githubs_answer() {
        let release = parse_release(SAMPLE, false).unwrap();
        assert_eq!(release.version, "1.4");
        assert_eq!(release.title, "Haiku v1.4");
        assert_eq!(release.zip_size, 48087603);
        assert_eq!(release.zip_sha256, "5d3a04e8c6905c380b361ee62cc146849c6efdfed4270a4162b17eec893e49bf");
        assert!(release.zip_url.ends_with("/v1.4/Haiku.zip"));
        assert!(release.sig_url.ends_with("/v1.4/Haiku.zip.sig"));
        // plain http is only taken from the test server on this PC
        let http = SAMPLE.replace("https://github.com", "http://127.0.0.1:9");
        assert!(parse_release(&http, false).unwrap().zip_url.is_empty());
        assert!(parse_release(&http, true).unwrap().zip_url.starts_with("http://127.0.0.1:9/"));
        let elsewhere = SAMPLE.replace("https://github.com", "http://example.com");
        assert!(parse_release(&elsewhere, true).unwrap().zip_url.is_empty());
        assert!(parse_release(r#"{"tag_name": "latest"}"#, false).is_err());
        assert!(parse_release("<html>", false).is_err());
    }

    #[test]
    fn the_notes_are_plain_lines() {
        let notes = "# Haiku 1.4\r\n\r\nHaiku 1.4 is **one** program, see the [README](https://x).\r\nIt runs.\r\n\r\n## What's new\r\n\r\n- `Haiku.exe` updates itself\r\n  over https\r\n* second\r\n  - nested\r\n1. first\r\n\r\n| File | Size |\r\n|---|---|\r\n| `Haiku.zip` | 48 MB |\r\n\r\n```\r\n5d3a  Haiku.zip\r\n```\r\n---\r\n";
        let lines = notes_lines(notes);
        assert_eq!(
            lines,
            vec![
                Line::Heading("Haiku 1.4".into()),
                Line::Blank,
                Line::Text("Haiku 1.4 is one program, see the README. It runs.".into()),
                Line::Blank,
                Line::Heading("What's new".into()),
                Line::Blank,
                Line::Bullet("Haiku.exe updates itself over https".into()),
                Line::Bullet("second".into()),
                Line::Bullet("  nested".into()),
                Line::Bullet("first".into()),
                Line::Blank,
                Line::Bullet("Haiku.zip: 48 MB".into()),
                Line::Blank,
                // a code block keeps its spacing
                Line::Text("5d3a  Haiku.zip".into()),
            ]
        );
        assert_eq!(inline("snake_case_name and _em_"), "snake_case_name and em");
    }

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("haiku-update-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_else(|_| "<none>".into())
    }

    /// an install: its exe, setup, the player's folders; a release with the
    /// same files new, and one more
    fn layout(name: &str) -> (PathBuf, PathBuf, PathBuf) {
        let base = folder(name);
        let root = base.join("Haiku");
        for (file, text) in [("Haiku.exe", "old exe"), ("SDL3.dll", "old sdl"), ("README.txt", "old readme"), ("setup/payload.txt", "a 2026-11-19 1.4"), ("setup/w64devkit/bin/gcc.exe", "old gcc"), ("games/USA/data/x", "game"), ("userdata/haiku/settings.toml", "settings"), ("games/USA/mods/m/a", "mod")] {
            put(&root.join(file), text);
        }
        let new = base.join("stage/new/Haiku");
        for (file, text) in [("Haiku.exe", "new exe"), ("SDL3.dll", "new sdl"), ("README.txt", "new readme"), ("setup/payload.txt", "b 2026-12-01 1.4.1"), ("setup/w64devkit/bin/gcc.exe", "new gcc"), ("LICENSES/new.txt", "new licence")] {
            put(&new.join(file), text);
        }
        (root, new, base.join("stage/old"))
    }

    #[test]
    fn a_release_swaps_in_and_leaves_the_players_files() {
        let (root, new, old) = layout("swap");
        check_release(&new, "1.4.1").unwrap();
        assert!(check_release(&new, "1.4.2").is_err());
        assert_eq!(swap(&root, &new, &old).unwrap(), 6);
        assert_eq!(read(&root.join("Haiku.exe")), "new exe");
        assert_eq!(read(&root.join("Haiku.exe.old")), "old exe");
        assert_eq!(read(&root.join("setup/w64devkit/bin/gcc.exe")), "new gcc");
        assert_eq!(read(&root.join("LICENSES/new.txt")), "new licence");
        assert_eq!(read(&old.join("SDL3.dll")), "old sdl");
        assert_eq!(read(&root.join("games/USA/data/x")), "game");
        assert_eq!(read(&root.join("userdata/haiku/settings.toml")), "settings");
        assert_eq!(read(&root.join("games/USA/mods/m/a")), "mod");
    }

    /// a file that can't be moved (in use, as Windows refuses a file open
    /// without delete sharing): every move made is undone
    #[cfg(windows)]
    #[test]
    fn a_file_in_use_undoes_the_whole_swap() {
        use std::os::windows::fs::OpenOptionsExt;
        let (root, new, old) = layout("inuse");
        let held = std::fs::OpenOptions::new().read(true).share_mode(1).open(root.join("setup/w64devkit/bin/gcc.exe")).unwrap();
        let error = swap(&root, &new, &old).unwrap_err();
        assert!(matches!(error.1.raw_os_error(), Some(5 | 32)), "{error:?}");
        drop(held);
        assert_eq!(read(&root.join("Haiku.exe")), "old exe");
        assert!(!root.join("Haiku.exe.old").exists());
        assert_eq!(read(&root.join("SDL3.dll")), "old sdl");
        assert_eq!(read(&root.join("README.txt")), "old readme");
        assert_eq!(read(&root.join("setup/payload.txt")), "a 2026-11-19 1.4");
        assert_eq!(read(&root.join("setup/w64devkit/bin/gcc.exe")), "old gcc");
        assert!(!root.join("LICENSES/new.txt").exists());
        // the new files are back where they were unpacked
        assert_eq!(read(&new.join("Haiku.exe")), "new exe");
    }

    #[test]
    fn a_release_holding_the_players_folders_is_refused() {
        let (_, new, _) = layout("refused");
        put(&new.join("userdata/haiku/settings.toml"), "theirs");
        assert!(check_release(&new, "1.4.1").unwrap_err().contains("userdata"));
    }
}
