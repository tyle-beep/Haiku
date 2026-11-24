//! hfr: high frame rates and display pacing for the PC port.
//!
//! Fates keeps its own clock in `GameTime` (USA Special Edition v2, at
//! 0x006DCA84). Every pass of nnMain's loop is one game tick:
//!
//! ```text
//! +0x00 ticks       ticks run since boot
//! +0x04 vsyncs      VBlanks counted by the _TickVSync callback
//! +0x08 last        vsyncs at the end of the tick before
//! +0x0C elapsed     vsyncs the last tick took
//! +0x10 step        fewest vsyncs a tick waits for (Graphics::WaitVSync)
//! +0x14 slow        most vsyncs a tick may count as
//! +0x18 delta       clamp(elapsed, step, slow): the tick's time step
//! +0x1C total       sum of delta, the game's own clock in 1/60 s
//! ```
//!
//! `GameTime::ResetFrameStep(n)` sets step, slow and delta to n, so a scene
//! runs at 60/n ticks a second and advances everything by n each tick:
//! about 200 Tick functions read `delta` (see dev/reports/hfr.md). The game
//! is therefore speed-correct at any tick rate the console's VBlank allows,
//! as long as `total` grows at the VBlank rate.
//!
//! This module reads that clock for measurements (ZAKURO_HFR_LOG), and holds
//! the frontend's present pacing for frame rates above the game's.

use std::io::Write;
use std::time::{Duration, Instant};

use zakuro_core::memory::Memory;

/// where GameTime lives in this executable.
pub const GAME_TIME: u32 = 0x006D_CA84;

/// the game's clock, as read from its memory.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct GameTime {
    pub ticks: u32,
    pub vsyncs: u32,
    pub step: u32,
    pub slow: u32,
    pub delta: u32,
    pub total: u32,
}

impl GameTime {
    pub fn read(memory: &mut Memory) -> GameTime {
        let mut raw = [0u8; 32];
        memory.read_bytes(zakuro_core::region::us(GAME_TIME), &mut raw);
        let word = |i: usize| u32::from_le_bytes(raw[i * 4..i * 4 + 4].try_into().unwrap());
        GameTime { ticks: word(0), vsyncs: word(1), step: word(4), slow: word(5), delta: word(6), total: word(7) }
    }

    /// game ticks a second the scene asks for.
    pub fn scene_rate(&self) -> f64 {
        59.8261 / self.step.max(1) as f64
    }
}

/// a log of the game's clock against the wall's, one line a second and one
/// at each change of the scene's frame step, written to the file named by
/// ZAKURO_HFR_LOG.
pub struct ClockLog {
    out: std::fs::File,
    start: Instant,
    last: GameTime,
    /// the step the scene asked for, see scene_step.
    asked: u32,
    first: Option<(Instant, GameTime, u64)>,
    line_at: Instant,
    /// the game's total at the line before.
    line_total: u32,
    frames: u64,
    presents: u64,
    present_times: Vec<f32>,
    last_present: Option<Instant>,
    /// each present's time since the start and since the one before, in
    /// milliseconds, to the file ZAKURO_HFR_FRAMES names.
    raw: Option<std::io::BufWriter<std::fs::File>>,
}

impl ClockLog {
    pub fn from_env() -> Option<ClockLog> {
        let path = std::env::var_os("ZAKURO_HFR_LOG")?;
        let mut out = std::fs::File::create(&path).ok()?;
        let _ = writeln!(out, "# wall_s\tframes\tticks\tvsyncs\tstep\tslow\tdelta\ttotal\tgame_s_per_wall_s\tgame_units_per_vblank\tgame_s_per_wall_s_this_line\tpresents_per_s\tpresent_ms_mean\tpresent_ms_p99\tpresent_ms_max");
        let now = Instant::now();
        Some(ClockLog {
            out,
            start: now,
            last: GameTime::default(),
            asked: 0,
            first: None,
            line_at: now,
            line_total: 0,
            frames: 0,
            presents: 0,
            present_times: Vec::new(),
            last_present: None,
            raw: std::env::var_os("ZAKURO_HFR_FRAMES").and_then(|path| std::fs::File::create(path).ok()).map(std::io::BufWriter::new),
        })
    }

    /// after each emulated frame.
    pub fn frame(&mut self, memory: &mut Memory) {
        self.frames += 1;
        let time = GameTime::read(memory);
        let now = Instant::now();
        if self.first.is_none() && time.total > 0 {
            self.first = Some((now, time, self.frames));
        }
        // the step the scene asked for, before the game-level limit
        let asked = scene_step().unwrap_or(0);
        if asked != self.asked {
            let _ = writeln!(self.out, "# frame {} the scene asks for step {} (was {})", self.frames, asked, self.asked);
            self.asked = asked;
        }
        let changed = time.step != self.last.step || time.slow != self.last.slow;
        if changed {
            let _ = writeln!(
                self.out,
                "# frame {} step {} -> {} (slow {}), the scene runs at {:.2} ticks/s",
                self.frames,
                self.last.step,
                time.step,
                time.slow,
                time.scene_rate()
            );
        }
        self.last = time;
        if changed || now - self.line_at >= Duration::from_secs(1) {
            self.line(now);
        }
    }

    /// after each picture handed to the display.
    pub fn present(&mut self) {
        let now = Instant::now();
        if let Some(last) = self.last_present {
            let interval = (now - last).as_secs_f32() * 1e3;
            self.present_times.push(interval);
            if let Some(raw) = &mut self.raw {
                let _ = writeln!(raw, "{:.3}\t{interval:.3}\t{}", (now - self.start).as_secs_f64() * 1e3, self.frames);
            }
        }
        self.last_present = Some(now);
        self.presents += 1;
    }

    fn line(&mut self, now: Instant) {
        let window = (now - self.line_at).as_secs_f64().max(1e-6);
        let (rate, per_vblank) = match self.first {
            Some((at, first, frames)) if now > at && self.frames > frames => {
                let units = self.last.total.wrapping_sub(first.total) as f64;
                (units / 60.0 / (now - at).as_secs_f64(), units / (self.frames - frames) as f64)
            }
            _ => (0.0, 0.0),
        };
        let mut times = std::mem::take(&mut self.present_times);
        times.sort_by(f32::total_cmp);
        let mean = if times.is_empty() { 0.0 } else { times.iter().sum::<f32>() / times.len() as f32 };
        let p99 = times.get((times.len() * 99 / 100).min(times.len().saturating_sub(1))).copied().unwrap_or(0.0);
        let max = times.last().copied().unwrap_or(0.0);
        let t = self.last;
        let this_line = match self.line_total {
            0 => 0.0,
            before => t.total.wrapping_sub(before) as f64 / 60.0 / window,
        };
        self.line_total = t.total;
        let _ = writeln!(
            self.out,
            "{:.3}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{:.1}\t{:.2}\t{:.2}\t{:.2}",
            (now - self.start).as_secs_f64(),
            self.frames,
            t.ticks,
            t.vsyncs,
            t.step,
            t.slow,
            t.delta,
            t.total,
            rate,
            per_vblank,
            this_line,
            self.presents as f64 / window,
            mean,
            p99,
            max
        );
        self.presents = 0;
        self.line_at = now;
    }
}

/// what the game build exposes of its timing overrides (overrides/timing.c),
/// registered by the program that links the recompiled code.
#[derive(Clone, Copy)]
pub struct GameTiming {
    /// the most VBlanks a game tick may wait for, 0 for the game's own.
    pub set_max_step: fn(u32),
    /// the step the running scene asked for, before the limit.
    pub scene_step: fn() -> u32,
}

static GAME_TIMING: std::sync::OnceLock<GameTiming> = std::sync::OnceLock::new();

/// called once by the program, before zakuro::run.
pub fn set_game_timing(timing: GameTiming) {
    let _ = GAME_TIMING.set(timing);
}

/// the step the running scene asked for, when the game build says.
pub fn scene_step() -> Option<u32> {
    GAME_TIMING.get().map(|timing| (timing.scene_step)())
}

/// puts the game-level frame rate setting to use: with `game_60fps` the
/// scenes Fates paces at 30 ticks a second tick at 60, at the same speed.
pub fn apply_game_timing(pc: &crate::settings::PcSettings) {
    if let Some(timing) = GAME_TIMING.get() {
        let mut limit = if pc.game_60fps && pc.frame_rate != 30 { 1 } else { 0 };
        // for measuring the game's own pacing against this one
        if let Ok(forced) = std::env::var("ZAKURO_HFR_GAME60") {
            limit = if forced == "0" { 0 } else { 1 };
        }
        (timing.set_max_step)(limit);
        log::info!("hfr: game ticks {}", if limit == 1 { "at 60 Hz in every scene" } else { "at the game's own rates" });
    }
}

// ---------------------------------------------------------------------------
// present pacing above the game's rate
// ---------------------------------------------------------------------------

/// one console refresh, as the frontend schedules them.
fn refresh() -> Duration {
    crate::qol::frame_period() // qol: a speed-th of the console's at a game speed
}

/// how the frontend shows pictures faster than the game makes them: the
/// game still runs one console refresh at a time on the wall clock, and
/// between those the frontend presents at its own rate, each present
/// showing the picture in between the game's last two that its time calls
/// for (the GPU draws them, see zakuro_gpu's interpolation and raster
/// LAYERS). The game's speed does not depend on any of it.
#[derive(Default)]
pub struct Pacer {
    /// the rate pictures are presented at, 0 while the old loop runs.
    pub rate: f64,
    /// when the next picture is presented.
    next_present: Option<Instant>,
    /// when the newest picture was latched by the game: the schedule time of
    /// the refresh that latched it, its refresh number, and its gap.
    latched: Option<(Instant, u64, u64)>,
    /// how long emulating a refresh takes, smoothed towards its slow ones.
    emulating: Duration,
    /// the fractions asked for the next picture's pictures in between.
    asked: Vec<f32>,
    /// pictures presented and how many of them were in between.
    pub presented: u64,
    pub in_between: u64,
    /// the display settings last put to use.
    pub display: crate::display::Applied,
    /// the refresh of the newest picture presented, without interpolation.
    shown_refresh: Option<u64>,
    /// whether pictures in between are drawn.
    interpolating: bool,
    /// the game keeps the console's speed first: while the emulation falls
    /// behind, the pictures in between stop, until it keeps up again.
    degraded: bool,
    /// half-seconds in a row the emulation kept up, or fell behind.
    keeping_up: u32,
    falling_behind: u32,
    /// smooth: how far behind its slot the game time a picture shows is,
    /// in seconds (see paced_pass), and whether pictures in between are
    /// offered at many places (the native renderer draws them on demand).
    latency: f64,
    dense: bool,
    /// smooth: the end of the last pass, and the largest recent picture
    /// gaps (the game can alternate pictures every refresh and every other).
    pass_end: Option<Instant>,
    gaps: std::collections::VecDeque<u64>,
    /// gfx-models: the last refreshes' covers (see hfr_refresh)
    covers: std::collections::VecDeque<Duration>,
    /// rt-merge: in real time, the longest a lend kept the frontend past
    /// its wake since the last refresh (a native call, a busy core): the
    /// pictures ahead must cover it as they cover an emulated refresh.
    held: Duration,
    /// rt-merge: zakuro_core::pcos::rt::totals at the last refresh.
    rt_totals: (u64, u64, u64, u64),
    /// rt-merge: how long recording a picture takes, smoothed towards its
    /// slow ones, and whether the pass ended to give the game's unfinished
    /// frame the time first (see paced_pass).
    present_cost: Duration,
    /// rtpace: what drawing and handing over a picture takes in real time
    /// (its picture in between drawn while the game runs, see paced_pass)
    draw_cost: Duration,
    game_first: bool,
    /// smooth2: pictures presented that show a new point of the game's
    /// time (a held frame shown again does not count), the last one's, and
    /// their rate over the last half second: the FPS the corner shows.
    distinct: u64,
    last_content: f64,
    pub shown_rate: Option<f32>,
    /// smooth2: uncapped, the mean cost of recording a picture, and the
    /// rate the present thread's free grid was last given.
    present_mean: Duration,
    free_rate: f64,
    /// vrrcap: variable refresh as last logged.
    vrr_logged: bool,
    /// smooth2: the next present cross-fades the top screen from the
    /// picture before (shown) to the newest, this far (see paced_pass).
    pub crossfade: Option<f32>,
    /// smooth2: pictures recorded with none waiting for the display (it
    /// was about to show the last again) since the last half second, the
    /// half seconds in a row without, and the step of the scale pictures in
    /// between are drawn at (BETWEEN_SCALES).
    starved: u32,
    calm: u32,
    scale_step: usize,
    /// gfxcastle: the last half seconds' starved counts (newest in the low
    /// byte), the calm half seconds a step back up needs now, and when the
    /// scale last went up (see hfr_title)
    starved_history: u32,
    calm_needed: u32,
    stepped_up: Option<Instant>,
    /// mapjit: when the latency was last moved (see paced_pass)
    latency_at: Option<Instant>,
    /// hfr60: presenting at about the console's rate (60) with Smooth 30
    /// FPS Scenes on: pictures in between only while the game runs a scene
    /// at 30 ticks a second or slower (`slow_scene`, GameTime's step at the
    /// last refresh), one halfway between each two of its pictures. The
    /// scenes at 60 are shown as without interpolation, and the renderer
    /// does not match their draws (`gpu_between`: what the GPU was last
    /// told, see hfr_between_sync).
    half_only: bool,
    slow_scene: bool,
    gpu_between: Option<bool>,
    /// hfr60: the game's clock (GameTime total) when it last moved, over the
    /// last seconds while pictures in between are drawn (see clock_rate);
    /// the half seconds in a row it ran slow; how long the emulation must
    /// keep up before they start again (half seconds, doubling while they
    /// keep slowing the game, see hfr_title), and when they last did.
    clock: std::collections::VecDeque<(Instant, u32)>,
    slow_halves: u32,
    recover_need: u32,
    recovered_at: Option<Instant>,
}

/// hfr60: the game's clock in a 30 FPS scene at full speed advances 59.83
/// units (1/60 s) a wall second, as the console's refreshes. Below this,
/// sustained (more than 1.5% slow), the pictures in between take time the
/// game needs.
const CLOCK_SLOW: f64 = 59.83 * 0.985;

impl Pacer {
    /// hfr60: the game's clock rate (units a second) over the last 1.5 to
    /// 2.5 s, from its moves; none for a shorter stretch. A stall over 100 ms
    /// (a scene loading) starts it over, so only a sustained slowing counts,
    /// not a load, which the refreshes' rate (57.5) catches.
    fn clock_rate(&self) -> Option<f64> {
        let (&(t0, c0), &(t1, c1)) = (self.clock.front()?, self.clock.back()?);
        let span = (t1 - t0).as_secs_f64();
        (span >= 1.5).then(|| c1.wrapping_sub(c0) as f64 / span)
    }

    fn clock_moved(&mut self, now: Instant, total: u32) {
        match self.clock.back() {
            Some(&(_, last)) if last == total => return,
            Some(&(at, _)) if now - at > Duration::from_millis(100) => self.clock.clear(),
            _ => {}
        }
        self.clock.push_back((now, total));
        while self.clock.front().is_some_and(|&(at, _)| now - at > Duration::from_millis(2500)) {
            self.clock.pop_front();
        }
    }
}

/// hfr60: the rates around the console's own (a 60 Hz display, 59.94 Hz
/// too, Frame Rate 60, a cap of 60), where only the game's 30 FPS scenes get
/// pictures in between (Smooth 30 FPS Scenes).
pub fn console_rate(rate: f64) -> bool {
    (59.0..=61.0).contains(&rate)
}

/// hfr60: Smooth 30 FPS Scenes as set, or FATES_HFR60=0/1 for tests.
fn smooth_30fps(pc: &crate::settings::PcSettings) -> bool {
    match std::env::var("FATES_HFR60") {
        Ok(v) => v != "0",
        Err(_) => pc.smooth_30fps,
    }
}

/// mapjit: how fast the latency may grow and shrink, as a share of the wall
/// time (see paced_pass): growing, the time shown runs 3% slow; shrinking,
/// 1% fast.
const SLEW_UP: f64 = 0.03;
const SLEW_DOWN: f64 = 0.01;

fn slews() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("FATES_PACE_SLEW").map_or(true, |v| v != "0"))
}

/// smooth2: the fractions of the render scale pictures in between are drawn
/// at while the frontend cannot record them in time (see hfr_title). A
/// picture in between shows for one refresh between two full-scale frames;
/// at the user's 8x and 21:9 one is 7680x1920 and took 4-13 ms to draw.
/// Not below 1/2: at the user's 8x that is 4x, about the display's own
/// width at 21:9; smaller, the softer pictures in between showed between
/// sharp frames.
const BETWEEN_SCALES: [f32; 3] = [1.0, 0.75, 0.5];

/// gfxcastle: the calm half seconds before the scale steps back up, at first
/// and at most (see hfr_title).
const SCALE_CALM: u32 = 16;
const SCALE_CALM_MOST: u32 = 120;

/// the fractions of the way from one picture to the next the GPU draws
/// pictures in between at, for presenting at `rate` a game that makes a
/// picture every `gap` refreshes: as many steps as presents fall in a
/// picture's time, rounded up, the ends being the two pictures themselves
/// (the one before is still on the GPU while the newest one's time runs).
pub fn fractions(rate: f64, gap: u64) -> Vec<f32> {
    let per_picture = rate * gap.max(1) as f64 * refresh().as_secs_f64();
    let whole = (per_picture - per_picture.round()).abs() < 0.03;
    let steps = if whole { per_picture.round() as usize } else { per_picture.ceil() as usize };
    let steps = steps.min(zakuro_gpu::Gpu::MAX_BETWEEN + 1);
    (1..steps).map(|k| k as f32 / steps as f32).collect()
}

/// gfx-models: the gap between pictures the game keeps to: the largest gap
/// that came at least twice among the last 16 (one hitch is no cadence),
/// 1 to 4 refreshes.
pub fn cadence(gaps: &std::collections::VecDeque<u64>) -> u64 {
    let recent: Vec<u64> = gaps.iter().rev().take(16).copied().collect();
    let mut best = 1;
    for &g in &recent {
        if g > best && recent.iter().filter(|&&h| h == g).count() >= 2 {
            best = g;
        }
    }
    best.clamp(1, 4)
}

/// gfx-models: the second largest, none being zero.
fn second_largest(v: &std::collections::VecDeque<Duration>) -> Duration {
    let (mut first, mut second) = (Duration::ZERO, Duration::ZERO);
    for &d in v {
        if d > first {
            second = first;
            first = d;
        } else if d > second {
            second = d;
        }
    }
    if v.len() < 2 { first } else { second }
}

/// smooth2: the rate "uncapped" asks for (VSync off, Frame Rate Display, no
/// cap): pictures go out as fast as they are drawn, no more often than
/// this, which keeps the loop from spinning when they cost nothing.
pub const UNCAPPED: f64 = 1000.0;

/// vrrcap: whether the display's refresh rate is variable (G-SYNC,
/// FreeSync): Options > Display > Variable Refresh, FATES_VRR=1 or 0 over it.
pub fn vrr_on(pc: &crate::settings::PcSettings) -> bool {
    match std::env::var("FATES_VRR").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => pc.vrr,
    }
}

/// the rate pictures are presented at (App::hfr_rate, before
/// ZAKURO_HFR_RATE): `frame_rate` 0 follows the display, `presents` is
/// whether a presenter shows them (with VSync no more than the display).
/// vrrcap: taken out of hfr_rate as it was, the variable refresh limit
/// added, so it can be tested.
pub fn paced_rate(frame_rate: u32, vsync: bool, fps_cap: u32, vrr: bool, display: f64, presents: bool) -> f64 {
    let mut rate = if frame_rate == 0 { display } else { frame_rate as f64 };
    // smooth2: VSync off with no frame rate asked for (Display): as fast
    // as the PC draws them (UNCAPPED), under the cap if there is one
    if !vsync && frame_rate == 0 {
        rate = UNCAPPED;
    }
    // with vsync the display shows no more than its refresh rate (the
    // presenter switches to FIFO at its next picture)
    if vsync && presents {
        rate = rate.min(display);
    }
    if fps_cap > 0 {
        rate = rate.min(fps_cap as f64);
    }
    // vrrcap: with variable refresh on, never above the rate where the
    // display stays in its variable range (Display, VSync off with no
    // cap included: above it the display tears or waits as with VSync)
    if vrr {
        rate = rate.min(vrr_limit(display));
    }
    rate
}

/// vrrcap: the highest rate presents go out at with variable refresh on a
/// display of `hz`: a little under it, as NVIDIA Reflex caps (hz - hz^2 /
/// 3600: 162 at 170 Hz, 166.5 at 175, 138 at 144, 224 at 240). At the
/// display's own rate, presents a refresh apart sit at the top of its
/// variable range, where one a little early waits for the display (as with
/// VSync) or tears; and the refresh grid Haiku otherwise presents on (DWM's
/// last refresh and period) means nothing when the display refreshes as
/// presents come. Under it, on a steady clock of Haiku's own, each picture
/// is shown as it is presented, one even step of the game's time apart.
pub fn vrr_limit(hz: f64) -> f64 {
    let hz = hz.clamp(30.0, 1000.0);
    hz - hz * hz / 3600.0
}

/// smooth2: whether frames with feedback post effects are cross-faded
/// (FATES_CROSSFADE=0 holds them, as before).
fn crossfades() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("FATES_CROSSFADE").map_or(true, |v| v != "0"))
}

/// rt-merge: how long before its slot a picture's recording must end, past
/// its cost (the present thread's own lead, a lend's overrun).
const GAME_FIRST_MARGIN: Duration = Duration::from_micros(2000);

/// rtpace: switches for its changes, on unless the variable is 0.
struct Switch(&'static str, std::sync::OnceLock<bool>);
static RAISE_FIRST: Switch = Switch("FATES_RT_RAISE_FIRST", std::sync::OnceLock::new());
static LEAD: Switch = Switch("FATES_RT_LEAD", std::sync::OnceLock::new());
static SCALE_CAP: Switch = Switch("FATES_RT_SCALE_CAP", std::sync::OnceLock::new());

fn rtpace_switch(switch: &Switch) -> bool {
    *switch.1.get_or_init(|| std::env::var(switch.0).map_or(true, |v| v != "0"))
}

/// sharp: a half second counts towards a step down only when the display
/// really went short of new pictures while the game kept up
/// (FATES_SHARP_STEP=0: any half second with pictures recorded late, as
/// before; that stepped down at every scene's load and on single hitches).
fn sharp_steps() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("FATES_SHARP_STEP").map_or(true, |v| v != "0"))
}

/// rtpace: what a present needs after its picture in between is drawn (the
/// frame's composition and the hand-over to the present thread), so a lend
/// while it is drawn ends this long before the slot at the latest.
const RTPACE_HANDOVER: Duration = Duration::from_micros(1500);

/// what a present shows.
#[derive(Clone, Copy, Debug, PartialEq)] // gfx-models: Before holds an f32
pub enum Pick {
    /// the picture before the newest, at 0.
    Previous,
    /// the newest picture's k-th picture in between, at ready[k - 1].
    Between(usize),
    /// the newest picture itself, at 1.
    Newest,
    /// gfx-models: the frame before's own picture in between, this part of
    /// the way from the picture before it (native frames)
    Before(f32),
    /// castlecam: the newest native frame's picture in between at exactly
    /// this part of the way (see exact_place)
    At(f32),
}

/// castlecam: whether presents show the native renderer's pictures in
/// between at exactly their slot's time (FATES_HFR_EXACT=0: the nearest of
/// the places offered, as before).
fn exact_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("FATES_HFR_EXACT").map_or(true, |v| v != "0"))
}

/// castlecam: the place a present shows the newest frame's time at, when
/// that is exact rather than the nearest offered (`nearest`). The native
/// renderer draws a picture in between only when it is shown, at whatever
/// place it is asked, so the places offered (eighths of the frame's time)
/// were a needless grid: at 175 presents a second a 30 Hz scene moves 0.171
/// of a frame per present, which the eighths turned into steps of 1/8 and
/// 2/8 in turn (a turning camera went on by half a step, then a whole one,
/// several times a frame), a 60 Hz scene steps of 2/8 and 3/8. Exact, each
/// present moves on by the same time. Only for a native frame that has
/// pictures in between (`ready`, which says it looks right drawn so);
/// within 1/256 of either end the frame's own pictures stand.
pub fn exact_place(native: bool, ready: &[f32], at: f64) -> Option<f32> {
    const END: f64 = 1.0 / 256.0;
    (exact_on() && native && !ready.is_empty() && at > END && at < 1.0 - END).then_some(at as f32)
}

/// which picture shows a point a fraction of the way between the picture
/// before and the newest, the nearest there is.
pub fn nearest(ready: &[f32], at: f32, previous: bool) -> Pick {
    let mut best = (Pick::Newest, (1.0 - at).abs());
    if previous && at.abs() < best.1 {
        best = (Pick::Previous, at.abs());
    }
    for (i, &fraction) in ready.iter().enumerate() {
        let distance = (fraction - at).abs();
        if distance < best.1 {
            best = (Pick::Between(i + 1), distance);
        }
    }
    best.0
}

impl Pacer {
    /// smooth2: a present showing the game's time `content` (refreshes).
    fn count_distinct(&mut self, content: f64) {
        if content != self.last_content {
            self.distinct += 1;
            self.last_content = content;
        }
    }

    /// starts over, after a pause or a change of rate.
    pub fn reset(&mut self) {
        self.next_present = None;
        self.latched = None;
        self.shown_refresh = None;
        self.asked.clear();
        self.latency = 0.0;
    }
}

impl super::App {
    /// the rate pictures are presented at from the settings and the display:
    /// the frame rate asked for (0 follows the display), no more than the
    /// display shows with vsync on, nor than the cap.
    pub(crate) fn hfr_rate(&self) -> f64 {
        let pc = &self.settings.pc;
        let display = self
            .window
            .as_ref()
            .and_then(|window| window.current_monitor())
            .and_then(|monitor| monitor.refresh_rate_millihertz())
            .map_or(60.0, |mhz| mhz as f64 / 1000.0);
        let presents = self.backend.as_ref().is_some_and(|backend| !matches!(backend, crate::present::Backend::None));
        let mut rate = paced_rate(pc.frame_rate, pc.vsync, pc.fps_cap, vrr_on(pc), display, presents);
        if let Ok(forced) = std::env::var("ZAKURO_HFR_RATE") {
            rate = forced.parse().unwrap_or(rate);
        }
        rate.max(1.0)
    }

    /// whether the frame rate loop runs: whenever a game does. at the
    /// console's rate it presents each picture once, on the display's
    /// schedule rather than skipping pictures while it waits for the
    /// display; above it, with frame interpolation, see hfr_interpolates.
    pub(crate) fn hfr_wanted(&self) -> bool {
        self.game.is_some() && std::env::var_os("ZAKURO_HFR_OFF").is_none()
    }

    /// whether frame interpolation draws pictures in between: presenting
    /// faster than the console's rate, or the old 30 to 60 option.
    pub(crate) fn hfr_interpolates(&self) -> bool {
        (self.settings.pc.frame_rate != 30 && self.hfr_rate() > 61.0) || self.active.frame_interpolation || self.hfr_half_only()
    }

    /// hfr60: whether pictures in between are drawn for the 30 FPS scenes
    /// alone: presenting at about 60 in a window, with Smooth 30 FPS Scenes
    /// on. A 30 Hz scene (gap 2) then gets one picture halfway
    /// (fractions(60, 2) = [0.5]), a 60 Hz one none.
    fn hfr_half_only(&self) -> bool {
        let pc = &self.settings.pc;
        self.window.is_some() && pc.frame_rate != 30 && !self.active.frame_interpolation && console_rate(self.hfr_rate()) && smooth_30fps(pc)
    }

    /// hfr60: whether the pictures shown now are interpolated: the frame
    /// rate's interpolation on, the emulation keeping up, and at 60 a 30 FPS
    /// scene running.
    fn hfr_smoothing(&self) -> bool {
        self.hfr.interpolating && !self.hfr.degraded && (!self.hfr.half_only || self.hfr.slow_scene)
    }

    /// hfr60: at 60, the renderer matches draws (frame interpolation on the
    /// GPU) only while a 30 FPS scene runs and the emulation keeps up, so a
    /// 60 FPS scene costs exactly what it did without interpolation. Told
    /// only when it changes. Leaving, the scale pictures in between were
    /// drawn at goes back to the renderer's own (full), so the frames that
    /// follow are drawn as without interpolation.
    fn hfr_between_sync(&mut self) {
        if !self.hfr.half_only {
            self.hfr.gpu_between = None;
            return;
        }
        let on = self.hfr.slow_scene && !self.hfr.degraded;
        if self.hfr.gpu_between == Some(on) {
            return;
        }
        self.hfr.gpu_between = Some(on);
        self.hfr.clock.clear();
        self.hfr.slow_halves = 0;
        let Some(game) = &mut self.game else { return };
        let drawn = game.system.set_interpolation(on);
        if !on {
            zakuro_core::native_gfx::set_between_fraction(0.0);
            zakuro_core::native_gfx::set_between_samples(0);
        }
        zakuro_gpu::smooth::point("hfr60", Instant::now(), on as i64, game.frame as f64);
        log::debug!("hfr60: pictures in between {} (frame {})", if drawn { "on, a 30 FPS scene" } else { "off" }, game.frame);
    }

    /// puts the frame rate settings to use, called from apply_settings and
    /// when a game starts.
    pub(crate) fn hfr_apply(&mut self) {
        apply_game_timing(&self.settings.pc);
        let wanted = self.hfr_wanted();
        let rate = if wanted { self.hfr_rate() } else { 0.0 };
        let interpolate = wanted && self.hfr_interpolates();
        // hfr60: at 60, for the 30 FPS scenes alone (hfr_between_sync)
        let half_only = interpolate && self.hfr_half_only();
        // smooth: the monitor rate, for the present thread grid
        let display_hz = self.window.as_ref().and_then(|w| w.current_monitor()).and_then(|m| m.refresh_rate_millihertz()).map_or(60.0, |mhz| mhz as f64 / 1000.0);
        // smooth2: with VSync off the present thread paces at the rate asked
        // for (uncapped: UNCAPPED) on a grid of its own
        let vsync = self.settings.pc.vsync;
        // vrrcap: with variable refresh, on the rate's own steady grid with
        // VSync on too, never on the display's refresh grid (see vrr_limit)
        let vrr = vrr_on(&self.settings.pc);
        if let Some(backend) = &mut self.backend {
            backend.set_refresh_hint(display_hz);
            backend.set_vrr(vrr);
            backend.set_free_rate(((!vsync || vrr) && wanted).then_some(rate));
        }
        if vrr != self.hfr.vrr_logged {
            self.hfr.vrr_logged = vrr;
            log::info!("vrrcap: variable refresh {}", if vrr { format!("on: at most {:.1} pictures a second, each on a steady clock of its own", vrr_limit(display_hz)) } else { "off".to_owned() });
        }
        self.hfr.free_rate = 0.0;
        if let Some(game) = &mut self.game {
            game.system.track_pictures(wanted);
            // hfr60: at 60 the GPU interpolates only in 30 FPS scenes, see
            // hfr_between_sync below
            if game.system.gpu.interpolates() != interpolate && !half_only {
                let on = game.system.set_interpolation(interpolate);
                log::info!("hfr: frame interpolation {}", if on { "on" } else { "off, it takes the 3D drawn on the GPU and shown with Vulkan" });
            }
            if !interpolate {
                game.system.set_between(&[]);
            }
            // hfr60: the scale pictures in between were drawn at is the
            // renderer's own again once none are drawn (a change from a
            // higher rate to 60 or below kept it, and the frames with it)
            if !interpolate && zakuro_core::native_gfx::between_fraction().is_some() {
                zakuro_core::native_gfx::set_between_fraction(0.0);
                zakuro_core::native_gfx::set_between_samples(0);
            }
        }
        if half_only != self.hfr.half_only {
            log::info!("hfr60: {}", if half_only { "at 60 FPS, the 30 FPS scenes get a picture in between each two of theirs" } else { "the 30 FPS scenes as the other scenes" });
        }
        self.hfr.interpolating = interpolate;
        self.hfr.half_only = half_only;
        // hfr60: told again (from a higher rate the GPU interpolates: off
        // until a 30 FPS scene runs)
        self.hfr.gpu_between = None;
        self.hfr_between_sync();
        if rate != self.hfr.rate {
            log::info!("hfr: presenting at {rate:.1} pictures a second{}", if rate == 0.0 { " (the console's rate)" } else { "" });
            self.hfr.rate = rate;
            self.hfr.reset();
        }
    }

    /// one pass of the loop at a high frame rate: the game's refresh when it
    /// is due, then the picture when it is due, then a wait for whichever
    /// comes first.
    pub(crate) fn hfr_step(&mut self, event_loop: &winit::event_loop::ActiveEventLoop) {
        if self.stop {
            event_loop.exit();
            return;
        }
        let period = refresh();
        let present_period = Duration::from_secs_f64(1.0 / self.hfr.rate.max(1.0));
        let now = Instant::now();
        // smooth: time the event loop spent elsewhere (window events), and
        // the loop's own errands, in the timing log when they take long
        // rt-merge: in real time, the frontend's own stalls (window events,
        // errands, the title) count in the cover the pictures ahead are sized
        // for, as a lend's overrun does; deterministic mode's emulation cost
        // covers them there
        let realtime = zakuro_core::pcos::rt::realtime();
        if let Some(end) = self.hfr.pass_end.take() {
            if now - end > Duration::from_millis(2) {
                zakuro_gpu::smooth::span("outside", end, now, 0, 0.0);
            }
            if realtime {
                self.hfr.held = self.hfr.held.max(now - end);
            }
        }
        if now > self.next_frame + super::CATCH_UP_LIMIT {
            self.next_frame = now;
            self.hfr.reset();
        }
        // smooth: each errand that takes long goes to the timing log
        // (errand, a = 0 library, 1 jobs, 2 pads, 3 room and download play,
        // 4 bottom screen, 5 the check script)
        let mut errand = Instant::now();
        let mut timed = |which: i64| {
            let now = Instant::now();
            if now - errand > Duration::from_millis(2) {
                zakuro_gpu::smooth::span("errand", errand, now, which, 0.0);
            }
            errand = now;
        };
        self.library.poll();
        timed(0);
        self.poll_jobs();
        timed(1);
        self.poll_gamepads();
        self.qol_poll(); // qol: the Game Speed key, the speed, the boon/bane panel
        self.states_poll(); // pulse: save states
        timed(2);
        if self.room.as_ref().is_some_and(|room| !room.open()) {
            self.leave_room();
            self.menus.message = Some("The room closed, or the connection to it was lost.".into());
        }
        self.download_play();
        timed(3);
        // wide: the bottom panel's polling, its scripted checks and the
        // mouse's touch, as App::step has them, which this loop skipped
        let composes = self.composes();
        self.bottom.preview = crate::fates_ui::enabled() && self.fates.options.previewing_bottom(); // bottom2
        self.bottom.poll(&self.active, composes);
        timed(4);
        self.bottom_check_step();
        timed(5);
        if self.mouse_down {
            self.touch();
        }
        if now.elapsed() > Duration::from_millis(1) {
            zakuro_gpu::smooth::since("errands", now, 0, 0.0);
        }
        if realtime {
            self.hfr.held = self.hfr.held.max(now.elapsed());
        }

        let typing = self.game.as_ref().is_some_and(|game| game.system.keyboard_request().is_some());
        let playing = self.game.is_some() && !self.paused && !self.menus.over_game() && !typing && !crate::pulse::holds(); // pulse: the rewind screen stops it
        // pace: why the game counts as stopped, in the timing log (a: 1
        // paused, 2 a menu over it, 4 typing, 8 Divine Pulse's screen)
        if !playing && self.game.is_some() && zakuro_gpu::smooth::enabled() {
            let why = self.paused as i64 | (self.menus.over_game() as i64) << 1 | (typing as i64) << 2 | (crate::pulse::holds() as i64) << 3;
            zakuro_gpu::smooth::point("stopped", now, why, 0.0);
        }
        if !playing {
            if let Some(audio) = &self.audio {
                audio.hold();
            }
        }

        // the game's refreshes, on the console's schedule
        let mut ran = 0;
        // behind, a refresh at a time with a picture between them, so the
        // screen keeps moving while the game catches up
        let picture_due = |pacer: &Pacer| pacer.next_present.is_some_and(|due| Instant::now() >= due);
        // smooth: with the present thread, its own loop (see paced_pass)
        if let Some(wake) = self.paced_pass(event_loop, playing) {
            let titled = Instant::now();
            self.hfr_title(playing);
            if titled.elapsed() > Duration::from_millis(1) {
                zakuro_gpu::smooth::since("title", titled, 0, 0.0);
            }
            if realtime {
                self.hfr.held = self.hfr.held.max(titled.elapsed());
            }
            let waited_from = Instant::now();
            // rt-merge: in real time the game's threads run in the wait
            // (native-rt); a lend ends when its thread next looks at the
            // clock, which can be late (a long native call)
            let lent = self.hfr_lend(wake, playing, None);
            // the game's frame done early (game_first): record at once
            let done = self.hfr.game_first && self.game.as_ref().is_some_and(|game| game.system.rt_settled());
            if !done {
                precise_sleep_until(wake);
            }
            // late from the wait's own start: time the title took is not a
            // late wake
            let late = Instant::now().saturating_duration_since(wake.max(waited_from));
            if lent && late > Duration::from_micros(500) {
                zakuro_gpu::smooth::span("lendover", wake, wake + late, 0, 0.0);
                self.hfr.held = self.hfr.held.max(late);
            } else if late > Duration::from_millis(1) {
                zakuro_gpu::smooth::span("oversleep", wake, wake + late, 0, 0.0);
            }
            // late by most of a refresh of the display: the core is taken.
            // a lend's overrun is the game's own work, not that: a busy
            // core shows there in the hand-offs' waits (os2's escape)
            if !lent && late > Duration::from_millis(4) {
                zakuro_core::pcos::threads::report_late();
            }
            self.hfr.pass_end = Some(Instant::now());
            return;
        }
        while playing && Instant::now() >= self.next_frame && ran < 4 && !(ran > 0 && picture_due(&self.hfr)) {
            self.hfr_refresh();
            ran += 1;
        }
        if !playing {
            self.next_frame = now + period;
        }

        // the pictures, when their time comes. a present that waits for the
        // display (FIFO) is queued to be shown at a refresh to come, so the
        // pictures of the newest frame's time can be handed over ahead,
        // before the next refresh of the game takes the thread; each shows
        // the point of the game's time its refresh of the display will
        let fifo = self.backend.as_ref().is_some_and(|backend| backend.waits_for_display());
        let ahead = if fifo { present_period * 2 } else { Duration::ZERO };
        for _ in 0..3 {
            let now = Instant::now();
            let slot = self.hfr.next_present.map_or(now, |slot| slot.max(now));
            if slot > now + ahead {
                break;
            }
            // without pictures in between, each of the game's pictures once,
            // as soon as it is made and the rate (a cap) allows
            let interpolating = self.hfr_smoothing(); // hfr60: at 60, 30 FPS scenes alone
            if !interpolating && playing {
                let newest = self.hfr.latched.map(|(_, refresh, _)| refresh);
                if newest.is_none() || newest == self.hfr.shown_refresh || slot > now {
                    break;
                }
                self.hfr.shown_refresh = newest;
            }
            let mut pick = Pick::Newest;
            // smooth: the point of the game's time this present shows, in
            // refreshes, for the timing log
            let mut content = self.hfr.latched.map_or(0.0, |(_, refresh, _)| refresh as f64);
            if let (Some(game), Some((scheduled, _, gap)), true) = (&mut self.game, self.hfr.latched, playing && interpolating) {
                // where between the picture before and the newest the
                // slot's time falls, the newest's time starting when it is
                // made, a smoothed emulation's time after its refresh began
                // (hfr60: at 60 too: so the slot showing the halfway picture
                // comes at least half a refresh after the newest is made,
                // which the GPU needs to draw it at a high render scale. Half
                // a refresh earlier, a third of them reached the display a
                // refresh or two late at 8x; measured in hfr60.md)
                let start = scheduled + self.hfr.emulating;
                let span = period * gap.max(1) as u32;
                let at = slot.saturating_duration_since(start).as_secs_f32() / span.as_secs_f32();
                // not made yet: it waits for the game's next refresh
                // (hfr60: at 60 up to a quarter past it the newest is the
                // nearest picture, and shows)
                let made_yet = if self.hfr.half_only { 1.25 } else { 1.02 };
                if at > made_yet && slot > now {
                    break;
                }
                let ready = game.system.between_ready();
                // the picture before only stands for the start of an
                // interpolated time, else the newest shows at once
                pick = nearest(ready, at.clamp(0.0, 1.0), !ready.is_empty());
                // castlecam: the native renderer's picture at the slot's own time
                // (hfr60: at 60, the halfway picture or a frame, the nearest)
                if let Some(exact) = exact_place(zakuro_core::native_gfx::active() && !self.hfr.half_only, ready, at as f64) {
                    pick = Pick::At(exact);
                }
                let gap_f = gap.max(1) as f64;
                content -= gap_f * (1.0 - match pick {
                    Pick::Previous => 0.0,
                    Pick::Between(layer) => ready.get(layer - 1).copied().unwrap_or(1.0) as f64,
                    Pick::Newest => 1.0,
                    Pick::Before(f) => f as f64 - 1.0, // gfx-models: not picked here
                    Pick::At(f) => f as f64,
                });
                match pick {
                    Pick::Previous => game.system.show_previous(),
                    Pick::Between(layer) => game.system.show_layer(layer),
                    Pick::Newest => game.system.show_layer(0),
                    Pick::Before(f) => game.system.show_before(f),
                    Pick::At(f) => game.system.show_at(f),
                }
            }
            // hfr60: at 60 below the display's rate (Frame Rate 60 on a
            // faster display), pictures recorded ahead go out at their own
            // slots on the present thread, not at once: handed over at once,
            // the frame before, the halfway picture and the frame were shown
            // in three refreshes in a row, then none for the rest of the
            // 30 Hz frame's time
            if self.hfr.half_only && interpolating && playing {
                if let Some(backend) = &mut self.backend {
                    if backend.pacing().is_some() {
                        backend.set_present_at(Some(slot));
                    }
                }
            }
            let start = Instant::now();
            self.present(event_loop);
            if let Some(game) = &mut self.game {
                game.system.show_layer(0);
                if let Some(clock) = &mut game.clock {
                    clock.present();
                }
            }
            self.spent.showing.add(start.elapsed());
            zakuro_gpu::smooth::since("present", start, self.hfr.presented as i64 + 1, content);
            self.hfr.count_distinct(content);
            self.shown += 1;
            self.hfr.presented += 1;
            self.hfr.in_between += matches!(pick, Pick::Between(_) | Pick::Before(_) | Pick::At(_)) as u64;
            self.hfr.next_present = Some(slot + present_period);
        }

        self.hfr_title(playing);

        // wait for whichever is due first
        let picture = self.hfr.next_present.map(|slot| slot.checked_sub(ahead).unwrap_or(slot));
        let wake = match playing {
            true => self.next_frame.min(picture.unwrap_or(self.next_frame)),
            false => picture.unwrap_or(self.next_frame),
        };
        // native-rt: in real time the game's threads run in the wait; a
        // wake already past (the picture waits for the game's next refresh)
        // still lends them up to a present period, or nothing would run them
        self.hfr_lend(wake, playing, Some(present_period));
        precise_sleep_until(wake);
    }

    /// rtpace: real time: starts drawing the upper screen's picture in
    /// between the present to come shows (its pick is made) on the native
    /// renderer's drawing thread, and lends the game's threads the time
    /// until it is drawn, at most until the slot less RTPACE_HANDOVER. The
    /// present then finds it drawn. Before, the frontend's thread drew it
    /// (5-5.7 ms at 8x with MSAA 8, recording and waiting for the GPU),
    /// presents went back to back, and the game's 30 Hz tick ran late (a
    /// picture shown twice). FATES_RT_DRAW_THREAD=0 draws it as before.
    /// The time this took on the frontend's thread (the present's cost).
    fn rtpace_draw_ahead(&mut self, slot: Instant) -> Duration {
        let started = Instant::now();
        // rtpace: the present thread off the game's core too
        static ANYWHERE: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*ANYWHERE.get_or_init(|| std::env::var("FATES_RT_DRAW_ANYWHERE").is_ok_and(|v| v == "1")) {
            zakuro_gpu::backend::keep_present_thread_off(zakuro_core::pcos::threads::game_core_mask());
        }
        let Some(game) = &mut self.game else { return Duration::ZERO };
        if !zakuro_core::native_gfx::begin_between(&mut game.system) {
            return started.elapsed();
        }
        let begun = Instant::now();
        let deadline = slot.checked_sub(RTPACE_HANDOVER).unwrap_or(begun).clamp(begun, begun + Duration::from_millis(12));
        game.system.lend_while(deadline, &zakuro_core::native_gfx::BETWEEN_BUSY);
        let now = Instant::now();
        zakuro_gpu::smooth::span("drawahead", started, now, zakuro_core::native_gfx::between_busy() as i64, (begun - started).as_secs_f64() * 1e3);
        now - started
    }

    /// rt-merge: in real time, before a refresh is raised: when the
    /// frontend's own work (pictures that took long) left the game's threads
    /// too little of the time since the last refresh to finish their frame,
    /// they get up to SETTLE more, until they are done. The refresh is then
    /// raised a little late, and the game makes its picture, rather than
    /// missing the refresh and losing its time (the game's clock counts a
    /// missed refresh as no time). The schedule stays (next_frame moves on
    /// by the period), so the refreshes keep the console's rate, and the
    /// lateness counts in the cover the pictures ahead are sized for.
    fn hfr_settle(&mut self) {
        const SETTLE: Duration = Duration::from_millis(8);
        const SETTLE_LATE: Duration = Duration::from_millis(3);
        if !zakuro_core::pcos::rt::realtime() {
            return;
        }
        // a refresh raised late (the frontend behind) still gets its frame a
        // little time: raised with none, it is missed for certain
        let limit = (self.next_frame + SETTLE).max(Instant::now() + SETTLE_LATE);
        let Some(game) = &mut self.game else { return };
        if !game.system.rt_settled() {
            let start = Instant::now();
            game.system.lend_until_settled(limit);
            zakuro_gpu::smooth::since("settle", start, 0, 0.0);
        }
    }

    /// rt-merge: in real time, lends the game's threads the frontend's wait
    /// until wake (System::lend_until; nothing in deterministic mode, where
    /// the refresh runs them). A wake already past lends nothing, unless
    /// `past` asks for up to that long, no further than the next refresh.
    /// Says whether time was lent.
    fn hfr_lend(&mut self, wake: Instant, playing: bool, past: Option<Duration>) -> bool {
        if !playing || !zakuro_core::pcos::rt::realtime() {
            return false;
        }
        let Some(game) = &mut self.game else { return false };
        let now = Instant::now();
        let until = match past {
            _ if wake > now => wake,
            Some(period) => self.next_frame.min(now + period).max(now),
            None => return false,
        };
        if until <= now {
            return false;
        }
        // rt-merge: a pass that ended for the game's frame (game_first)
        // takes the time back as soon as the frame is done
        if self.hfr.game_first && past.is_none() {
            game.system.lend_until_settled(until);
            return true;
        }
        game.system.lend_until(until);
        true
    }

    /// the title, and the switch to no pictures in between while the
    /// emulation falls behind, every half second.
    fn hfr_title(&mut self, playing: bool) {
        if self.last_title_update.elapsed() >= Duration::from_millis(500) {
            // sharp: whether the half second just past went without pictures
            // in between (see the scale's steps below)
            let was_degraded = self.hfr.degraded;
            // falling behind the console's rate: no pictures in between
            // until the emulation keeps up for a while
            if let (Some(game), true) = (&self.game, playing && self.hfr.interpolating) {
                let behind = game.fps < 57.5;
                (self.hfr.falling_behind, self.hfr.keeping_up) = if behind { (self.hfr.falling_behind + 1, 0) } else { (0, self.hfr.keeping_up + 1) };
                // hfr60: at 60, a 30 FPS scene's pictures in between can slow
                // the game a little on a PC that keeps up without them (its
                // refreshes still above 57.5 a second, or in real time its
                // ticks late): its clock running more than 1.5% slow for 1.5 s
                // stops them too (measured: a PC at 14 ms a refresh ran at
                // 0.97 of the game's speed with them, 0.997 without)
                let rate = self.hfr.clock_rate();
                let slow = self.hfr.half_only && self.hfr.gpu_between == Some(true) && rate.is_some_and(|r| r < CLOCK_SLOW);
                self.hfr.slow_halves = if slow { self.hfr.slow_halves + 1 } else { 0 };
                if slow {
                    self.hfr.keeping_up = 0;
                }
                // gfxcastle: dev, FATES_GFX_SEQ's captures stall the game on purpose
                if !self.hfr.degraded && (self.hfr.falling_behind >= 3 || self.hfr.slow_halves >= 3) && std::env::var_os("FATES_GFX_SEQ").is_none() {
                    self.hfr.degraded = true;
                    // hfr60: they slowed the game again soon after they
                    // started again: twice as long before the next try (up
                    // to a minute), so a PC at the edge loses next to no time
                    if self.hfr.half_only {
                        let again = self.hfr.recovered_at.is_some_and(|t| t.elapsed() < Duration::from_secs(20));
                        self.hfr.recover_need = if again { (self.hfr.recover_need.max(2) * 2).min(120) } else { 2 };
                    }
                    match self.hfr.slow_halves >= 3 {
                        true => log::warn!("hfr60: the game runs {:.1}% slow with pictures in between, they stop for {:.0} s or more", 100.0 * (1.0 - rate.unwrap_or(0.0) / 59.83), self.hfr.recover_need.max(2) as f32 / 2.0),
                        false => log::warn!("hfr: the emulation falls behind ({:.1} refreshes a second), pictures in between stop until it keeps up", game.fps),
                    }
                // smooth2: back after a second of keeping up (was five): a
                // battle's loading made the game fall behind, and its whole
                // opening then went without pictures in between
                } else if self.hfr.degraded && self.hfr.keeping_up >= if self.hfr.half_only { self.hfr.recover_need.max(2) } else { 2 } {
                    self.hfr.degraded = false;
                    self.hfr.recovered_at = Some(Instant::now());
                    log::info!("hfr: the emulation keeps up again, pictures in between start again");
                }
            }
            // hfr60: at 60 the switch above stops the renderer's matching too
            self.hfr_between_sync();
            // smooth2: pictures in between at a smaller scale while the
            // display goes without pictures (the frontend cannot record them
            // in time), back up after four calm seconds
            // hfr60: at 60, only while a 30 FPS scene's pictures in between are drawn
            if playing && self.hfr.interpolating && (!self.hfr.half_only || self.hfr.gpu_between == Some(true)) && zakuro_core::native_gfx::active() && std::env::var_os("FATES_GFX_HFR_SCALE").is_none() {
                let mut starved = std::mem::take(&mut self.hfr.starved);
                let step = self.hfr.scale_step;
                // sharp: and only when the display got fewer than nine in
                // ten of the new pictures it could show while the game kept
                // up: a scene's load (the game itself behind) and the odd
                // late picture no longer take the scale down; with every
                // picture now following the scale, each step shows
                let elapsed = self.last_title_update.elapsed().as_secs_f32().max(1e-3);
                let new_pictures = self.hfr.distinct as f32 / elapsed;
                let display = self.window.as_ref().and_then(|w| w.current_monitor()).and_then(|m| m.refresh_rate_millihertz()).map_or(60.0, |mhz| mhz as f32 / 1000.0);
                let can_show = display.min(self.hfr_rate() as f32);
                // (and pictures in between were being drawn: right after a
                // load they are off for a second, the display then gets only
                // the game's 60 a second)
                let kept_up = self.game.as_ref().is_some_and(|g| g.fps >= 57.5) && !self.hfr.degraded && !was_degraded;
                zakuro_gpu::smooth::point("hfrstarved", Instant::now(), starved as i64, new_pictures as f64);
                if sharp_steps() && !(kept_up && new_pictures < 0.9 * can_show) {
                    starved = 0;
                }
                // gfxcastle: the frames are shown at this scale too now (fates-gfx
                // hfr.rs reduced_frame), so a change of it shows over the whole
                // picture: it changes rarely. Down after two of the last three
                // half seconds went without pictures (or one went badly), not
                // after any one hitch (a scene loading); back up after eight
                // calm seconds, twice as long each time a step up had to come
                // back down within twenty seconds (up to a minute), so a scene
                // at the edge settles at the lower scale rather than
                // alternating sharp and soft every few seconds.
                let bad = |n: u32| n > 2;
                self.hfr.starved_history = (self.hfr.starved_history << 8) | starved.min(255);
                let recent = [self.hfr.starved_history & 0xFF, (self.hfr.starved_history >> 8) & 0xFF, (self.hfr.starved_history >> 16) & 0xFF];
                let down = starved > 12 || (bad(recent[0]) && recent.iter().filter(|&&n| bad(n)).count() >= 2);
                if self.hfr.calm_needed == 0 {
                    self.hfr.calm_needed = SCALE_CALM;
                }
                // perf3: with Anti-Aliasing above 4x, one step past the
                // scales: pictures in between (and the frames shown at their
                // scale) at 4 samples, the next thing to give when the scale
                // is at its floor. Not lower: the display also goes without
                // pictures when the game's thread is late, which fewer
                // samples do not help, and 4x at the display's own pixels
                // looks almost as 8x
                let steps = BETWEEN_SCALES.len() + if zakuro_core::native_gfx::msaa() > 4 { 1 } else { 0 };
                if down && step + 1 < steps {
                    self.hfr.scale_step += 1;
                    self.hfr.calm = 0;
                    self.hfr.starved_history = 0;
                    if self.hfr.stepped_up.is_some_and(|t| t.elapsed() < Duration::from_secs(20)) {
                        self.hfr.calm_needed = (self.hfr.calm_needed * 2).min(SCALE_CALM_MOST);
                    }
                } else if starved == 0 {
                    self.hfr.calm += 1;
                    if self.hfr.calm >= self.hfr.calm_needed && step > 0 {
                        self.hfr.scale_step -= 1;
                        self.hfr.calm = 0;
                        self.hfr.stepped_up = Some(Instant::now());
                    }
                    // two calm minutes since the last step up: patient again
                    if self.hfr.stepped_up.is_some_and(|t| t.elapsed() > Duration::from_secs(120)) {
                        self.hfr.calm_needed = SCALE_CALM;
                    }
                } else {
                    self.hfr.calm = 0;
                }
                if self.hfr.scale_step != step {
                    let scale = BETWEEN_SCALES[self.hfr.scale_step.min(BETWEEN_SCALES.len() - 1)];
                    zakuro_gpu::smooth::point("hfrscale", Instant::now(), self.hfr.scale_step as i64, scale as f64);
                    log::debug!("smooth2: pictures in between at {scale:.2} of the scale (step {})", self.hfr.scale_step);
                }
                // gfxcastle: never below the display's own pixels: the top
                // screen is at most the window's height, so a picture with at
                // least that many rows is not magnified. At the user's 8x in a
                // 1440-row window that is 3/4 (1920 rows drawn, 1440 shown),
                // where smooth2's 1/2 was magnified 1.5x and soft
                let floor = match (&self.game, &self.window) {
                    (Some(game), Some(window)) => {
                        let rows = 240.0 * game.system.gpu.scale().max(1) as f32;
                        (window.inner_size().height as f32 / rows).clamp(0.0, 1.0)
                    }
                    _ => 0.0,
                };
                let scale = BETWEEN_SCALES[self.hfr.scale_step.min(BETWEEN_SCALES.len() - 1)];
                // rtpace: and never above them: a picture in between is on
                // screen for one refresh, and drawn larger than the window
                // shows it (the user's 8x is 1920 rows in a 1440-row window)
                // it only cost GPU time. With the game's ticks on time the
                // pacing saw no starved display and stepped up to full scale,
                // and the GPU (750 ms of each second near a wall) then made
                // the compositor miss refreshes, 5-8 a second, and each step
                // up or down remade the targets (a 60 ms stall).
                // FATES_RT_SCALE_CAP=0 as before. sharp: with the cap the
                // scale is the floor whenever the window is smaller than the
                // render, whatever the step, and the frames with nothing
                // moving are shown at it too (fates-gfx hfr.rs reduced_frame):
                // one scale for every picture. Full scale everywhere was
                // measured with mapjit's pacing and still overloads the GPU
                // (battle: 30 refreshes a second without a new picture
                // against 2; castle by a wall: 4 against 0.7; sharp.md)
                // hfr60: not at 60, where a 30 FPS scene's pictures in between
                // are 30 a second: capped, every battle's frames were drawn
                // at the window's own pixels (the user's 8x at 6x), and the
                // renderer's targets were made again as each battle started
                // and ended (a hitch each)
                let scale = match rtpace_switch(&SCALE_CAP) && floor > 0.0 && !self.hfr.half_only {
                    true => scale.min(floor),
                    false => scale,
                };
                zakuro_core::native_gfx::set_between_fraction(scale.max(floor));
                // perf3: a step the floor (or the scales' end) leaves without
                // effect draws the pictures in between at 4 samples at most:
                // at the user's 8x scale in a 1440-row window the floor is 3/4,
                // so the step to 1/2 drew exactly as 3/4 and the GPU, with 8
                // samples a picture, stayed behind
                let unused = (0..=self.hfr.scale_step)
                    .filter(|&k| k >= BETWEEN_SCALES.len() || (k > 0 && BETWEEN_SCALES[k] < floor - 1e-3))
                    .count() as u32;
                let cap = if unused == 0 { 0 } else { 4 };
                zakuro_core::native_gfx::set_between_samples(cap);
            }
            let shown = self.shown as f32 / self.last_title_update.elapsed().as_secs_f32();
            // smooth2: the corner's FPS: new pictures a second, as the
            // display gets them (not the game's 60 refreshes a second)
            self.hfr.shown_rate = Some(std::mem::take(&mut self.hfr.distinct) as f32 / self.last_title_update.elapsed().as_secs_f32());
            self.shown = 0;
            self.last_title_update = Instant::now();
            // native-rt: the sound's underruns, as App::step reports them
            let underruns = self.audio.as_ref().map_or(0, |audio| audio.take_underruns());
            if underruns > 0 && playing {
                log::warn!("the sound ran dry {underruns} times, the emulation is falling behind");
            }
            // pace: the game's sound against the wall clock, and the stretching
            if let Some((wall, made, stretch)) = self.audio.as_ref().and_then(|audio| audio.take_flow()) {
                let speed = made / wall;
                if playing && (speed < 0.985 || stretch > 1.01) {
                    log::warn!("pace: the game made {made:.2} s of sound in {wall:.2} s ({speed:.3}x real time), stretched {:.1}% to fill", (stretch - 1.0) * 100.0);
                } else {
                    log::info!("pace: the game made {made:.2} s of sound in {wall:.2} s ({speed:.3}x real time), stretched {:.1}%", (stretch - 1.0) * 100.0);
                }
            }
            if let Some(game) = &self.game {
                let super::Spent { emulating, showing } = std::mem::take(&mut self.spent);
                log::debug!(target: "zakuro::fps", "{:.1} frames emulated and {shown:.1} shown a second, emulating took {emulating}, showing {showing}", game.fps);
            }
            if let Some(window) = &self.window {
                let title = match &self.game {
                    // port: the window says the project's name, Haiku, nothing else
                    _ if crate::fates_ui::enabled() => "Haiku".to_owned(),
                    Some(game) => format!("Zakuro - {} - {} | {shown:.0} fps", game.name, game.system.status_line()),
                    None => "Zakuro".to_owned(),
                };
                window.set_title(&title);
            }
        }
    }

    /// emulates the refresh that is due, and takes note of its picture.
    fn hfr_refresh(&mut self) {
        let period = refresh();
        let scheduled = self.next_frame;
        let start = Instant::now();
        self.emulate();
        let took = start.elapsed();
        self.spent.emulating.add(took);
        // hfr60: at 60, whether the game runs a 30 FPS scene now (GameTime's
        // step: 2 in battles, 3D cutscenes, the title screen), which alone
        // gets pictures in between
        if self.hfr.half_only {
            if let Some(game) = &mut self.game {
                let time = GameTime::read(&mut game.system.memory);
                self.hfr.slow_scene = time.step >= 2;
                // and how fast its clock runs while pictures in between are drawn
                if self.hfr.gpu_between == Some(true) {
                    self.hfr.clock_moved(Instant::now(), time.total);
                }
            }
            self.hfr_between_sync();
        }
        // rt-merge: in real time the refresh itself takes microseconds; what
        // the pictures ahead and the latency must cover is how long after
        // its due time the refresh's picture is there (raised late after a
        // lend that overran), and the longest any lend kept the frontend
        let cover = match zakuro_core::pcos::rt::realtime() {
            true => (start + took).saturating_duration_since(scheduled).max(std::mem::take(&mut self.hfr.held)).min(period * 2),
            false => took,
        };
        // rt-merge: in real time, the refresh in the timing log: a = 1 when
        // absorbed, + 2 when a game thread was still busy; b = the time lent
        // since the refresh before (ms), and in `rtidle` the time of it the
        // threads slept with everything blocked
        if zakuro_gpu::smooth::enabled() && zakuro_core::pcos::rt::realtime() {
            // rtpace: dev, the refresh's due time (a = the refresh's frame,
            // b = how late it was raised, ms), and the game's tick counter
            // for the `gtick` probe
            zakuro_core::pcos::rtpace::set_probe(zakuro_core::region::us(GAME_TIME));
            if let Some(game) = &self.game {
                zakuro_gpu::smooth::point("rtdue", scheduled, game.frame as i64, start.saturating_duration_since(scheduled).as_secs_f64() * 1e3);
            }
            let (absorbed, busy, lent, idle) = zakuro_core::pcos::rt::totals();
            let (a0, b0, l0, i0) = std::mem::replace(&mut self.hfr.rt_totals, (absorbed, busy, lent, idle));
            let flags = (absorbed > a0) as i64 + 2 * (busy > b0) as i64;
            zakuro_gpu::smooth::point("rtrefresh", start, flags, (lent - l0) as f64 / 1e6);
            zakuro_gpu::smooth::point("rtidle", start, 0, (idle - i0) as f64 / 1e6);
        }
        // smooth: the refresh in the timing log, with the game's clock
        if zakuro_gpu::smooth::enabled() {
            if let Some(game) = &mut self.game {
                let time = GameTime::read(&mut game.system.memory);
                zakuro_gpu::smooth::span("emu", start, start + took, game.frame as i64, time.total as f64);
            }
        }
        // towards the slow ones fast, back down slowly
        // gfx-models: the second slowest of the last 32 refreshes. One hitch
        // (a scene loading) cannot be covered by pictures ahead anyway; taken
        // as the cost it made the latency up to two refreshes longer for a
        // second or more, and slots showed the frame before (judder)
        self.hfr.covers.push_back(cover);
        if self.hfr.covers.len() > 32 {
            self.hfr.covers.pop_front();
        }
        self.hfr.emulating = match std::env::var_os("ZAKURO_PACE_COVER").is_some_and(|v| v == "old") {
            true => if cover > self.hfr.emulating { cover } else { (self.hfr.emulating * 31 + cover) / 32 },
            false => second_largest(&self.hfr.covers),
        };
        self.next_frame += period;
        let rate = self.hfr.rate;
        let dense = self.hfr.dense;
        let Some(game) = &mut self.game else { return };
        let (gap, at) = game.system.picture_gap();
        if self.hfr.latched.is_none_or(|(_, refresh, _)| refresh != at) {
            self.hfr.latched = Some((scheduled, at, gap));
            // mapjit: dev, the picture taken: its native frame (a) and the
            // refresh it came at (b), the numbers presents' game time is in
            if zakuro_gpu::smooth::enabled() {
                zakuro_gpu::smooth::point("latch", scheduled, game.system.native_last().map_or(-1, |id| id as i64), at as f64);
            }
            // smooth: the last second's or so
            self.hfr.gaps.push_back(gap);
            if self.hfr.gaps.len() > 48 {
                self.hfr.gaps.pop_front();
            }
            // the next picture's pictures in between, for its gap
            // (hfr60: at 60 the one halfway, fractions(60, 2), and the
            // nearest picture shown: one picture in between a 30 Hz frame)
            let wanted = if self.hfr.degraded {
                Vec::new()
            } else if dense && !self.hfr.half_only && gap <= 2 && crate::qol::speed() <= 1.0 {
                // (qol: above 1x the game makes its pictures faster than at
                // the console's rate, so the offer follows the rate below)
                // smooth: the native renderer draws a picture in between
                // only when it is shown, so it can be offered at many
                // places for the present thread's slots to pick from
                let steps = zakuro_gpu::Gpu::MAX_BETWEEN + 1;
                (1..steps).map(|k| k as f32 / steps as f32).collect()
            } else {
                fractions(rate, gap)
            };
            if wanted != self.hfr.asked {
                game.system.set_between(&wanted);
                self.hfr.asked = wanted;
            }
        }
    }

    /// smooth: one pass of the loop when presents go out on the present
    /// thread (zakuro-gpu backend vulkan_pacer), at slots on the display's
    /// refresh grid. Returns when to wake, none where that thread does not
    /// pace presents (another backend, vsync off, a frame rate below the
    /// display's), for the loop above.
    ///
    /// - Pictures are recorded ahead, one per slot, as many as cover the
    ///   emulation of a refresh (`depth`), so every refresh of the display
    ///   gets a new picture while the game's refresh is emulated.
    /// - Each slot shows the game's time `latency` before it. The pictures
    ///   are placed at the times their refreshes were due, so the time
    ///   shown moves exactly as the slots do, refresh after refresh: no
    ///   picture is shown early or late against its neighbours.
    /// - `latency` is the game's picture gap, the emulation's cost and one
    ///   display refresh: enough that the pictures for the slots up to the
    ///   end of the next emulation exist before it starts.
    fn paced_pass(&mut self, event_loop: &winit::event_loop::ActiveEventLoop, playing: bool) -> Option<Instant> {
        if std::env::var_os("FATES_PACED").is_some_and(|v| v == "0") {
            return None;
        }
        // smooth2: uncapped (VSync off, no rate asked for): pictures as
        // fast as the frontend can record them while leaving half its time
        // to the game (recorded back to back they starved it: the game ran
        // at 0.976 of its speed), never slower than the display
        if self.hfr.rate >= UNCAPPED && !self.settings.pc.vsync {
            let cost = self.hfr.present_mean.as_secs_f64().max(1e-4);
            let display_hz = self.window.as_ref().and_then(|w| w.current_monitor()).and_then(|m| m.refresh_rate_millihertz()).map_or(60.0, |mhz| mhz as f64 / 1000.0);
            let want = (0.5 / cost).clamp(display_hz, UNCAPPED);
            if self.hfr.free_rate == 0.0 || (want / self.hfr.free_rate - 1.0).abs() > 0.1 {
                self.hfr.free_rate = want;
                if let Some(backend) = &mut self.backend {
                    backend.set_free_rate(Some(want));
                }
            }
        }
        let (_, display, _, _) = self.backend.as_ref()?.pacing()?;
        // a frame rate below the display's keeps the old loop
        // (smooth2: with VSync off the grid is the rate's own)
        if self.hfr.rate < 0.95 / display.as_secs_f64() && self.settings.pc.vsync {
            return None;
        }
        let game_refresh = refresh().as_secs_f64();
        let interpolating = self.hfr_smoothing() && playing; // hfr60: at 60, 30 FPS scenes alone
        // hfr60: at 60 the halfway picture and the frames, the nearest
        // (exact places would draw two pictures in between a 30 Hz frame)
        self.hfr.dense = zakuro_core::native_gfx::active() && !self.hfr.half_only;
        // the emulation's cost as the pictures ahead must cover it: its
        // smoothed slow ones, not loading's (those hitch anyway)
        let emulating = self.hfr.emulating.as_secs_f64().min(2.0 * game_refresh);
        let margin = 0.0015;
        // hfr60: at 60 on a 60 Hz display, two pictures ahead cover 33 ms,
        // more than three at 175 Hz: a third only made the 30 FPS scenes'
        // pictures a refresh later (the 60 FPS ones, not interpolated, are
        // recorded as before)
        let fewest = if self.hfr.half_only && interpolating && display.as_secs_f64() > 0.012 { 2 } else { 3 };
        let depth = (((emulating + margin) / display.as_secs_f64()).ceil() as usize + 1).clamp(fewest, zakuro_gpu::backend::vulkan::MOST_AHEAD);
        // the largest of the recent pictures' gaps: a picture with a gap of
        // 1 can be followed by one 2 refreshes later, which the slots must
        // wait for as well
        // gfx-models: the game's cadence, not its hitches: the largest gap
        // that came at least twice among the last 16 pictures (a game
        // alternating gaps of 1 and 2 still gets 2). Before, the largest of
        // the last 48 was used, so one hitch (a scene loading, the level-up
        // screen opening) made the latency a refresh or two longer for the
        // next 1.6 s, and every slot whose time fell before the frame
        // before showed that frame: each frame held for half its time, then
        // the pictures in between hurried through the rest (judder)
        let gap = match std::env::var_os("ZAKURO_PACE_COVER").is_some_and(|v| v == "old") {
            true => self.hfr.gaps.iter().copied().max().unwrap_or(1).clamp(1, 4),
            false => cadence(&self.hfr.gaps),
        } as f64;
        if std::env::var_os("ZAKURO_PACE_TRACE").is_some() {
            static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
            eprintln!("pace: {:.3} cadence {gap} largest {} latency {:.1} ms emulating {:.1} ms", T0.get_or_init(Instant::now).elapsed().as_secs_f64(), self.hfr.gaps.iter().copied().max().unwrap_or(1), self.hfr.latency * 1e3, emulating * 1e3);
        }
        // hfr60: at 60 a 30 FPS scene's pictures are the frames and the
        // halfway ones, the nearest: a slot needs the newest frame once its
        // time is a quarter of the way to it, so the time shown can be half
        // a refresh sooner than a whole gap behind
        let gap = if self.hfr.half_only && interpolating { (gap - 0.5).max(1.0) } else { gap };
        let wanted = gap * game_refresh + emulating + display.as_secs_f64() + margin;
        // rtpace: in real time a refresh is raised on time and costs
        // nothing (emulating is how late it came), but pictures are still
        // recorded up to `depth` slots ahead: the latency covers that lead
        // too, or the slots ran past the newest frame's time while the next
        // was still to come and waited for the game (the display showed a
        // picture twice). FATES_RT_LEAD=0 as before
        let wanted = match zakuro_core::pcos::rt::realtime() && rtpace_switch(&LEAD) {
            true => wanted + (depth.saturating_sub(2)) as f64 * display.as_secs_f64(),
            false => wanted,
        };
        // the latency moves slowly, so the time shown does not jump
        // mapjit: by at most a share of the wall time since the last pass (the
        // time shown runs at most SLEW_UP slower or SLEW_DOWN faster than the
        // clock). Up to 0.2 ms a pass, with several passes a refresh, made one
        // refresh in six show a step 3-15% shorter or longer than the rest
        // while the map scrolled (measured). FATES_PACE_SLEW=0 as before
        let now_pass = Instant::now();
        let since = self.hfr.latency_at.map_or(0.0, |t| now_pass.saturating_duration_since(t).as_secs_f64()).min(0.05);
        self.hfr.latency_at = Some(now_pass);
        let (down, up) = match slews() {
            true => (-SLEW_DOWN * since, SLEW_UP * since),
            false => (-0.0002, 0.0002),
        };
        self.hfr.latency = if self.hfr.latency == 0.0 || (wanted - self.hfr.latency).abs() > 0.05 {
            wanted
        } else {
            self.hfr.latency + (wanted - self.hfr.latency).clamp(down, up)
        };

        // the pictures ahead
        let mut recorded = 0;
        let mut waiting_for_game = false;
        let realtime = zakuro_core::pcos::rt::realtime(); // rt-merge
        let mut game_first = None;
        let mut refresh_first = false; // rtpace
        while recorded < zakuro_gpu::backend::vulkan::MOST_AHEAD {
            let Some((slot, _, pending, _)) = self.backend.as_ref().and_then(|backend| backend.pacing()) else { break };
            if pending >= depth {
                break;
            }
            // rtpace: in real time raising a refresh takes microseconds, so
            // one that is due is raised before another picture is recorded
            // (while one still waits for the display): the game's ticks keep
            // the console's steady schedule. Recording a picture took about a
            // refresh of the display, so this loop ran on and refreshes were
            // raised 10-30 ms late (FATES_RT_RAISE_FIRST=0 as before)
            if realtime && playing && pending >= 1 && Instant::now() >= self.next_frame && rtpace_switch(&RAISE_FIRST) {
                break;
            }
            // rtpace: and a picture whose drawing would run past the refresh
            // due next waits for it (while two still wait for the display):
            // the refresh is raised on time and the drawing's lend runs the
            // tick it starts, rather than the refresh waiting for the drawing
            // (ticks then started up to 6 ms late)
            let now = Instant::now();
            if realtime && playing && pending >= 2 && rtpace_switch(&RAISE_FIRST) && self.next_frame > now && self.next_frame < now + self.hfr.draw_cost {
                refresh_first = true;
                break;
            }
            // smooth2: none waiting: the display shows the last again
            if pending == 0 && playing && self.hfr.presented > 0 {
                self.hfr.starved += 1;
            }
            // rt-merge: in real time, the game's frame first. While its
            // threads have not finished the frame since the last refresh and
            // a picture is still waiting for the display, the next picture
            // is recorded only when it must be (its slot less its cost), and
            // the time until then is lent to the game (until its threads are
            // done): pictures that take long to draw (in between, 5 ms or
            // more) recorded back to back would otherwise leave the game no
            // time, and it would miss the refresh, which loses game time.
            // Deterministic mode runs the game's frame first by construction.
            if realtime && playing && pending >= 1 && self.game.as_ref().is_some_and(|game| !game.system.rt_settled()) {
                if let Some(start_by) = slot.checked_sub(self.hfr.present_cost + GAME_FIRST_MARGIN) {
                    if start_by > Instant::now() + Duration::from_micros(500) {
                        game_first = Some(start_by);
                        break;
                    }
                }
            }
            let mut pick = Pick::Newest;
            let mut content = self.hfr.latched.map_or(0.0, |(_, refresh, _)| refresh as f64);
            if let (Some(game), Some((scheduled, _, gap)), true) = (&mut self.game, self.hfr.latched, interpolating) {
                // where between the picture before and the newest the
                // slot's game time falls, each picture at its refresh's time
                let span = gap.max(1) as f64 * game_refresh;
                let shows = slot.checked_sub(Duration::from_secs_f64(self.hfr.latency));
                let at = shows.map_or(0.0, |shows| {
                    let before = scheduled.checked_sub(Duration::from_secs_f64(span));
                    match before {
                        Some(before) if shows >= before => (shows - before).as_secs_f64() / span,
                        Some(before) => -((before - shows).as_secs_f64() / span),
                        None => 1.0,
                    }
                });
                // not made yet: wait for the game, unless the display would
                // go without a picture (hfr60: at 60 up to a quarter past
                // it the newest is the nearest picture, and shows)
                let made_yet = if self.hfr.half_only { 1.25 } else { 1.01 };
                if at > made_yet && pending > 0 {
                    zakuro_gpu::smooth::point("ahead", Instant::now(), pending as i64, at);
                    waiting_for_game = true;
                    break;
                }
                let ready = game.system.between_ready();
                pick = nearest(ready, at.clamp(0.0, 1.0) as f32, !ready.is_empty());
                // castlecam: the native renderer's picture at the slot's own time
                if let Some(exact) = exact_place(self.hfr.dense, ready, at) {
                    pick = Pick::At(exact);
                }
                // gfx-models: a time before the frame before's (the latency
                // grew after the game was late): the frame before's own
                // pictures in between, in sixteenths, rather than holding it
                if at < -1.0 / 32.0 && self.hfr.dense && !ready.is_empty() && std::env::var_os("ZAKURO_PACE_COVER").is_none_or(|v| v != "old") {
                    // in the frame before's own time, which spans its gap
                    let previous_gap = self.hfr.gaps.iter().rev().nth(1).copied().unwrap_or(gap).max(1) as f64;
                    let f = 1.0 + at * gap.max(1) as f64 / previous_gap;
                    // castlecam: exactly, not in sixteenths (FATES_HFR_EXACT=0 as before)
                    let f = f.clamp(0.0, 1.0);
                    pick = Pick::Before(if exact_on() { f as f32 } else { (f * 16.0).round() as f32 / 16.0 });
                }
                let gap_f = gap.max(1) as f64;
                content -= gap_f * (1.0 - match pick {
                    Pick::Previous => 0.0,
                    Pick::Between(layer) => ready.get(layer - 1).copied().unwrap_or(1.0) as f64,
                    Pick::Newest => 1.0,
                    Pick::Before(f) => (f as f64 - 1.0) * self.hfr.gaps.iter().rev().nth(1).copied().unwrap_or(gap).max(1) as f64 / gap_f,
                    Pick::At(f) => f as f64,
                });
                // smooth2: a frame whose post effects read the frames before
                // (the battle transitions' swirl and blur) has no picture in
                // between and was held for its whole time, then jumped: it is
                // cross-faded from the picture before instead
                let fade = at.clamp(0.0, 1.0) as f32;
                if ready.is_empty() && gap <= 2 && fade > 0.02 && fade < 0.98 && crossfades() && zakuro_core::native_gfx::newest_carried(0) {
                    pick = Pick::Previous;
                    self.hfr.crossfade = Some(fade);
                    content = self.hfr.latched.map_or(0.0, |(_, refresh, _)| refresh as f64) - gap_f * (1.0 - fade as f64);
                }
                match pick {
                    Pick::Previous => game.system.show_previous(),
                    Pick::Between(layer) => game.system.show_layer(layer),
                    Pick::Newest => game.system.show_layer(0),
                    Pick::Before(f) => game.system.show_before(f),
                    Pick::At(f) => game.system.show_at(f),
                }
            } else if playing && pending > 0 && self.hfr.latched.map(|(_, refresh, _)| refresh) == self.hfr.shown_refresh {
                // without pictures in between, a picture once is enough
                // while others wait; the display shows it on
                break;
            }
            self.hfr.shown_refresh = self.hfr.latched.map(|(_, refresh, _)| refresh);
            if let Some(backend) = &mut self.backend {
                backend.set_present_at(Some(slot));
            }
            // rtpace: in real time the picture in between is drawn on a
            // thread of its own while the game's threads have the time
            let ahead = match realtime && playing && interpolating {
                true => self.rtpace_draw_ahead(slot),
                false => Duration::ZERO,
            };
            let start = Instant::now();
            self.present(event_loop);
            if let Some(game) = &mut self.game {
                game.system.show_layer(0);
                if let Some(clock) = &mut game.clock {
                    clock.present();
                }
            }
            let took = start.elapsed() + ahead; // rtpace: the drawing too
            // rtpace: what drawing a picture takes, towards the slow ones
            self.hfr.draw_cost = if took > self.hfr.draw_cost { (self.hfr.draw_cost + took) / 2 } else { (self.hfr.draw_cost * 15 + took) / 16 };
            self.spent.showing.add(took);
            // rt-merge: towards the slow ones fast, back down slowly
            self.hfr.present_cost = if took > self.hfr.present_cost { took } else { (self.hfr.present_cost * 15 + took) / 16 };
            self.hfr.present_mean = (self.hfr.present_mean * 15 + took) / 16; // smooth2
            zakuro_gpu::smooth::since("present", start, self.hfr.presented as i64 + 1, content);
            self.hfr.count_distinct(content);
            self.shown += 1;
            self.hfr.presented += 1;
            self.hfr.in_between += matches!(pick, Pick::Between(_) | Pick::Before(_) | Pick::At(_)) as u64;
            recorded += 1;
        }

        // the game's refresh when due: the pictures ahead cover it
        let now = Instant::now();
        if now > self.next_frame + super::CATCH_UP_LIMIT {
            self.next_frame = now;
            self.hfr.latched = None;
        }
        if playing && now >= self.next_frame {
            if zakuro_gpu::smooth::enabled() {
                // the pictures ahead as the refresh starts, and how far they reach
                let (_, _, pending, last) = self.backend.as_ref()?.pacing()?;
                let reach = last.map_or(0.0, |last| last.saturating_duration_since(now).as_secs_f64() * 1e3);
                zakuro_gpu::smooth::point("cover", now, pending as i64, reach);
            }
            self.hfr_settle();
            self.hfr_refresh();
            // the next pass records at once
            return Some(Instant::now());
        }
        if !playing {
            self.next_frame = now + refresh();
        }
        // wake for the game's refresh, or when a picture ahead went out
        let (_, display, pending, last) = self.backend.as_ref()?.pacing()?;
        let mut wake = if playing { self.next_frame } else { now + Duration::from_millis(4) };
        // rt-merge: the game's frame first, until the next picture must start
        self.hfr.game_first = game_first.is_some();
        if let Some(start_by) = game_first {
            wake = wake.min(start_by);
        } else if !waiting_for_game && !refresh_first {
            let free = match (pending >= depth, last) {
                (true, Some(last)) => last.checked_sub(display * (depth as u32 - 1)).unwrap_or(now),
                _ => now,
            };
            wake = wake.min(free.max(now + Duration::from_micros(200)));
        }
        Some(wake.min(now + Duration::from_millis(4)))
    }
}

/// sleeps until a time, the last stretch spinning, which the OS's sleep
/// would overshoot by up to a millisecond or more.
pub fn precise_sleep_until(when: Instant) {
    let now = Instant::now();
    if when <= now {
        return;
    }
    let left = when - now;
    if left > Duration::from_micros(1500) {
        std::thread::sleep(left - Duration::from_micros(1000));
    }
    while Instant::now() < when {
        std::hint::spin_loop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_cover_the_presents_of_a_picture() {
        assert!(fractions(60.0, 1).is_empty(), "the console's rate draws none");
        assert_eq!(fractions(120.0, 1), vec![0.5], "two presents a picture");
        assert_eq!(fractions(60.0, 2), vec![0.5], "a 30 Hz scene at 60");
        assert_eq!(fractions(144.0, 1).len(), 2, "2.4 presents a picture, thirds");
        assert_eq!(fractions(165.0, 1).len(), 2);
        assert_eq!(fractions(240.0, 1), vec![0.25, 0.5, 0.75]);
        assert_eq!(fractions(240.0, 2).len(), 7, "capped at the layers there are");
    }

    #[test]
    fn at_60_a_30_fps_scene_gets_one_picture_halfway() {
        // hfr60: the band where only 30 FPS scenes are smoothed
        assert!(console_rate(60.0) && console_rate(59.94) && console_rate(60.0 * 1.0001));
        assert!(!console_rate(61.5) && !console_rate(75.0) && !console_rate(50.0) && !console_rate(30.0));
        for rate in [59.94, 60.0] {
            assert_eq!(fractions(rate, 2), vec![0.5], "{rate}: one halfway");
            assert!(fractions(rate, 1).is_empty(), "{rate}: a 60 FPS scene none");
        }
        // shown the nearest: the frame before, the halfway picture, the newest
        assert_eq!(nearest(&[0.5], 0.2, true), Pick::Previous);
        assert_eq!(nearest(&[0.5], 0.3, true), Pick::Between(1));
        assert_eq!(nearest(&[0.5], 0.8, true), Pick::Newest);
    }

    #[test]
    fn a_hitch_is_no_cadence() {
        let q = |v: &[u64]| v.iter().copied().collect::<std::collections::VecDeque<u64>>();
        assert_eq!(cadence(&q(&[2, 2, 4, 2, 2, 2])), 2);
        assert_eq!(cadence(&q(&[1, 2, 1, 2, 1])), 2);
        assert_eq!(cadence(&q(&[3, 3, 3])), 3);
        assert_eq!(cadence(&q(&[1, 1, 1, 9])), 1);
        assert_eq!(cadence(&q(&[])), 1);
    }

    #[test]
    fn variable_refresh_stays_under_the_display() {
        // vrrcap: the reporter's 170 Hz, this PC's 175, and the usual others
        assert!((super::vrr_limit(170.0) - 161.97).abs() < 0.01);
        assert!((super::vrr_limit(175.0) - 166.49).abs() < 0.01);
        assert!((super::vrr_limit(144.0) - 138.24).abs() < 0.01);
        assert!((super::vrr_limit(60.0) - 59.0).abs() < 0.01);
        for hz in [48.0, 60.0, 120.0, 144.0, 165.0, 170.0, 175.0, 240.0, 360.0] {
            let limit = super::vrr_limit(hz);
            assert!(limit < hz * 0.97 + 1.0 && limit > hz * 0.89, "{hz}: {limit}");
        }
    }

    #[test]
    fn variable_refresh_off_paces_as_before_and_on_stays_under() {
        // vrrcap: off, the rate is what hfr_rate gave before (written out
        // here as it was); on, never above the limit, Display and VSync off
        // uncapped included, and lower rates as asked
        let before = |frame_rate: u32, vsync: bool, fps_cap: u32, display: f64, presents: bool| {
            let mut rate = if frame_rate == 0 { display } else { frame_rate as f64 };
            if !vsync && frame_rate == 0 {
                rate = super::UNCAPPED;
            }
            if vsync && presents {
                rate = rate.min(display);
            }
            if fps_cap > 0 {
                rate = rate.min(fps_cap as f64);
            }
            rate
        };
        for &display in &[60.0, 120.0, 144.0, 165.0, 170.0, 175.0, 240.0] {
            for &frame_rate in &[0u32, 30, 60, 120, 144, 165, 240] {
                for &fps_cap in &[0u32, 30, 60, 120, 144, 165, 240] {
                    for vsync in [false, true] {
                        for presents in [false, true] {
                            let off = super::paced_rate(frame_rate, vsync, fps_cap, false, display, presents);
                            assert_eq!(off, before(frame_rate, vsync, fps_cap, display, presents));
                            let on = super::paced_rate(frame_rate, vsync, fps_cap, true, display, presents);
                            assert!(on <= super::vrr_limit(display) + 1e-9 && on <= off);
                            if off < super::vrr_limit(display) {
                                assert_eq!(on, off, "a rate under the limit stays as asked");
                            }
                        }
                    }
                }
            }
        }
        // the reporter's 170 Hz: Display with VSync on or off, and 165, go to 162; 144 stays
        for vsync in [false, true] {
            assert!((super::paced_rate(0, vsync, 0, true, 170.0, true) - 161.97).abs() < 0.01);
            assert!((super::paced_rate(165, vsync, 0, true, 170.0, true) - 161.97).abs() < 0.01);
            assert_eq!(super::paced_rate(144, vsync, 0, true, 170.0, true), 144.0);
        }
    }

    #[test]
    fn the_nearest_picture_is_shown() {
        let ready = [0.25, 0.5, 0.75];
        assert_eq!(nearest(&ready, 0.05, true), Pick::Previous);
        assert_eq!(nearest(&ready, 0.3, true), Pick::Between(1));
        assert_eq!(nearest(&ready, 0.9, true), Pick::Newest);
        assert_eq!(nearest(&[], 0.1, false), Pick::Newest, "nothing in between, the newest at once");
    }
}
