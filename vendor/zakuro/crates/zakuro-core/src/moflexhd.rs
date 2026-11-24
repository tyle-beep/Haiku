//! moflexhd: where a movie's time goes (issue #7, higher-resolution MOFLEX
//! movies), for tests: `FATES_MOVIE_STATS=1` has the movie player's stages
//! counted and a headless run end with a `moflexhd:` line.
//!
//! - the game's Mobiclip decoder (VX2DecoderUnpack) and deblocking filter
//!   (MoDeblock), timed in overrides/moflexhd.c by the CPU time of the guest
//!   thread running them (QueryThreadCycleTime: other threads, and time
//!   the thread was not running, are not counted);
//! - the YUV-to-RGB conversion (services::y2r), on the host, and the
//!   writing of its picture to guest memory once the GPU is done with it;
//! - the 2D renderer's reading of a changed movie picture into texels
//!   (source/gfx, `texture_oriented_read`), for textures of 256K texels or
//!   more.
//!
//! Off, nothing is timed and the overrides only test a word.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;

/// the stages, the `kind` of `moflexhd_note`.
pub mod kind {
    /// VX2DecoderUnpack, one picture decoded (cycles).
    pub const DECODE: usize = 0;
    /// MoDeblock run by the game's filter (cycles).
    pub const DEBLOCK: usize = 1;
    /// MoDeblock answered for an HD picture without filtering (count).
    pub const UNDEBLOCKED: usize = 2;
    /// y2r: a conversion, the samples gathered and converted (nanoseconds).
    pub const CONVERT: usize = 3;
    /// y2r: the GPU's work on the output range finished and the picture
    /// written there (nanoseconds).
    pub const PLACED: usize = 4;
    /// the 2D renderer: a large texture read into texels (nanoseconds).
    pub const TEXELS: usize = 5;
    /// the 2D renderer: a large changed texture's bytes synced from the
    /// emulated GPU's buffers (nanoseconds).
    pub const TEX_SYNC: usize = 6;
    /// ... read from guest memory and hashed (nanoseconds).
    pub const TEX_READ: usize = 7;
    /// ... looked up in the texture packs (nanoseconds).
    pub const TEX_PACK: usize = 8;
    pub const COUNT: usize = 9;
}

const NAMES: [&str; kind::COUNT] = ["decode", "deblock", "undeblocked", "y2r", "y2r out", "texels", "texture sync", "texture read", "texture packs"];

struct Stage {
    calls: AtomicU64,
    total: AtomicU64,
    most: AtomicU64,
    /// pixels or texels, where the stage has them.
    pixels: AtomicU64,
}

#[allow(clippy::declare_interior_mutable_const)]
const STAGE: Stage = Stage { calls: AtomicU64::new(0), total: AtomicU64::new(0), most: AtomicU64::new(0), pixels: AtomicU64::new(0) };
static STAGES: [Stage; kind::COUNT] = [STAGE; kind::COUNT];

/// nonzero while counting; overrides/moflexhd.c reads it.
#[no_mangle]
pub static moflexhd_counting: AtomicU32 = AtomicU32::new(0);

/// whether FATES_MOVIE_STATS asks for the counts (read once).
pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let on = std::env::var_os("FATES_MOVIE_STATS").is_some_and(|v| v != "0");
        moflexhd_counting.store(on as u32, Ordering::Relaxed);
        on
    })
}

/// a stage's run: its time (cycles or nanoseconds, by stage) and pixels.
pub fn note(kind: usize, amount: u64, pixels: u64) {
    let Some(stage) = STAGES.get(kind) else { return };
    stage.calls.fetch_add(1, Ordering::Relaxed);
    stage.total.fetch_add(amount, Ordering::Relaxed);
    stage.most.fetch_max(amount, Ordering::Relaxed);
    stage.pixels.fetch_add(pixels, Ordering::Relaxed);
    // FATES_MOVIE_STATS=2: the counts every 60 large textures, for a window
    if kind == kind::TEXELS && stage.calls.load(Ordering::Relaxed) % 60 == 0 && std::env::var_os("FATES_MOVIE_STATS").is_some_and(|v| v == "2") {
        if let Some(line) = summary() {
            eprintln!("{line}");
        }
    }
}

/// for overrides/moflexhd.c: `void moflexhd_note(uint32_t kind, uint64_t
/// cycles, uint32_t pixels);`
#[no_mangle]
pub extern "C" fn moflexhd_note(kind: u32, cycles: u64, pixels: u32) {
    note(kind as usize, cycles, pixels as u64);
}

/// thread cycles per nanosecond, measured once.
fn cycles_per_ns() -> f64 {
    static RATE: OnceLock<f64> = OnceLock::new();
    *RATE.get_or_init(|| {
        #[cfg(target_arch = "x86_64")]
        {
            // QueryThreadCycleTime counts at the time stamp counter's rate
            let (c0, t0) = (unsafe { std::arch::x86_64::_rdtsc() }, std::time::Instant::now());
            std::thread::sleep(std::time::Duration::from_millis(50));
            let (c1, t1) = (unsafe { std::arch::x86_64::_rdtsc() }, std::time::Instant::now());
            (c1 - c0) as f64 / (t1 - t0).as_nanos().max(1) as f64
        }
        #[cfg(not(target_arch = "x86_64"))]
        {
            1.0
        }
    })
}

/// the counts for the end of a headless run, when counting.
pub fn summary() -> Option<String> {
    if !on() {
        return None;
    }
    let rate = cycles_per_ns();
    let mut parts = Vec::new();
    for (k, stage) in STAGES.iter().enumerate() {
        let calls = stage.calls.load(Ordering::Relaxed);
        if calls == 0 {
            continue;
        }
        let ns = |v: u64| if k == kind::DECODE || k == kind::DEBLOCK { v as f64 / rate } else { v as f64 };
        let (total, most, pixels) = (stage.total.load(Ordering::Relaxed), stage.most.load(Ordering::Relaxed), stage.pixels.load(Ordering::Relaxed));
        if k == kind::UNDEBLOCKED {
            parts.push(format!("{} {calls}", NAMES[k]));
        } else {
            let per = ns(total) / calls as f64 / 1e6;
            let per_mpix = if pixels > 0 { format!(", {:.2} ms/Mpixel", ns(total) / 1e6 / (pixels as f64 / 1e6)) } else { String::new() };
            parts.push(format!("{} {calls} x {per:.3} ms (most {:.3} ms{per_mpix})", NAMES[k], ns(most) / 1e6));
        }
    }
    Some(format!("moflexhd: {}", if parts.is_empty() { "no movie".to_owned() } else { parts.join("; ") }))
}

/// tests: FATES_Y2R_VARY=1, every converted picture marked with its number
/// (services::y2r), standing in for a movie whose pictures all differ.
pub fn vary() -> bool {
    static VARY: OnceLock<bool> = OnceLock::new();
    *VARY.get_or_init(|| std::env::var_os("FATES_Y2R_VARY").is_some_and(|v| v != "0"))
}

/// the number of the next marked picture.
pub fn next_picture() -> u32 {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// for overrides/moflexhd.c: a movie too large for the game's decoder (more
/// than 1024 wide or high) ends at once instead of hanging on black; said
/// once a size, in the log, for the modder.
#[no_mangle]
pub extern "C" fn moflexhd_unplayable(width: u32, height: u32) {
    static SAID: std::sync::Mutex<Vec<(u32, u32)>> = std::sync::Mutex::new(Vec::new());
    let mut said = SAID.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if said.contains(&(width, height)) {
        return;
    }
    said.push((width, height));
    log::warn!(
        "moflexhd: a {width}x{height} movie can't be played: the game's movie decoder takes pictures up to 1024 wide and its textures are 1024x1024 at most (the 3DS's limit); the movie is skipped. Up to 1024x576 (16:9) or 960x576 (5:3) plays"
    );
}

/// where the YUV-to-RGB converter wrote its last pictures (guest address,
/// length, and the renderer's frame it last wrote there): the movie
/// player's textures. The renderer keeps only the last few pictures of a
/// texture there (source/gfx moflexhd.rs).
static PICTURES: std::sync::Mutex<Vec<(u32, u32, u64)>> = std::sync::Mutex::new(Vec::new());
static ANY_PICTURE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// selena: the renderer's frame now (source/gfx, each frame), which a
/// picture's place is stamped with.
static FRAME: AtomicU64 = AtomicU64::new(0);

/// selena: a place the converter wrote no picture to for this many of the
/// renderer's frames (1 s at 60) is a movie's no more. A movie writes one
/// every 2 to 4 frames while it plays; once it ends, its memory goes back
/// to the game, which puts portraits, a model's hair and the like there
/// (issue #16: those were taken for movie pictures for the rest of the run).
pub const PICTURE_FRAMES: u64 = 60;

/// selena: the renderer's frame now (source/gfx calls this each frame).
pub fn frame(now: u64) {
    FRAME.store(now, Ordering::Relaxed);
    movie_frame(); // moflexdec
}

/// services::y2r: a picture went to `len` bytes at `addr`.
pub fn picture_written(addr: u32, len: u32) {
    let now = FRAME.load(Ordering::Relaxed);
    let mut pictures = PICTURES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(place) = pictures.iter_mut().find(|p| p.0 == addr && p.1 == len) {
        place.2 = now;
        return;
    }
    pictures.push((addr, len, now));
    if pictures.len() > 8 {
        // the place written longest ago goes
        if let Some(oldest) = pictures.iter().enumerate().min_by_key(|(_, p)| p.2).map(|(i, _)| i) {
            pictures.remove(oldest);
        }
    }
    ANY_PICTURE.store(true, Ordering::Relaxed);
}

/// moflexdec: whether a texture at `addr` is one the converter writes an HD
/// picture into: is_picture's place (where a picture starts, written in
/// the last PICTURE_FRAMES frames), for a picture of more than the
/// console's 400x240 (in bytes, at the most a pixel takes). The game's own
/// movies never are; memory the game reuses after a movie never is.
pub fn is_hd_picture(addr: u32) -> bool {
    if !ANY_PICTURE.load(Ordering::Relaxed) {
        return false;
    }
    let now = FRAME.load(Ordering::Relaxed);
    let pictures = PICTURES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    pictures.iter().any(|&(start, len, at)| len > 400 * 240 * 4 && addr == start && now.saturating_sub(at) <= PICTURE_FRAMES)
}

/// whether a texture at `addr` is one the converter writes pictures into:
/// it starts where a picture was written (selena: not merely inside one),
/// in the last PICTURE_FRAMES frames.
pub fn is_picture(addr: u32) -> bool {
    if !ANY_PICTURE.load(Ordering::Relaxed) {
        return false;
    }
    let now = FRAME.load(Ordering::Relaxed);
    let pictures = PICTURES.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    pictures.iter().any(|&(start, _, at)| addr == start && now.saturating_sub(at) <= PICTURE_FRAMES)
}

#[cfg(test)]
mod selena_tests {
    use super::*;

    /// a movie's texture is one while pictures go there; once they stop,
    /// whatever the game puts at that place (or inside it) is not.
    #[test]
    fn a_place_stops_being_a_picture() {
        frame(1000);
        picture_written(0x3000_0000, 0x6_0000);
        assert!(is_picture(0x3000_0000));
        assert!(!is_picture(0x3000_4000), "inside the picture, not at its start");
        frame(1000 + PICTURE_FRAMES);
        assert!(is_picture(0x3000_0000));
        frame(1001 + PICTURE_FRAMES);
        assert!(!is_picture(0x3000_0000), "no picture for a second");
        picture_written(0x3000_0000, 0x6_0000);
        assert!(is_picture(0x3000_0000), "pictures again");
    }
}

// -- moflexdec: how fast a movie really played ---------------------------------

/// one movie's pictures as the converter turned them out (each picture the
/// player shows is converted once, a picture it drops is not).
struct Movie {
    width: usize,
    lines: usize,
    pictures: u64,
    first: std::time::Instant,
    last: std::time::Instant,
    /// the longest time between two pictures, and how many waits were
    /// longer than two pictures of a 30 a second movie (67 ms)
    longest: std::time::Duration,
    late: u64,
}

static MOVIE: std::sync::Mutex<Option<Movie>> = std::sync::Mutex::new(None);
/// a movie is under way (MOVIE holds it); `movie_frame` looks at nothing else
/// while it is false.
static PLAYING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// services::y2r: a picture of `width` x `lines` converted. A picture of
/// another size, or one after a pause of a second or more, starts a new
/// movie (the one before is said, see `said`).
pub fn picture_converted(width: usize, lines: usize) {
    let now = std::time::Instant::now();
    let mut movie = MOVIE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match movie.as_mut() {
        Some(m) if m.width == width && m.lines == lines && now - m.last < std::time::Duration::from_secs(1) => {
            let gap = now - m.last;
            m.longest = m.longest.max(gap);
            m.late += (gap > std::time::Duration::from_millis(67)) as u64;
            m.pictures += 1;
            m.last = now;
        }
        _ => {
            if let Some(done) = movie.take() {
                said(&done);
            }
            *movie = Some(Movie { width, lines, pictures: 1, first: now, last: now, longest: std::time::Duration::ZERO, late: 0 });
            PLAYING.store(true, Ordering::Relaxed);
        }
    }
}

/// each frame (`frame`): a movie with no picture for a second has ended
/// and is said.
fn movie_frame() {
    if !PLAYING.load(Ordering::Relaxed) {
        return;
    }
    let mut movie = MOVIE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if movie.as_ref().is_some_and(|m| m.last.elapsed() >= std::time::Duration::from_secs(1)) {
        if let Some(done) = movie.take() {
            said(&done);
        }
        PLAYING.store(false, Ordering::Relaxed);
    }
}

/// the movie under way, said now (the end of a headless run).
pub fn movie_end() {
    if let Some(done) = MOVIE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() {
        said(&done);
    }
    PLAYING.store(false, Ordering::Relaxed);
}

/// one line in the log for a movie larger than the console's screen (a
/// modder's: how fast it played, for a report like issue #7), or any movie
/// when FATES_MOVIE_STATS counts.
fn said(m: &Movie) {
    if m.width * m.lines <= 400 * 240 && !on() || m.pictures < 2 {
        return;
    }
    let secs = (m.last - m.first).as_secs_f64();
    let rate = (m.pictures - 1) as f64 / secs.max(1e-9);
    log::info!(
        "moflexhd: a {}x{} movie showed {} pictures in {secs:.2} s, {rate:.2} a second; the longest wait for a picture {:.0} ms, {} waits over 67 ms",
        m.width,
        m.lines,
        m.pictures,
        m.longest.as_secs_f64() * 1e3,
        m.late
    );
}

#[cfg(test)]
mod moflexdec_tests {
    use super::*;

    /// only the place of a picture over 400x240 is an HD picture's: not a
    /// game movie's (400x240, RGB8), not inside one, not once it stopped.
    #[test]
    fn only_an_hd_picture_place_is_one() {
        frame(5000);
        picture_written(0x3100_0000, 400 * 240 * 3);
        picture_written(0x3200_0000, 1024 * 576 * 3);
        assert!(is_picture(0x3100_0000) && !is_hd_picture(0x3100_0000), "the game's own movie");
        assert!(is_hd_picture(0x3200_0000));
        assert!(!is_hd_picture(0x3200_4000), "inside the picture");
        frame(5001 + PICTURE_FRAMES);
        assert!(!is_hd_picture(0x3200_0000), "no picture for a second");
    }
}
