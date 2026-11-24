//! compile32 (issue #13, "stuck at 32% in compiling the game's code"):
//! how Haiku Setup runs the compiler on the game's C. It used the
//! recompiler's own loop (recomp3ds::compile::objects, which code mods
//! still use); this one runs the same compiler with the same flags and the
//! same command line, so the objects are the same bytes, and around that:
//!
//! - **memory.** How many compilers run at once comes from the memory the
//!   PC has free and what each file is expected to take (gcc keeps a whole
//!   file of generated functions in memory: up to 400 bytes for each byte
//!   of C). The slowest files (the longest functions) go first. A file starts
//!   only when the memory left holds it, so the biggest never run side by
//!   side on a PC with little memory; one always runs, whatever it needs.
//! - **a job object for each compiler (Windows).** Its memory and processor
//!   time are read while it runs (the log and the progress screen show
//!   them); what it starts (cc1, as) ends with it; a crash ends it at once
//!   instead of waiting on Windows' error reporting; and it can't take more
//!   than the PC's memory less 1.5 GB, so a file too big for the PC fails
//!   (and goes to a lower level at once) rather than swapping for hours.
//! - **a compiler that does nothing** (no processor time for five minutes:
//!   held by antivirus, or stuck) is stopped and started again; buildslow:
//!   only where the PC reports the compilers' processor time (Wine and
//!   Proton report none: issue #17's Steam Deck had every compiler at 0 s,
//!   and the slowest file stopped three times). Everywhere, a compiler
//!   that runs far longer than its file should (at least an hour; eight
//!   times what the files done so far say it should take) is stopped and
//!   tried again with fewer optimisations.
//! - **buildslow (issue #13): a function too long for -O2.** gcc's time
//!   at -O2 grows much faster than a function's length:
//!   `event::InitializeCommand` (1.1 MB of C, USA's code155.c, EUR's
//!   code156.c) took 6 minutes here and over an hour on a Celeron, a tenth
//!   of the whole build. It runs once, at start (it registers the event
//!   commands), so a function that long is compiled at -O1 (an attribute
//!   on its declaration, in a header given with -include; the rest of the
//!   file stays at -O2), in seconds.
//! - **a file that fails** is compiled again (on its own, once nothing else
//!   runs, when it was memory or a crash); if that fails for want of memory
//!   too, or it reached its cap, at -O1, then -O0. Only then does that
//!   file's code differ from other PCs' (it runs the same, a little
//!   slower), and the log and the last screen say so.
//!
//! The recompiler's sources are part of the module's stamp (every change
//! there has every player's Haiku.exe build the module again), so this
//! lives in Haiku Setup instead; `FLAGS` and `GCC_FLAGS` must stay the same
//! as recomp3ds::compile's (a test checks).

use std::collections::VecDeque;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::log;

/// recomp3ds::compile's FLAGS (floating point rounds exactly as the
/// interpreter's: nothing fused into a multiply-add) and GCC_FLAGS (gcc
/// collects its garbage as often on a big PC as on a small one)
pub const FLAGS: &[&str] = &["-O2", "-ffp-contract=off", "-fno-math-errno", "-w"];
pub const GCC_FLAGS: &[&str] = &["--param", "ggc-min-expand=20", "--param", "ggc-min-heapsize=65536"];

/// buildslow (issue #17): the Wine or Proton Haiku runs under, if it does
/// (ntdll's wine_get_version, else Wine's registry key), as "Wine 9.0" or
/// "Wine 9.0, Proton"; once, then kept
pub fn wine() -> Option<String> {
    static WINE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    WINE.get_or_init(|| {
        // HAIKU_SETUP_TEST_WINE=<version>: act as under that Wine (a test)
        if let Some(test) = std::env::var("HAIKU_SETUP_TEST_WINE").ok().filter(|v| !v.is_empty()) {
            return Some(format!("Wine {test}, HAIKU_SETUP_TEST_WINE"));
        }
        #[cfg(windows)]
        let found = win::wine_version();
        #[cfg(not(windows))]
        let found: Option<String> = None;
        found.map(|w| if std::env::var_os("STEAM_COMPAT_DATA_PATH").is_some() || std::env::var_os("SteamAppId").is_some() { format!("{w}, Steam's Proton") } else { w })
    })
    .clone()
}

/// buildslow: the cores Haiku may use (the process's affinity, which a
/// player or a test can limit)
pub fn cores() -> usize {
    let all = std::thread::available_parallelism().map_or(4, |n| n.get());
    #[cfg(windows)]
    if let Some(n) = win::affinity_cores() {
        return n.clamp(1, all.max(1));
    }
    all
}

/// what a file of generated functions takes in gcc at its peak, for each
/// byte of C: gcc 16.2 at -O2 took 150 (the median) to 380 on USA's files
/// (dev/reports/compile32.md; recomp3ds::compile reckons 600, which made a
/// PC with less than about 3 GB free compile one file at a time); entries.c
/// (the tables) took 13
const PER_BYTE: u64 = 400;
const PER_BYTE_TABLES: u64 = 30;
/// a compiler takes at least this much
const LEAST: u64 = 200 << 20;
/// the memory left to everything else when deciding what runs
const RESERVE: u64 = 1 << 30;
/// what a compiler may never take: the PC's memory less this
const CAP_RESERVE: u64 = 1536 << 20;
/// a compiler that gets less processor time than this in STALL is stopped
const STALL: Duration = Duration::from_secs(300);
const STALL_CPU: Duration = Duration::from_secs(1);

/// the optimisation levels a file is tried at, in turn, once a try on its
/// own ran out of memory
const LEVELS: &[&str] = &["-O2", "-O1", "-O0"];

/// buildslow: a function at least this long is compiled at -O1 (only
/// `event::InitializeCommand`, 1.1 MB, in every version; the next longest
/// are 0.54 MB)
pub const SLOW_FUNCTION: u64 = 768 << 10;
/// buildslow: a compiler running longer than this, and LIMIT_TIMES what
/// its file should take at this PC's pace, is stopped
const LIMIT_FLOOR: Duration = Duration::from_secs(3600);
const LIMIT_TIMES: f64 = 8.0;
/// a compiler that hasn't started in this long is given up on (issue #17:
/// a start that never came back left "running nothing" until Cancel,
/// which couldn't stop it either)
const START_LIMIT: Duration = Duration::from_secs(60);
/// the time left is shown once this many files have finished
pub const ESTIMATE_AFTER: usize = 3;

/// buildslow: what a file is expected to take, in seconds of one core of
/// the PC the figures were fitted on (dev/buildslow/fit.py, on the per-file
/// processor times of a USA build on an E-core of an i9-13900KS: 14.2,
/// 284, 3.1, rms error of the log 0.38): MODEL_A for each MB of C, and
/// MODEL_B times the longest function's length in MB to the power MODEL_P
/// (gcc's time grows much faster than a function's length). Only the shape matters: the
/// estimate scales it by the pace of the files finished.
const MODEL_A: f64 = 14.0;
const MODEL_B: f64 = 280.0;
const MODEL_P: f64 = 3.1;
/// a function lowered to -O1 takes about this much per MB
const MODEL_LOWERED: f64 = 14.0;

/// what compiling a file should take (see MODEL_A)
pub fn predicted(name: &str, bytes: u64, scan: &Scan, level: usize) -> f64 {
    let mb = bytes as f64 / 1e6;
    if name == "entries.c" {
        return 1.0 + mb * 0.2;
    }
    let lowered: u64 = if level == 0 { scan.slow.iter().map(|f| f.length).sum() } else { 0 };
    let kept = if level == 0 { scan.longest_kept } else { scan.longest };
    let base = MODEL_A * (bytes - lowered.min(bytes)) as f64 / 1e6 + MODEL_LOWERED * lowered as f64 / 1e6 + MODEL_B * (kept as f64 / 1e6).powf(MODEL_P);
    // -O1 and -O0 take a fraction of -O2's time
    base * [1.0, 0.35, 0.2][level.min(2)]
}

/// one compiler running, for the progress screen and the log
#[derive(Clone, Debug)]
pub struct Activity {
    pub file: String,
    pub started: Instant,
    /// the most memory it has taken so far (0 when it can't be read)
    pub peak: u64,
    /// the processor time it has had
    pub cpu: Duration,
    /// 0: the first try; more: tried again
    pub attempt: usize,
    pub level: &'static str,
    /// buildslow: what it should take (predicted)
    pub predicted: f64,
}

/// what the caller sees while it runs
#[derive(Default)]
pub struct Watch {
    pub running: Mutex<Vec<Activity>>,
    /// bytes of C compiled (finished files)
    pub bytes_done: std::sync::atomic::AtomicU64,
    /// buildslow: the work done and to do, for the time left
    pub plan: Mutex<Plan>,
    /// buildslow: false while the compilers' processor time isn't known to
    /// be reported (Wine, Proton), so nobody is shown a misleading 0 s
    pub cpu_known: AtomicBool,
}

/// buildslow: the build's work, in predicted seconds (see MODEL_A), and
/// what the files finished really took
#[derive(Default, Clone, Debug)]
pub struct Plan {
    pub predicted_total: f64,
    pub predicted_done: f64,
    /// the finished files' times, in seconds, and what was predicted for
    /// the tries that made them
    pub took_done: f64,
    pub predicted_ran: f64,
    pub files_done: usize,
    pub jobs: usize,
}

impl Plan {
    /// how long the files finished took for each predicted second: this
    /// PC's pace (1 on the PC the figures were fitted on, 10 on one ten
    /// times slower), once ESTIMATE_AFTER files have finished
    pub fn pace(&self) -> Option<f64> {
        (self.files_done >= ESTIMATE_AFTER && self.predicted_ran > 0.0).then(|| self.took_done / self.predicted_ran)
    }
}

impl Watch {
    /// buildslow (issue #13): how long the rest of the build should take,
    /// from this PC's pace on the files finished so far: what is left of
    /// those running and the files not started, at as many at once as have
    /// run on average, and never less than the longest of those running
    pub fn left(&self, elapsed: Duration) -> Option<Duration> {
        let plan = self.plan.lock().unwrap().clone();
        let pace = plan.pace()?;
        let running = self.running.lock().unwrap().clone();
        Some(time_left(&plan, pace, &running.iter().map(|a| (a.predicted, a.started.elapsed().as_secs_f64())).collect::<Vec<_>>(), elapsed.as_secs_f64()))
    }
}

/// the time left: `running` is each running file's (predicted, seconds so far)
pub fn time_left(plan: &Plan, pace: f64, running: &[(f64, f64)], elapsed: f64) -> Duration {
    let mut running_left = 0.0;
    let mut longest_left = 0.0f64;
    let mut running_predicted = 0.0;
    for &(predicted, so_far) in running {
        // one past its time is taken to need a tenth of it again
        let left = (predicted * pace - so_far).max(predicted * pace * 0.1);
        running_left += left;
        longest_left = longest_left.max(left);
        running_predicted += predicted;
    }
    let queued = (plan.predicted_total - plan.predicted_done - running_predicted).max(0.0) * pace;
    let busy = (plan.took_done + running.iter().map(|r| r.1).sum::<f64>()) / elapsed.max(1.0);
    let at_once = busy.clamp(1.0, plan.jobs.max(1) as f64);
    Duration::from_secs_f64(((running_left + queued) / at_once).max(longest_left))
}

/// buildslow: "about 25 min" or "about 1 h 50 min"
pub fn about(d: Duration) -> String {
    let min = (d.as_secs_f64() / 60.0).ceil().max(1.0) as u64;
    if min >= 60 {
        // past an hour, to five minutes
        let min = min.div_ceil(5) * 5;
        format!("about {} h {:02} min", min / 60, min % 60)
    } else {
        format!("about {min} min")
    }
}

/// what came of a build
#[derive(Default, Debug)]
pub struct Outcome {
    /// files compiled at a lower level than -O2, and at which
    pub lowered: Vec<(String, &'static str)>,
    /// how many tries were made again
    pub retried: usize,
}

/// the PC's memory, in bytes
#[derive(Clone, Copy, Debug)]
pub struct Memory {
    pub total: u64,
    pub free: u64,
    /// what can still be committed (RAM and page file)
    pub commit_free: u64,
}

/// HAIKU_SETUP_TEST_MEMORY=<GB> has the build act as on a PC with that
/// much memory, of which 2.5 GB are taken by everything else (a test of
/// the scheduling and of a file that runs out of memory, on a big PC)
fn test_memory() -> Option<u64> {
    std::env::var("HAIKU_SETUP_TEST_MEMORY").ok().and_then(|v| v.trim().parse::<f64>().ok()).filter(|&gb| gb > 0.0).map(|gb| (gb * (1u64 << 30) as f64) as u64)
}

const TEST_OTHERS: u64 = 2560 << 20;

/// the memory now; `ours` is what the compilers running take (only used by
/// the test memory, which can't see them)
pub fn memory(ours: u64) -> Option<Memory> {
    if let Some(total) = test_memory() {
        let free = total.saturating_sub(TEST_OTHERS).saturating_sub(ours);
        return Some(Memory { total, free, commit_free: free });
    }
    system_memory()
}

#[cfg(windows)]
fn system_memory() -> Option<Memory> {
    let s = win::memory_status()?;
    Some(Memory { total: s.total_physical, free: s.available_physical, commit_free: s.available_page_file })
}

#[cfg(not(windows))]
fn system_memory() -> Option<Memory> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let field = |name: &str| -> Option<u64> {
        let line = info.lines().find(|l| l.starts_with(name))?;
        Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
    };
    Some(Memory { total: field("MemTotal:")?, free: field("MemAvailable:")?, commit_free: field("MemAvailable:")? + field("SwapFree:").unwrap_or(0) })
}

/// what compiling a file of `bytes` of C named `name` is expected to take
/// at its peak
pub fn expected(name: &str, bytes: u64) -> u64 {
    let per = if name == "entries.c" { PER_BYTE_TABLES } else { PER_BYTE };
    (bytes * per).max(LEAST)
}

/// the length of the longest function in a file of generated C, a function
/// starting at a line that starts with a letter and ends in ") {"
#[cfg(test)]
pub fn longest_function(text: &[u8]) -> u64 {
    scan(text).longest
}

/// buildslow: a function too long for -O2 (SLOW_FUNCTION): the line it
/// starts with, less " {" (`void f_00424968(Context *ctx)`), and its length
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlowFunction {
    pub declaration: String,
    pub length: u64,
}

/// what a file of generated C holds, for the order, the estimate and -O1
#[derive(Clone, Debug, Default)]
pub struct Scan {
    /// the longest function's length
    pub longest: u64,
    /// the longest of those left at -O2
    pub longest_kept: u64,
    /// those compiled at -O1
    pub slow: Vec<SlowFunction>,
}

/// the functions of a file of generated C: each starts at a line that
/// starts with a letter and ends in ") {", and runs to the next
pub fn scan(text: &[u8]) -> Scan {
    let mut out = Scan::default();
    let mut start: Option<(usize, &[u8])> = None;
    let mut at = 0;
    let close = |out: &mut Scan, s: usize, line: &[u8], end: usize| {
        let length = (end - s) as u64;
        out.longest = out.longest.max(length);
        // only a plain declaration is copied into the header
        let declaration = String::from_utf8_lossy(&line[..line.len() - 2]).trim_end().to_owned();
        let plain = declaration.bytes().all(|b| b.is_ascii_alphanumeric() || b" _*(),".contains(&b));
        if length >= SLOW_FUNCTION && plain && declaration.contains('(') {
            out.slow.push(SlowFunction { declaration, length });
        } else {
            out.longest_kept = out.longest_kept.max(length);
        }
    };
    for line in text.split(|&b| b == b'\n') {
        let line_start = at;
        at += line.len() + 1;
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.first().is_some_and(|b| b.is_ascii_alphabetic()) && line.ends_with(b") {") {
            if let Some((s, l)) = start {
                close(&mut out, s, l, line_start);
            }
            start = Some((line_start, line));
        }
    }
    if let Some((s, l)) = start {
        close(&mut out, s, l, text.len().max(s));
    }
    out
}

/// buildslow: the header that has a file's too-long functions compiled at
/// -O1: recomp.h (guarded), then each one declared with the attribute
/// before the file's own declarations and definition, which take it on
pub fn slow_header(slow: &[SlowFunction]) -> String {
    let mut text = String::from("/* Haiku Setup (buildslow): functions too long for -O2, compiled at -O1. */\n#include \"recomp.h\"\n");
    for f in slow {
        text += &format!("__attribute__((optimize(\"O1\"))) {};\n", f.declaration);
    }
    text
}

/// whether a file expected to take `need` may start beside the ones
/// running: within the budget made at the start (the memory free then,
/// less the reserve), and within the memory free now, less what those
/// running are still expected to grow by
pub fn fits(need: u64, using: u64, budget: u64, free_now: u64, still_to_grow: u64) -> bool {
    using + need <= budget && need + still_to_grow + RESERVE <= free_now
}

pub fn gb(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1u64 << 30) as f64)
}

pub fn clock(d: Duration) -> String {
    let s = d.as_secs();
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

/// why a try failed
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// it reached its cap (the PC's memory less 1.5 GB): it would every
    /// time, whatever else runs
    AtCap,
    /// gcc said so, or Windows refused it memory (the PC's memory and page
    /// file were taken, by it and everything else)
    Memory,
    /// no processor time for STALL
    Stalled,
    /// buildslow: ran far longer than its file should (LIMIT_FLOOR)
    TooLong,
    /// it crashed (an exception code) without saying it was memory
    Crashed,
    /// it said why (a compile error, a file it can't write)
    Other,
}

/// what a try's exit and output say
pub fn classify(code: Option<i32>, output: &str, peak: u64, cap: Option<u64>) -> Failure {
    let text = output.to_ascii_lowercase();
    let said_memory = ["out of memory", "virtual memory exhausted", "cannot allocate memory", "not enough memory", "memory exhausted"].iter().any(|s| text.contains(s));
    let code = code.map(|c| c as u32);
    // STATUS_NO_MEMORY, STATUS_COMMITMENT_LIMIT, STATUS_STACK_OVERFLOW (a
    // deep function: a smaller level recurses less, too)
    let memory_code = code.is_some_and(|c| matches!(c, 0xC000_0017 | 0xC000_012D | 0xC000_00FD));
    let at_cap = cap.is_some_and(|cap| peak + (64 << 20) >= cap);
    if at_cap && (said_memory || memory_code || code.is_some_and(|c| c != 0)) {
        Failure::AtCap
    } else if said_memory || memory_code {
        Failure::Memory
    } else if code.is_some_and(|c| c >= 0xC000_0000) {
        Failure::Crashed
    } else {
        Failure::Other
    }
}

/// what to do after a try failed: the level to try again at, or nothing.
/// A file that reached its cap goes to the next level at once (it would
/// reach it again). Otherwise the first failure is tried again at the same
/// level (the same object a PC with more memory makes: it may have failed
/// beside others); only memory, or a compiler that crashes on its own,
/// takes it to a lower level; a compiler stuck three times is given up on.
/// buildslow: one that ran far too long goes lower at once (at the same
/// level it would take as long again).
pub fn next_try(failure: Failure, attempt: usize, level: usize) -> Option<usize> {
    let lower = (level + 1 < LEVELS.len()).then_some(level + 1);
    match failure {
        Failure::AtCap | Failure::TooLong => lower,
        _ if attempt == 0 => Some(level),
        Failure::Memory | Failure::Crashed => lower,
        Failure::Stalled => (attempt < 2).then_some(level),
        Failure::Other => None,
    }
}

/// one try at one file
struct Try {
    index: usize,
    attempt: usize,
    level: usize,
    /// run with nothing else beside it
    alone: bool,
}

struct State {
    next: usize,
    again: VecDeque<Try>,
    running: usize,
    /// the expected needs of those running
    using: u64,
    alone: bool,
    failed: Vec<(String, Failure)>,
    outcome: Outcome,
}

/// buildslow: whether the compilers' processor time can be read, which the
/// rule for a stuck compiler (no processor time in 5 minutes) needs
struct Accounting {
    state: std::sync::atomic::AtomicU8,
}

/// not trusted: Wine and Proton (their job objects report no processor time)
const ACCOUNTING_UNTRUSTED: u8 = 0;
/// Windows: expected to work
const ACCOUNTING_EXPECTED: u8 = 1;
/// seen working: a compiler had processor time
const ACCOUNTING_SEEN: u8 = 2;
/// seen not working: a compiler that ran for seconds finished with none
const ACCOUNTING_BROKEN: u8 = 3;

impl Accounting {
    fn usable(&self) -> bool {
        matches!(self.state.load(Ordering::Relaxed), ACCOUNTING_EXPECTED | ACCOUNTING_SEEN)
    }

    /// a reading of a running compiler
    fn reading(&self, cpu: Duration, watch: &Watch) {
        if cpu > Duration::ZERO && self.state.compare_exchange(ACCOUNTING_EXPECTED, ACCOUNTING_SEEN, Ordering::Relaxed, Ordering::Relaxed).is_ok() {
            watch.cpu_known.store(true, Ordering::Relaxed);
        }
    }

    /// a compiler that finished: one that ran for seconds with no processor
    /// time shows the PC doesn't report it
    fn finished(&self, source: &str, took: Duration, cpu: Duration, watch: &Watch) {
        if cpu == Duration::ZERO && took >= Duration::from_secs(5) {
            let before = self.state.swap(ACCOUNTING_BROKEN, Ordering::Relaxed);
            if before != ACCOUNTING_BROKEN && before != ACCOUNTING_SEEN {
                watch.cpu_known.store(false, Ordering::Relaxed);
                log::line(format!(
                    "compile: this PC doesn't report the compilers' processor time ({source} ran {:.0} s and showed none): a compiler is only stopped when it runs far longer than its file should ({} at least)",
                    took.as_secs_f32(),
                    clock(LIMIT_FLOOR)
                ));
            } else if before == ACCOUNTING_SEEN {
                self.state.store(ACCOUNTING_SEEN, Ordering::Relaxed);
            }
        }
    }
}

/// compiles sources (names inside dir, run from dir with relative names,
/// as recomp3ds::compile does with RECOMP_RELATIVE) with gcc, at most jobs
/// at once, each into an object beside it. done hears after each file how
/// many are done of how many; cancel stops it, ending the compilers.
#[allow(clippy::too_many_arguments)]
pub fn objects(gcc: &Path, dir: &Path, sources: &[String], jobs: usize, watch: &Watch, done: &(dyn Fn(usize, usize, &str, bool) + Sync), cancel: &AtomicBool) -> Result<Outcome, String> {
    let t0 = Instant::now();
    let sizes: Vec<u64> = sources.iter().map(|s| std::fs::metadata(dir.join(s)).map_or(0, |m| m.len())).collect();
    let scans: Vec<Scan> = sources.iter().map(|s| std::fs::read(dir.join(s)).map_or_else(|_| Scan::default(), |text| scan(&text))).collect();
    let needs: Vec<u64> = (0..sources.len()).map(|i| expected(&sources[i], sizes[i])).collect();
    // buildslow: HAIKU_SETUP_NO_LOWER=1 compiles every function at -O2 (a
    // test: the time it takes, and that the game is the same)
    let lower = std::env::var("HAIKU_SETUP_NO_LOWER").map_or(true, |v| v != "1");
    // the -O1 headers, for the files with a function too long for -O2
    let headers: Vec<Option<String>> = (0..sources.len())
        .map(|i| {
            if !lower || scans[i].slow.is_empty() {
                return None;
            }
            let name = format!("{}.slow.h", Path::new(&sources[i]).file_stem()?.to_string_lossy());
            std::fs::write(dir.join(&name), slow_header(&scans[i].slow)).ok().map(|_| name)
        })
        .collect();
    // what each file is compiled as: without a header, all of it at -O2
    let scans: Vec<Scan> = scans.into_iter().enumerate().map(|(i, s)| if headers[i].is_some() { s } else { Scan { longest: s.longest, longest_kept: s.longest, slow: Vec::new() } }).collect();
    let predicted_at = |i: usize, level: usize| predicted(&sources[i], sizes[i], &scans[i], level);
    let first: Vec<f64> = (0..sources.len()).map(|i| predicted_at(i, 0)).collect();
    // the slowest first (gcc's time grows faster than a function's length),
    // so that it runs beside all the others instead of last, on its own
    let mut order: Vec<usize> = (0..sources.len()).collect();
    order.sort_by(|&a, &b| first[b].total_cmp(&first[a]).then(sizes[b].cmp(&sizes[a])));
    let start = memory(0);
    let budget = start.map_or(u64::MAX, |m| m.free.saturating_sub(RESERVE));
    let cap = start.map(|m| m.total.saturating_sub(CAP_RESERVE).max(2 << 30));
    // HAIKU_SETUP_TEST_CAP=<MB>: each compiler's cap (a test of a file
    // running out of memory)
    let cap = std::env::var("HAIKU_SETUP_TEST_CAP").ok().and_then(|v| v.trim().parse::<u64>().ok()).map(|mb| mb << 20).or(cap);
    let jobs = jobs.max(1);
    let stall = std::env::var("HAIKU_SETUP_STALL_SECS").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&s| s > 0).map_or(STALL, Duration::from_secs);
    // HAIKU_SETUP_LIMIT_SECS=<s>: the time a compiler is let run, whatever
    // its file (a test of one that runs too long)
    let fixed = std::env::var("HAIKU_SETUP_LIMIT_SECS").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&s| s > 0).map(Duration::from_secs);
    let floor = fixed.unwrap_or(LIMIT_FLOOR);
    let wine = wine();
    let accounting = Accounting { state: std::sync::atomic::AtomicU8::new(if wine.is_some() { ACCOUNTING_UNTRUSTED } else { ACCOUNTING_EXPECTED }) };
    watch.cpu_known.store(false, Ordering::Relaxed);
    if let Some(w) = &wine {
        log::line(format!("compile: running under Wine/Proton ({w}): the compilers' processor time isn't reported there, so a compiler is only stopped when it runs far longer than its file should ({} at least)", clock(floor)));
    }
    *watch.plan.lock().unwrap() = Plan { predicted_total: first.iter().sum(), jobs, ..Plan::default() };
    match start {
        Some(m) => log::line(format!(
            "compile: {} files, {:.0} MB of C; memory {}, {} free, {} more can be committed; {jobs} at once at most ({} cores), each up to {}{}",
            sources.len(),
            sizes.iter().sum::<u64>() as f64 / 1e6,
            gb(m.total),
            gb(m.free),
            gb(m.commit_free),
            cores(),
            gb(cap.unwrap_or(0)),
            if test_memory().is_some() { " (test: HAIKU_SETUP_TEST_MEMORY)" } else { "" }
        )),
        None => log::line(format!("compile: {} files; the memory can't be read; {jobs} at once at most", sources.len())),
    }
    for (i, header) in headers.iter().enumerate() {
        if header.is_some() {
            let names: Vec<String> = scans[i].slow.iter().map(|f| format!("{} ({:.0} KB)", f.declaration.split('(').next().unwrap_or("").rsplit(' ').next().unwrap_or(""), f.length as f64 / 1e3)).collect();
            log::line(format!("compile: {}: {} compiled at -O1, the rest at -O2 (a function this long takes gcc minutes at -O2, over an hour on a slow PC)", sources[i], names.join(", ")));
        }
    }
    for &i in order.iter().take(3) {
        log::line(format!("compile: {} ({:.1} MB, its longest function {:.0} KB) first, expected to take {}", sources[i], sizes[i] as f64 / 1e6, scans[i].longest_kept as f64 / 1e3, gb(needs[i])));
    }
    if let Some(i) = (0..sources.len()).max_by_key(|&i| needs[i]) {
        log::line(format!("compile: the most memory is expected for {}: {}", sources[i], gb(needs[i])));
    }

    let state = Mutex::new(State { next: 0, again: VecDeque::new(), running: 0, using: 0, alone: false, failed: Vec::new(), outcome: Outcome::default() });
    let changed = Condvar::new();
    let finished = std::sync::atomic::AtomicUsize::new(0);
    let total = sources.len();
    std::thread::scope(|scope| {
        for _ in 0..jobs.min(sources.len().max(1)) {
            scope.spawn(|| loop {
                // what to compile next, or nothing when all is done
                let task = {
                    let mut s = state.lock().unwrap();
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            break None;
                        }
                        if s.alone {
                            s = changed.wait_timeout(s, Duration::from_secs(1)).unwrap().0;
                            continue;
                        }
                        // a file tried again beside the others (it was stuck,
                        // or failed otherwise) goes before the rest
                        let beside = s.again.iter().position(|t| !t.alone);
                        if beside.is_none() && s.next == order.len() {
                            // what is left runs on its own (it ran out of memory
                            // or crashed beside others), once nothing else runs
                            if s.running > 0 {
                                s = changed.wait_timeout(s, Duration::from_secs(1)).unwrap().0;
                                continue;
                            }
                            let Some(t) = s.again.pop_front() else { break None };
                            s.alone = true;
                            s.running = 1;
                            s.using = needs[t.index];
                            break Some(t);
                        }
                        // buildslow (issue #17): not map_or(order[s.next], ..), which
                        // indexed past the end once every file had started: a file
                        // tried again then panicked every worker ("running nothing",
                        // and Cancel couldn't stop it)
                        let index = match beside {
                            Some(b) => s.again[b].index,
                            None => order[s.next],
                        };
                        let start_it = s.running == 0 || {
                            let live = watch.running.lock().unwrap();
                            let ours: u64 = live.iter().map(|a| a.peak).sum();
                            let to_grow: u64 = live.iter().map(|a| sources.iter().position(|x| *x == a.file).map_or(0, |i| needs[i]).saturating_sub(a.peak)).sum();
                            drop(live);
                            match memory(ours) {
                                Some(now) => fits(needs[index], s.using, budget, now.free, to_grow),
                                None => true,
                            }
                        };
                        if start_it {
                            s.running += 1;
                            s.using += needs[index];
                            if let Some(b) = beside {
                                break s.again.remove(b);
                            }
                            s.next += 1;
                            break Some(Try { index, attempt: 0, level: 0, alone: false });
                        }
                        s = changed.wait_timeout(s, Duration::from_secs(2)).unwrap().0;
                    }
                };
                let Some(task) = task else {
                    changed.notify_all();
                    break;
                };
                let source = &sources[task.index];
                // the -O1 header only beside -O2 (at -O0 it would raise it)
                let header = if task.level == 0 { headers[task.index].as_deref() } else { None };
                let predicted = predicted_at(task.index, task.level);
                let limits = Limits { cap, stall, floor, fixed: fixed.is_some(), predicted, accounting: &accounting };
                let result = compile_one(gcc, dir, source, header, &task, &limits, watch, cancel);
                let mut s = state.lock().unwrap();
                s.running -= 1;
                s.using = s.using.saturating_sub(needs[task.index]);
                if task.alone {
                    s.alone = false;
                }
                match result {
                    Ok(ran) => {
                        if task.attempt > 0 {
                            log::line(format!("compile: {source} compiled{}{} in {:.0} s", if task.alone { " on its own" } else { " again" }, if task.level > 0 { format!(" at {}", LEVELS[task.level]) } else { String::new() }, ran.took.as_secs_f32()));
                        }
                        if task.level > 0 {
                            s.outcome.lowered.push((source.clone(), LEVELS[task.level]));
                        }
                        watch.bytes_done.fetch_add(sizes[task.index], Ordering::Relaxed);
                        let estimate = {
                            let mut plan = watch.plan.lock().unwrap();
                            // the work done is what was planned for the file; the pace,
                            // the time of the try that made it for what it should take
                            plan.predicted_done += first[task.index];
                            plan.took_done += ran.took.as_secs_f64();
                            plan.predicted_ran += predicted;
                            plan.files_done += 1;
                            (plan.files_done == ESTIMATE_AFTER).then(|| plan.pace()).flatten()
                        };
                        drop(s);
                        if let (Some(pace), Some(left)) = (estimate, watch.left(t0.elapsed())) {
                            // buildslow (issue #13): the whole build's time, early, so a
                            // slow PC's two hours are expected rather than "stuck"
                            log::line(format!(
                                "compile: after {ESTIMATE_AFTER} files this PC takes {pace:.2} s for each second the figures expect: the whole compile should take {}, {} from now (an estimate; it is updated as files finish)",
                                about(t0.elapsed() + left),
                                about(left)
                            ));
                        }
                        done(finished.fetch_add(1, Ordering::Relaxed) + 1, total, source, true);
                        changed.notify_all();
                        continue;
                    }
                    Err(failure) => {
                        if cancel.load(Ordering::Relaxed) {
                            drop(s);
                            changed.notify_all();
                            continue;
                        }
                        let next = next_try(failure, task.attempt, task.level);
                        match next {
                            Some(level) => {
                                let why = match failure {
                                    Failure::AtCap => "it needs more memory than this PC has",
                                    Failure::Memory => "it ran out of memory",
                                    Failure::Stalled => "it did nothing for too long",
                                    Failure::TooLong => "it ran far longer than it should",
                                    Failure::Crashed => "the compiler crashed",
                                    Failure::Other => "it failed",
                                };
                                // memory or a crash: on its own, when nothing else
                                // runs; otherwise beside the others, first
                                let alone = matches!(failure, Failure::AtCap | Failure::Memory | Failure::Crashed);
                                log::line(format!(
                                    "compile: {source}: {why}; trying again{}{}",
                                    if alone { " on its own, when nothing else runs" } else { "" },
                                    if level != task.level || level > 0 { format!(", at {}", LEVELS[level]) } else { String::new() }
                                ));
                                s.outcome.retried += 1;
                                s.again.push_back(Try { index: task.index, attempt: task.attempt + 1, level, alone });
                            }
                            None => {
                                log::line(format!("compile: {source} failed ({failure:?}), {} tries", task.attempt + 1));
                                s.failed.push((source.clone(), failure));
                                // what this build can't finish is still counted, so the
                                // others carry on
                                drop(s);
                                done(finished.fetch_add(1, Ordering::Relaxed) + 1, total, source, false);
                                changed.notify_all();
                                continue;
                            }
                        }
                        drop(s);
                        changed.notify_all();
                    }
                }
            });
        }
    });
    for name in headers.iter().flatten() {
        let _ = std::fs::remove_file(dir.join(name));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err("stopped".to_owned());
    }
    let s = state.into_inner().unwrap();
    if !s.failed.is_empty() {
        let names: Vec<&str> = s.failed.iter().map(|(name, _)| name.as_str()).collect();
        let mut text = format!("{} failed to compile, {}", names.len(), names.join(" "));
        // buildslow (issue #17): security software is named only where it
        // can be the cause: a PC that reports processor time showed the
        // compiler getting none, three times
        if s.failed.iter().any(|(_, f)| *f == Failure::Stalled) {
            text += "; the compiler got no processor time for 5 minutes, three times: something on this PC kept it from running, for example security software scanning it. Allow the Haiku folder (setup\\w64devkit) in your security software and build again";
        } else if s.failed.iter().any(|(_, f)| *f == Failure::TooLong) {
            text += "; the compiler ran far longer than this file should take on this PC, at every level of optimisation. Close other programs and build again; what was compiled is kept";
        } else if s.failed.iter().any(|(_, f)| matches!(f, Failure::Memory | Failure::AtCap)) {
            text += "; this PC ran out of memory: close other programs and build again";
        }
        return Err(text);
    }
    Ok(s.outcome)
}

struct Ran {
    took: Duration,
}

/// what a try may take
struct Limits<'a> {
    cap: Option<u64>,
    stall: Duration,
    /// the least time a compiler is let run
    floor: Duration,
    /// the floor is the limit (a test)
    fixed: bool,
    /// what the file should take, in predicted seconds
    predicted: f64,
    accounting: &'a Accounting,
}

/// one try at one file: the compiler started, watched to its end
#[allow(clippy::too_many_arguments)]
fn compile_one(gcc: &Path, dir: &Path, source: &str, header: Option<&str>, task: &Try, limits: &Limits, watch: &Watch, cancel: &AtomicBool) -> Result<Ran, Failure> {
    let object = Path::new(source).with_extension("o");
    let mut command = Command::new(gcc);
    let level = LEVELS[task.level];
    command.args(FLAGS.iter().map(|&f| if f == "-O2" { level } else { f })).args(GCC_FLAGS).current_dir(dir).arg("-I").arg(".");
    if let Some(header) = header {
        command.arg("-include").arg(header);
    }
    command.arg("-c").arg(source).arg("-o").arg(&object).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let started = Instant::now();
    // HAIKU_SETUP_TEST_STALL=<file>: its first try is never let run (a test
    // of a compiler held by antivirus)
    let hold = task.attempt == 0 && std::env::var("HAIKU_SETUP_TEST_STALL").is_ok_and(|v| v == source);
    // HAIKU_SETUP_TEST_START_HANG=<file>: its first start never comes back
    // (a test of issue #17's "running nothing"); HAIKU_SETUP_START_SECS=<s>
    // how long a start is waited for
    let hang = task.attempt == 0 && std::env::var("HAIKU_SETUP_TEST_START_HANG").is_ok_and(|v| v == source);
    let start_limit = std::env::var("HAIKU_SETUP_START_SECS").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&s| s > 0).map_or(START_LIMIT, Duration::from_secs);
    let mut process = match Process::start_within(command, limits.cap, hold, hang, start_limit) {
        Ok(p) => p,
        Err(e) => {
            log::line(format!("compile: {source}: the compiler can't be started ({e})"));
            return Err(Failure::Other);
        }
    };
    watch.running.lock().unwrap().push(Activity { file: source.to_owned(), started, peak: 0, cpu: Duration::ZERO, attempt: task.attempt, level, predicted: limits.predicted });
    let update = |peak: u64, cpu: Duration| {
        let mut live = watch.running.lock().unwrap();
        if let Some(a) = live.iter_mut().find(|a| a.file == source) {
            a.peak = peak;
            a.cpu = cpu;
        }
    };
    let mut mark = (Instant::now(), Duration::ZERO);
    let mut stalled = false;
    let mut too_long = false;
    let mut cancelled = false;
    let mut last_read = Instant::now() - Duration::from_secs(1);
    let code = loop {
        match process.try_wait() {
            Ok(Some(code)) => break Some(code),
            Ok(None) => {}
            Err(_) => break None,
        }
        if cancel.load(Ordering::Relaxed) {
            cancelled = true;
            process.kill();
            break None;
        }
        if last_read.elapsed() >= Duration::from_millis(500) {
            last_read = Instant::now();
            if let Some((peak, cpu)) = process.usage() {
                update(peak, cpu);
                limits.accounting.reading(cpu, watch);
                if cpu >= mark.1 + STALL_CPU {
                    mark = (Instant::now(), cpu);
                } else if mark.0.elapsed() >= limits.stall && limits.accounting.usable() {
                    log::line(format!("compile: {source} has had {:.1} s of processor time in the last {} (min:s; {} in all, {} MB): stopping it", (cpu - mark.1).as_secs_f32(), clock(limits.stall), clock(started.elapsed()), peak >> 20));
                    stalled = true;
                    process.kill();
                    break None;
                }
            }
            // buildslow: far longer than the file should take at this PC's
            // pace (known after a few files; until then the floor)
            let pace = watch.plan.lock().unwrap().pace().unwrap_or(1.0);
            let limit = if limits.fixed { limits.floor } else { limits.floor.max(Duration::from_secs_f64((LIMIT_TIMES * limits.predicted * pace).min(1e6))) };
            if started.elapsed() >= limit {
                log::line(format!("compile: {source} has run {} (min:s), and a file like it should take about {} here: stopping it", clock(started.elapsed()), clock(Duration::from_secs_f64((limits.predicted * pace).min(1e6)))));
                too_long = true;
                process.kill();
                break None;
            }
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let (peak, cpu) = process.usage().unwrap_or((0, Duration::ZERO));
    // what it started ends with it, so its output is all there
    process.end();
    let output = process.output();
    watch.running.lock().unwrap().retain(|a| a.file != source);
    for line in output.lines() {
        log::line(format!("  gcc: {line}"));
    }
    let took = started.elapsed();
    let ok = code == Some(0) && !stalled && !too_long && !cancelled && dir.join(&object).is_file();
    if ok {
        limits.accounting.finished(source, took, cpu, watch);
    }
    let cpu_known = watch.cpu_known.load(Ordering::Relaxed);
    log::line(format!(
        "compile: {source} {} in {:.1} s ({}{}){}",
        if ok { "compiled" } else if cancelled { "stopped" } else { "failed" },
        took.as_secs_f32(),
        if cpu_known { format!("{:.1} s of processor time", cpu.as_secs_f32()) } else { "processor time not reported".to_owned() },
        if peak > 0 { format!(", {} MB at most", peak >> 20) } else { String::new() },
        if level != "-O2" {
            format!(" at {level}")
        } else if header.is_some() {
            " (its longest function at -O1)".to_owned()
        } else {
            String::new()
        }
    ));
    if ok {
        return Ok(Ran { took });
    }
    // no half-written object may pass for a built one
    let _ = std::fs::remove_file(dir.join(&object));
    if cancelled {
        return Err(Failure::Other);
    }
    if stalled || too_long {
        // with the memory short, it was swapping rather than stuck
        if memory(peak).is_some_and(|m| m.free < RESERVE) {
            log::line(format!("compile: {source}: the memory was short ({} free)", gb(memory(peak).map_or(0, |m| m.free))));
            return Err(Failure::Memory);
        }
        return Err(if stalled { Failure::Stalled } else { Failure::TooLong });
    }
    if let Some(code) = code {
        log::line(format!("compile: {source}: the compiler ended with {code} (0x{:08X})", code as u32));
    }
    Err(classify(code, &output, peak, limits.cap))
}

/// waits a while for a process that was told to end (a process that never
/// ends must not hold the build: issue #17, Wine)
fn wait_within(child: &mut std::process::Child, limit: Duration) -> Option<i32> {
    let until = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status.code().unwrap_or(-1)),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(50)),
            _ => return None,
        }
    }
}

/// a compiler running, and its output being read
struct Process {
    child: std::process::Child,
    #[cfg(windows)]
    job: Option<win::Job>,
    out: std::sync::mpsc::Receiver<Vec<u8>>,
}

impl Process {
    /// starts it on a thread of its own, and gives up on it after limit:
    /// what was started late is ended when it comes (its job closes)
    fn start_within(command: Command, cap: Option<u64>, hold: bool, hang: bool, limit: Duration) -> std::io::Result<Process> {
        let (send, receive) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            if hang {
                std::thread::sleep(limit * 2);
            }
            let _ = send.send(Process::start(command, cap, hold));
        });
        match receive.recv_timeout(limit) {
            Ok(result) => result,
            Err(_) => Err(std::io::Error::new(std::io::ErrorKind::TimedOut, format!("it hadn't started after {} s", limit.as_secs()))),
        }
    }

    fn start(mut command: Command, cap: Option<u64>, hold: bool) -> std::io::Result<Process> {
        #[cfg(windows)]
        let (mut child, job) = {
            use std::os::windows::process::CommandExt;
            const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
            const CREATE_SUSPENDED: u32 = 0x0000_0004;
            // nowindow: below normal, with no console window, and held until
            // it is in its job (so cc1 and as are, too)
            command.creation_flags(crate::spawn::CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS | CREATE_SUSPENDED);
            let mut child = command.spawn()?;
            let job = win::Job::new(cap);
            let in_job = job.as_ref().is_some_and(|j| j.assign(&child));
            if (hold && in_job) || win::resume(&child) {
                (child, job.filter(|_| in_job))
            } else {
                // it couldn't be let run: started again the usual way, unwatched
                let _ = child.kill();
                let _ = wait_within(&mut child, Duration::from_secs(10));
                command.creation_flags(crate::spawn::CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
                (command.spawn()?, None)
            }
        };
        #[cfg(not(windows))]
        let mut child = {
            let _ = (cap, hold);
            command.spawn()?
        };
        let (send, out) = std::sync::mpsc::channel();
        for pipe in [child.stdout.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>), child.stderr.take().map(|p| Box::new(p) as Box<dyn std::io::Read + Send>)] {
            let send = send.clone();
            std::thread::spawn(move || {
                let mut text = Vec::new();
                if let Some(mut pipe) = pipe {
                    let _ = pipe.read_to_end(&mut text);
                }
                let _ = send.send(text);
            });
        }
        #[cfg(windows)]
        return Ok(Process { child, job, out });
        #[cfg(not(windows))]
        Ok(Process { child, out })
    }

    /// its exit code once it has ended
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(self.child.try_wait()?.map(|status| status.code().unwrap_or(-1)))
    }

    /// the most memory it (with what it started) has taken, and its
    /// processor time; only known in a job
    fn usage(&self) -> Option<(u64, Duration)> {
        // HAIKU_SETUP_TEST_NO_CPU=1: the job reports nothing, as under Wine
        // (issue #17's log: every compiler at 0 s and 0 MB; a test)
        if std::env::var("HAIKU_SETUP_TEST_NO_CPU").is_ok_and(|v| v == "1") {
            return self.job.as_ref().map(|_| (0, Duration::ZERO));
        }
        #[cfg(windows)]
        return self.job.as_ref().and_then(|j| j.usage());
        #[cfg(not(windows))]
        None
    }

    fn kill(&mut self) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
        let _ = self.child.kill();
        let _ = wait_within(&mut self.child, Duration::from_secs(30));
    }

    /// ends whatever it started that is still there
    fn end(&mut self) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
        if wait_within(&mut self.child, Duration::from_secs(30)).is_none() {
            let _ = self.child.kill();
        }
    }

    /// what it printed (both pipes; a tool left holding them is given up
    /// on after a while)
    fn output(&self) -> String {
        let mut text = Vec::new();
        for _ in 0..2 {
            if let Ok(part) = self.out.recv_timeout(Duration::from_secs(30)) {
                text.extend(part);
            }
        }
        String::from_utf8_lossy(&text).into_owned()
    }
}

impl Drop for Process {
    /// one given up on (start_within) or left running ends
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
    }
}

#[cfg(windows)]
#[allow(non_snake_case, clippy::upper_case_acronyms)]
mod win {
    use std::ffi::c_void;
    use std::time::Duration;

    type HANDLE = isize;

    #[repr(C)]
    #[derive(Default)]
    pub(super) struct BasicLimit {
        per_process_user_time: i64,
        per_job_user_time: i64,
        flags: u32,
        min_working_set: usize,
        max_working_set: usize,
        active_processes: u32,
        affinity: usize,
        priority_class: u32,
        scheduling_class: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    pub(super) struct IoCounters {
        counts: [u64; 6],
    }

    /// JOBOBJECT_EXTENDED_LIMIT_INFORMATION
    #[repr(C)]
    #[derive(Default)]
    pub(super) struct ExtendedLimit {
        basic: BasicLimit,
        io: IoCounters,
        process_memory_limit: usize,
        job_memory_limit: usize,
        peak_process_memory: usize,
        peak_job_memory: usize,
    }

    /// JOBOBJECT_BASIC_ACCOUNTING_INFORMATION
    #[repr(C)]
    #[derive(Default)]
    pub(super) struct Accounting {
        user: i64,
        kernel: i64,
        period_user: i64,
        period_kernel: i64,
        page_faults: u32,
        processes: u32,
        active: u32,
        terminated: u32,
    }

    /// THREADENTRY32
    #[repr(C)]
    #[derive(Default)]
    struct ThreadEntry {
        size: u32,
        usage: u32,
        thread: u32,
        owner: u32,
        base_priority: i32,
        delta_priority: i32,
        flags: u32,
    }

    /// MEMORYSTATUSEX
    #[repr(C)]
    #[derive(Default)]
    pub(super) struct MemoryStatus {
        length: u32,
        load: u32,
        pub total_physical: u64,
        pub available_physical: u64,
        pub total_page_file: u64,
        pub available_page_file: u64,
        total_virtual: u64,
        available_virtual: u64,
        available_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateJobObjectW(attributes: *const c_void, name: *const u16) -> HANDLE;
        fn SetInformationJobObject(job: HANDLE, class: i32, info: *const c_void, length: u32) -> i32;
        fn QueryInformationJobObject(job: HANDLE, class: i32, info: *mut c_void, length: u32, returned: *mut u32) -> i32;
        fn AssignProcessToJobObject(job: HANDLE, process: HANDLE) -> i32;
        fn TerminateJobObject(job: HANDLE, code: u32) -> i32;
        fn CloseHandle(handle: HANDLE) -> i32;
        fn CreateToolhelp32Snapshot(flags: u32, process: u32) -> HANDLE;
        fn Thread32First(snapshot: HANDLE, entry: *mut ThreadEntry) -> i32;
        fn Thread32Next(snapshot: HANDLE, entry: *mut ThreadEntry) -> i32;
        fn OpenThread(access: u32, inherit: i32, thread: u32) -> HANDLE;
        fn ResumeThread(thread: HANDLE) -> u32;
        fn GlobalMemoryStatusEx(status: *mut MemoryStatus) -> i32;
        fn GetModuleHandleW(name: *const u16) -> HANDLE;
        fn GetProcAddress(module: HANDLE, name: *const u8) -> *const c_void;
        fn GetCurrentProcess() -> HANDLE;
        fn GetProcessAffinityMask(process: HANDLE, process_mask: *mut usize, system_mask: *mut usize) -> i32;
    }

    #[link(name = "advapi32")]
    extern "system" {
        fn RegOpenKeyExW(key: HANDLE, sub_key: *const u16, options: u32, access: u32, result: *mut HANDLE) -> i32;
        fn RegCloseKey(key: HANDLE) -> i32;
    }

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// an export of ntdll, or null
    fn ntdll(name: &[u8]) -> *const c_void {
        // SAFETY: ntdll is loaded in every process; the names end in a nul
        unsafe {
            let module = GetModuleHandleW(wide("ntdll.dll").as_ptr());
            if module == 0 {
                return std::ptr::null();
            }
            GetProcAddress(module, name.as_ptr())
        }
    }

    /// buildslow: Wine's version (wine_get_version, which only Wine's ntdll
    /// has), or "Wine" when only its registry key is there
    pub(super) fn wine_version() -> Option<String> {
        let get = ntdll(b"wine_get_version\0");
        if !get.is_null() {
            // SAFETY: wine_get_version takes nothing and returns a static
            // nul-terminated string
            let version = unsafe {
                let get: extern "C" fn() -> *const std::ffi::c_char = std::mem::transmute(get);
                let text = get();
                if text.is_null() { String::new() } else { std::ffi::CStr::from_ptr(text).to_string_lossy().into_owned() }
            };
            return Some(if version.is_empty() { "Wine".to_owned() } else { format!("Wine {version}") });
        }
        const HKEY_CURRENT_USER: HANDLE = 0x8000_0001u32 as i32 as HANDLE;
        const KEY_READ: u32 = 0x2_0019;
        let mut key: HANDLE = 0;
        // SAFETY: the name ends in a nul; the key is closed after
        if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, wide("Software\\Wine").as_ptr(), 0, KEY_READ, &mut key) } == 0 {
            unsafe { RegCloseKey(key) };
            return Some("Wine".to_owned());
        }
        None
    }

    /// buildslow: how many cores this process may run on
    pub(super) fn affinity_cores() -> Option<usize> {
        let (mut process, mut system) = (0usize, 0usize);
        // SAFETY: the pseudo handle of this process, two outputs
        (unsafe { GetProcessAffinityMask(GetCurrentProcess(), &mut process, &mut system) } != 0 && process != 0).then(|| process.count_ones() as usize)
    }

    const BASIC_ACCOUNTING: i32 = 1;
    const EXTENDED_LIMIT: i32 = 9;
    const LIMIT_JOB_MEMORY: u32 = 0x0200;
    const LIMIT_DIE_ON_UNHANDLED_EXCEPTION: u32 = 0x0400;
    const LIMIT_KILL_ON_JOB_CLOSE: u32 = 0x2000;

    pub(super) fn memory_status() -> Option<MemoryStatus> {
        let mut status = MemoryStatus { length: std::mem::size_of::<MemoryStatus>() as u32, ..Default::default() };
        // SAFETY: status is a MEMORYSTATUSEX with its length set
        (unsafe { GlobalMemoryStatusEx(&mut status) } != 0).then_some(status)
    }

    /// a job object holding one compiler and what it starts
    pub(super) struct Job(HANDLE);

    // SAFETY: a job handle may be used from any thread
    unsafe impl Send for Job {}
    unsafe impl Sync for Job {}

    impl Job {
        /// a job whose processes end with it and on a crash, together
        /// taking no more than cap
        pub(super) fn new(cap: Option<u64>) -> Option<Job> {
            // SAFETY: no attributes, no name
            let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
            if handle == 0 {
                return None;
            }
            let job = Job(handle);
            let mut limit = ExtendedLimit::default();
            limit.basic.flags = LIMIT_KILL_ON_JOB_CLOSE | LIMIT_DIE_ON_UNHANDLED_EXCEPTION;
            if let Some(cap) = cap {
                limit.basic.flags |= LIMIT_JOB_MEMORY;
                limit.job_memory_limit = cap as usize;
            }
            // SAFETY: limit is a JOBOBJECT_EXTENDED_LIMIT_INFORMATION of its size
            let set = unsafe { SetInformationJobObject(job.0, EXTENDED_LIMIT, &limit as *const _ as *const c_void, std::mem::size_of::<ExtendedLimit>() as u32) };
            (set != 0).then_some(job)
        }

        pub(super) fn assign(&self, child: &std::process::Child) -> bool {
            use std::os::windows::io::AsRawHandle;
            // SAFETY: both handles are open
            unsafe { AssignProcessToJobObject(self.0, child.as_raw_handle() as HANDLE) != 0 }
        }

        /// the most memory committed at once, and the processor time
        pub(super) fn usage(&self) -> Option<(u64, Duration)> {
            let mut limit = ExtendedLimit::default();
            let mut accounting = Accounting::default();
            // SAFETY: each buffer is the class's structure, of its size
            unsafe {
                if QueryInformationJobObject(self.0, EXTENDED_LIMIT, &mut limit as *mut _ as *mut c_void, std::mem::size_of::<ExtendedLimit>() as u32, std::ptr::null_mut()) == 0 {
                    return None;
                }
                if QueryInformationJobObject(self.0, BASIC_ACCOUNTING, &mut accounting as *mut _ as *mut c_void, std::mem::size_of::<Accounting>() as u32, std::ptr::null_mut()) == 0 {
                    return None;
                }
            }
            let ticks = (accounting.user.max(0) + accounting.kernel.max(0)) as u64;
            Some((limit.peak_job_memory as u64, Duration::from_nanos(ticks * 100)))
        }

        pub(super) fn terminate(&self) {
            // SAFETY: the handle is open
            unsafe { TerminateJobObject(self.0, 1) };
        }
    }

    impl Drop for Job {
        fn drop(&mut self) {
            // SAFETY: closing it ends what is left in it (kill on close)
            unsafe { CloseHandle(self.0) };
        }
    }

    /// lets a process started suspended run: NtResumeProcess on its handle
    /// (buildslow: no snapshot of every thread, which a process that hasn't
    /// finished starting may be missing from, under Wine at least), else
    /// its thread found in a snapshot and resumed
    pub(super) fn resume(child: &std::process::Child) -> bool {
        use std::os::windows::io::AsRawHandle;
        let nt = ntdll(b"NtResumeProcess\0");
        if !nt.is_null() {
            // SAFETY: NtResumeProcess(HANDLE) -> NTSTATUS; the handle is open
            let status = unsafe {
                let nt: extern "system" fn(HANDLE) -> i32 = std::mem::transmute(nt);
                nt(child.as_raw_handle() as HANDLE)
            };
            if status >= 0 {
                return true;
            }
        }
        resume_thread(child.id())
    }

    fn resume_thread(process: u32) -> bool {
        const SNAP_THREAD: u32 = 0x0000_0004;
        const THREAD_SUSPEND_RESUME: u32 = 0x0002;
        // SAFETY: the snapshot and the thread are closed after use; the
        // entry has its size set
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(SNAP_THREAD, 0);
            if snapshot == -1 {
                return false;
            }
            let mut resumed = false;
            let mut entry = ThreadEntry { size: std::mem::size_of::<ThreadEntry>() as u32, ..Default::default() };
            let mut more = Thread32First(snapshot, &mut entry) != 0;
            while more {
                if entry.owner == process {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.thread);
                    if thread != 0 {
                        resumed |= ResumeThread(thread) != u32::MAX;
                        CloseHandle(thread);
                    }
                }
                entry.size = std::mem::size_of::<ThreadEntry>() as u32;
                more = Thread32Next(snapshot, &mut entry) != 0;
            }
            CloseHandle(snapshot);
            resumed
        }
    }

    #[cfg(test)]
    #[test]
    fn the_structures_are_windows_own_size() {
        if cfg!(target_pointer_width = "64") {
            assert_eq!(std::mem::size_of::<BasicLimit>(), 64);
            assert_eq!(std::mem::size_of::<ExtendedLimit>(), 144);
        }
        assert_eq!(std::mem::size_of::<Accounting>(), 48);
        assert_eq!(std::mem::size_of::<ThreadEntry>(), 28);
        assert_eq!(std::mem::size_of::<MemoryStatus>(), 64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flags_are_the_recompilers() {
        // the objects must be the same bytes as recomp3ds::compile makes
        let text = include_str!("../../vendor/3dsrecomp/src/compile.rs");
        let quoted = |flags: &[&str]| flags.iter().map(|f| format!("\"{f}\"")).collect::<Vec<_>>().join(", ");
        assert!(text.contains(&format!("const FLAGS: &[&str] = &[{}];", quoted(FLAGS))), "recomp3ds::compile's FLAGS changed");
        assert!(text.contains(&format!("const GCC_FLAGS: &[&str] = &[{}];", quoted(GCC_FLAGS))), "recomp3ds::compile's GCC_FLAGS changed");
    }

    #[test]
    fn a_file_starts_only_when_the_memory_holds_it() {
        let g = 1u64 << 30;
        // 16 GB free: two files of 1 GB beside each other
        assert!(fits(g, g, 15 * g, 14 * g, 0));
        // the budget made at the start is spent
        assert!(!fits(g, 14 * g + g / 2, 15 * g, 14 * g, 0));
        // what those running are still to take counts
        assert!(!fits(g, g, 15 * g, 3 * g, 2 * g));
        assert!(fits(g, g, 15 * g, 3 * g, 0));
    }

    #[test]
    fn what_a_failure_was() {
        assert_eq!(classify(Some(1), "cc1.exe: out of memory allocating 65536 bytes after a total of 6442450944 bytes", 0, None), Failure::Memory);
        assert_eq!(classify(Some(0xC000_0017u32 as i32), "", 0, None), Failure::Memory);
        assert_eq!(classify(Some(1), "", (6 << 30) - (10 << 20), Some(6 << 30)), Failure::AtCap);
        assert_eq!(classify(Some(1), "cc1.exe: out of memory allocating 16931680 bytes", 402 << 20, Some(400 << 20)), Failure::AtCap);
        assert_eq!(classify(Some(1), "cc1.exe: out of memory allocating 16931680 bytes", 2 << 30, Some(6 << 30)), Failure::Memory);
        assert_eq!(classify(Some(0xC000_0005u32 as i32), "", 0, Some(6 << 30)), Failure::Crashed);
        assert_eq!(classify(Some(1), "code001.c:1: error: expected ';'", 1 << 30, Some(6 << 30)), Failure::Other);
    }

    #[test]
    fn a_failed_file_is_tried_again_then_lower_only_for_memory() {
        assert_eq!(next_try(Failure::Other, 0, 0), Some(0));
        assert_eq!(next_try(Failure::Memory, 0, 0), Some(0));
        assert_eq!(next_try(Failure::Memory, 1, 0), Some(1));
        assert_eq!(next_try(Failure::Memory, 2, 1), Some(2));
        assert_eq!(next_try(Failure::Memory, 3, 2), None);
        assert_eq!(next_try(Failure::AtCap, 0, 0), Some(1));
        assert_eq!(next_try(Failure::AtCap, 1, 2), None);
        assert_eq!(next_try(Failure::Other, 1, 0), None);
        assert_eq!(next_try(Failure::Stalled, 1, 0), Some(0));
        assert_eq!(next_try(Failure::Stalled, 2, 0), None);
        assert_eq!(next_try(Failure::Crashed, 1, 0), Some(1));
        // buildslow: too long goes lower at once
        assert_eq!(next_try(Failure::TooLong, 0, 0), Some(1));
        assert_eq!(next_try(Failure::TooLong, 1, 1), Some(2));
        assert_eq!(next_try(Failure::TooLong, 2, 2), None);
    }

    #[test]
    fn a_function_too_long_for_o2_is_found_and_declared() {
        let mut c = b"#include \"recomp.h\"
#include \"functions.h\"

void f_1(Context *ctx) {
    x;
}

void f_00424968(Context *ctx) {
".to_vec();
        c.extend(std::iter::repeat_n(b"    y;
".as_slice(), (SLOW_FUNCTION / 7 + 1) as usize).flatten());
        c.extend(b"}

void f_3(Context *ctx) {
    z;
}
");
        let s = scan(&c);
        assert_eq!(s.slow.len(), 1);
        assert_eq!(s.slow[0].declaration, "void f_00424968(Context *ctx)");
        assert!(s.slow[0].length >= SLOW_FUNCTION);
        assert_eq!(s.longest, s.slow[0].length);
        assert!(s.longest_kept < 100);
        let header = slow_header(&s.slow);
        assert!(header.contains("#include \"recomp.h\"
"));
        assert!(header.contains("__attribute__((optimize(\"O1\"))) void f_00424968(Context *ctx);
"));
        // nothing that long: nothing lowered
        assert!(scan(b"void f_1(Context *ctx) {
}
").slow.is_empty());
    }

    #[test]
    fn the_jobs_for_a_pc_with_few_cores() {
        // issue #13: a 2-core Celeron had 1
        assert_eq!(crate::toolchain::jobs_for(1), 1);
        assert_eq!(crate::toolchain::jobs_for(2), 2);
        assert_eq!(crate::toolchain::jobs_for(4), 3);
        assert_eq!(crate::toolchain::jobs_for(8), 6);
        assert_eq!(crate::toolchain::jobs_for(32), 30);
    }

    #[test]
    fn the_time_left_follows_the_pace() {
        // 100 predicted seconds, 10 done in 30 s (pace 3), 2 at once, one
        // running (predicted 20, 15 s in)
        let plan = Plan { predicted_total: 100.0, predicted_done: 10.0, took_done: 30.0, predicted_ran: 10.0, files_done: 3, jobs: 2 };
        assert_eq!(plan.pace(), Some(3.0));
        let left = time_left(&plan, 3.0, &[(20.0, 15.0)], 22.5);
        // running: 60 - 15 = 45 left; queued: 70 * 3 = 210; 2 at once
        assert_eq!(left.as_secs(), (45 + 210) / 2);
        // the longest running file bounds it from below
        let left = time_left(&plan, 3.0, &[(90.0, 0.0)], 30.0);
        assert_eq!(left.as_secs(), 270);
        // too few files: no estimate
        assert_eq!(Plan { files_done: 2, ..plan }.pace(), None);
        assert_eq!(about(Duration::from_secs(25 * 60 - 10)), "about 25 min");
        assert_eq!(about(Duration::from_secs(107 * 60)), "about 1 h 50 min");
    }

    #[test]
    fn a_function_lowered_is_predicted_quicker() {
        let mb = 1_000_000u64;
        let whole = Scan { longest: 1_119_000, longest_kept: 1_119_000, slow: Vec::new() };
        let lowered = Scan { longest: 1_119_000, longest_kept: 200_000, slow: vec![SlowFunction { declaration: "void f(Context *ctx)".into(), length: 1_119_000 }] };
        let a = predicted("code155.c", 3 * mb / 2, &whole, 0);
        let b = predicted("code155.c", 3 * mb / 2, &lowered, 0);
        assert!(b * 5.0 < a, "{a} {b}");
        assert!(predicted("code001.c", mb, &Scan { longest: 60_000, longest_kept: 60_000, slow: Vec::new() }, 0) > 1.0);
    }

    #[test]
    fn the_longest_function_is_found() {
        let c = b"#include \"recomp.h\"\n\nvoid f_1(Context *ctx) {\n    x;\n}\n\nvoid f_2(Context *ctx) {\n    y;\n    y;\n    y;\n}\nconst uint32_t t[] = {\n1,\n};\n";
        // f_2 runs from its line to the end (the table after it included)
        let f2 = c.len() - String::from_utf8_lossy(c).find("void f_2").unwrap();
        assert_eq!(longest_function(c), f2 as u64);
        assert_eq!(longest_function(b"no functions here\n"), 0);
    }

    #[test]
    fn times_read_as_a_clock() {
        assert_eq!(clock(Duration::from_secs(65)), "1:05");
        assert_eq!(clock(Duration::from_secs(3725)), "1:02:05");
    }
}
