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
//!   held by antivirus, or stuck) is stopped and started again.
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
}

/// what the caller sees while it runs
#[derive(Default)]
pub struct Watch {
    pub running: Mutex<Vec<Activity>>,
    /// bytes of C compiled (finished files)
    pub bytes_done: std::sync::atomic::AtomicU64,
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
pub fn longest_function(text: &[u8]) -> u64 {
    let mut longest = 0;
    let mut start = None;
    let mut at = 0;
    for line in text.split(|&b| b == b'\n') {
        let line_start = at;
        at += line.len() + 1;
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.first().is_some_and(|b| b.is_ascii_alphabetic()) && line.ends_with(b") {") {
            if let Some(s) = start {
                longest = longest.max(line_start - s);
            }
            start = Some(line_start);
        }
    }
    if let Some(s) = start {
        longest = longest.max(text.len().saturating_sub(s));
    }
    longest as u64
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
pub fn next_try(failure: Failure, attempt: usize, level: usize) -> Option<usize> {
    let lower = (level + 1 < LEVELS.len()).then_some(level + 1);
    match failure {
        Failure::AtCap => lower,
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

/// compiles sources (names inside dir, run from dir with relative names,
/// as recomp3ds::compile does with RECOMP_RELATIVE) with gcc, at most jobs
/// at once, each into an object beside it. done hears after each file how
/// many are done of how many; cancel stops it, ending the compilers.
#[allow(clippy::too_many_arguments)]
pub fn objects(gcc: &Path, dir: &Path, sources: &[String], jobs: usize, watch: &Watch, done: &(dyn Fn(usize, usize, &str, bool) + Sync), cancel: &AtomicBool) -> Result<Outcome, String> {
    let sizes: Vec<u64> = sources.iter().map(|s| std::fs::metadata(dir.join(s)).map_or(0, |m| m.len())).collect();
    let longest: Vec<u64> = sources.iter().map(|s| std::fs::read(dir.join(s)).map_or(0, |text| longest_function(&text))).collect();
    let needs: Vec<u64> = (0..sources.len()).map(|i| expected(&sources[i], sizes[i])).collect();
    // the slowest first (the longest function: gcc's time grows faster
    // than a function's length), so that it runs beside all the others
    // instead of last, on its own
    let mut order: Vec<usize> = (0..sources.len()).collect();
    order.sort_by_key(|&i| (std::cmp::Reverse(longest[i]), std::cmp::Reverse(sizes[i])));
    let start = memory(0);
    let budget = start.map_or(u64::MAX, |m| m.free.saturating_sub(RESERVE));
    let cap = start.map(|m| m.total.saturating_sub(CAP_RESERVE).max(2 << 30));
    // HAIKU_SETUP_TEST_CAP=<MB>: each compiler's cap (a test of a file
    // running out of memory)
    let cap = std::env::var("HAIKU_SETUP_TEST_CAP").ok().and_then(|v| v.trim().parse::<u64>().ok()).map(|mb| mb << 20).or(cap);
    let jobs = jobs.max(1);
    let stall = std::env::var("HAIKU_SETUP_STALL_SECS").ok().and_then(|v| v.trim().parse::<u64>().ok()).filter(|&s| s > 0).map_or(STALL, Duration::from_secs);
    match start {
        Some(m) => log::line(format!(
            "compile: {} files, {:.0} MB of C; memory {}, {} free, {} more can be committed; {jobs} at once at most, each up to {}{}",
            sources.len(),
            sizes.iter().sum::<u64>() as f64 / 1e6,
            gb(m.total),
            gb(m.free),
            gb(m.commit_free),
            gb(cap.unwrap_or(0)),
            if test_memory().is_some() { " (test: HAIKU_SETUP_TEST_MEMORY)" } else { "" }
        )),
        None => log::line(format!("compile: {} files; the memory can't be read; {jobs} at once at most", sources.len())),
    }
    for &i in order.iter().take(3) {
        log::line(format!("compile: {} ({:.1} MB, its longest function {:.0} KB) first, expected to take {}", sources[i], sizes[i] as f64 / 1e6, longest[i] as f64 / 1e3, gb(needs[i])));
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
                        let index = beside.map_or(order[s.next], |b| s.again[b].index);
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
                let result = compile_one(gcc, dir, source, &task, cap, stall, watch, cancel);
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
                        drop(s);
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
    if cancel.load(Ordering::Relaxed) {
        return Err("stopped".to_owned());
    }
    let s = state.into_inner().unwrap();
    if !s.failed.is_empty() {
        let names: Vec<&str> = s.failed.iter().map(|(name, _)| name.as_str()).collect();
        let mut text = format!("{} failed to compile, {}", names.len(), names.join(" "));
        if s.failed.iter().any(|(_, f)| *f == Failure::Stalled) {
            text += "; the compiler stopped working, three times: antivirus software can hold it. Allow the Haiku folder (setup\\w64devkit) in it and build again";
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

/// one try at one file: the compiler started, watched to its end
#[allow(clippy::too_many_arguments)]
fn compile_one(gcc: &Path, dir: &Path, source: &str, task: &Try, cap: Option<u64>, stall: Duration, watch: &Watch, cancel: &AtomicBool) -> Result<Ran, Failure> {
    let object = Path::new(source).with_extension("o");
    let mut command = Command::new(gcc);
    let level = LEVELS[task.level];
    command
        .args(FLAGS.iter().map(|&f| if f == "-O2" { level } else { f }))
        .args(GCC_FLAGS);
    if cfg!(unix) {
        // the module links into a shared library on unix
        command.arg("-fPIC");
    }
    command
        .current_dir(dir)
        .arg("-I")
        .arg(".")
        .arg("-c")
        .arg(source)
        .arg("-o")
        .arg(&object)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    // HAIKU_SETUP_TEST_STALL=<file>: its first try is never let run (a test
    // of a compiler held by antivirus)
    let hold = task.attempt == 0 && std::env::var("HAIKU_SETUP_TEST_STALL").is_ok_and(|v| v == source);
    let mut process = match Process::start(command, cap, hold) {
        Ok(p) => p,
        Err(e) => {
            log::line(format!("compile: {source}: the compiler can't be started ({e})"));
            return Err(Failure::Other);
        }
    };
    watch.running.lock().unwrap().push(Activity { file: source.to_owned(), started, peak: 0, cpu: Duration::ZERO, attempt: task.attempt, level });
    let update = |peak: u64, cpu: Duration| {
        let mut live = watch.running.lock().unwrap();
        if let Some(a) = live.iter_mut().find(|a| a.file == source) {
            a.peak = peak;
            a.cpu = cpu;
        }
    };
    let mut mark = (Instant::now(), Duration::ZERO);
    let mut stalled = false;
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
                if cpu >= mark.1 + STALL_CPU {
                    mark = (Instant::now(), cpu);
                } else if mark.0.elapsed() >= stall {
                    log::line(format!("compile: {source} has had {:.1} s of processor time in the last {} (min:s; {} in all, {} MB): stopping it", (cpu - mark.1).as_secs_f32(), clock(stall), clock(started.elapsed()), peak >> 20));
                    stalled = true;
                    process.kill();
                    break None;
                }
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
    let ok = code == Some(0) && !stalled && !cancelled && dir.join(&object).is_file();
    log::line(format!(
        "compile: {source} {} in {:.1} s ({:.1} s of processor time{}){}",
        if ok { "compiled" } else if cancelled { "stopped" } else { "failed" },
        took.as_secs_f32(),
        cpu.as_secs_f32(),
        if peak > 0 { format!(", {} MB at most", peak >> 20) } else { String::new() },
        if level != "-O2" { format!(" at {level}") } else { String::new() }
    ));
    if ok {
        return Ok(Ran { took });
    }
    // no half-written object may pass for a built one
    let _ = std::fs::remove_file(dir.join(&object));
    if cancelled {
        return Err(Failure::Other);
    }
    if stalled {
        // with the memory short, it was swapping rather than stuck
        if memory(peak).is_some_and(|m| m.free < RESERVE) {
            log::line(format!("compile: {source}: the memory was short ({} free)", gb(memory(peak).map_or(0, |m| m.free))));
            return Err(Failure::Memory);
        }
        return Err(Failure::Stalled);
    }
    if let Some(code) = code {
        log::line(format!("compile: {source}: the compiler ended with {code} (0x{:08X})", code as u32));
    }
    Err(classify(code, &output, peak, cap))
}

/// a compiler running, and its output being read
struct Process {
    child: std::process::Child,
    #[cfg(windows)]
    job: Option<win::Job>,
    out: std::sync::mpsc::Receiver<Vec<u8>>,
}

impl Process {
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
            if (hold && in_job) || win::resume(child.id()) {
                (child, job.filter(|_| in_job))
            } else {
                // it couldn't be let run: started again the usual way, unwatched
                let _ = child.kill();
                let _ = child.wait();
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
        let _ = self.child.wait();
    }

    /// ends whatever it started that is still there
    fn end(&mut self) {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            job.terminate();
        }
        let _ = self.child.wait();
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

    /// lets a process started suspended run: its one thread resumed
    pub(super) fn resume(process: u32) -> bool {
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
