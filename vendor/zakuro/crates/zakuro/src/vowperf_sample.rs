//! vowperf: a sampling profiler for headless runs, a test tool that is off
//! unless `HAIKU_SAMPLE=<report file>` names where its report goes.
//!
//! A thread of its own wakes about every half millisecond, stops each of
//! the process's other threads in turn (SuspendThread), reads where it is
//! (GetThreadContext, rip) and lets it go on. Nothing is allocated while a
//! thread is stopped, so a thread holding the heap's lock can't hang it.
//! Samples where a thread waits (in ntdll, kernel32 or kernelbase) are
//! counted apart and left out of the tables.
//!
//! The report names each sample's place: the guest function (the entry of
//! the game's module or of a code mod's patch module whose code is nearest
//! below rip), or the host module and its offset, which `addr2line` turns
//! into a function of Haiku.exe. `HAIKU_SAMPLE_FRAMES=<from>-<to>` keeps
//! only the samples taken while the headless loop ran those frames; each
//! sample is also marked with whether overrides/info_ui.c was predicting
//! (its word 28) at the time.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;

/// the frame the headless loop is on, which the loop keeps up to date.
pub static FRAME: AtomicU64 = AtomicU64::new(0);

static STOP: AtomicBool = AtomicBool::new(false);
static SAMPLER: Mutex<Option<std::thread::JoinHandle<Tally>>> = Mutex::new(None);

#[derive(Default)]
struct Tally {
    /// (thread id, rip, predicting) -> samples
    hits: HashMap<(u32, usize, bool), u64>,
    waits: u64,
    taken: u64,
    nanos_in_sampler: u64,
}

#[cfg(windows)]
mod win {
    pub const TH32CS_SNAPTHREAD: u32 = 0x4;
    pub const THREAD_SUSPEND_RESUME: u32 = 0x2;
    pub const THREAD_GET_CONTEXT: u32 = 0x8;
    pub const THREAD_QUERY_INFORMATION: u32 = 0x40;
    pub const CONTEXT_CONTROL: u32 = 0x0010_0001;

    #[repr(C)]
    pub struct ThreadEntry32 {
        pub size: u32,
        pub usage: u32,
        pub thread_id: u32,
        pub owner_process_id: u32,
        pub base_pri: i32,
        pub delta_pri: i32,
        pub flags: u32,
    }

    /// the x64 CONTEXT, 1232 bytes, 16-byte aligned; ContextFlags at 0x30,
    /// Rip at 0xF8.
    #[repr(C, align(16))]
    pub struct Context(pub [u8; 1232]);

    #[repr(C)]
    pub struct MemoryBasicInformation {
        pub base_address: usize,
        pub allocation_base: usize,
        pub allocation_protect: u32,
        pub partition_id: u16,
        pub region_size: usize,
        pub state: u32,
        pub protect: u32,
        pub kind: u32,
    }

    unsafe extern "system" {
        pub fn CreateToolhelp32Snapshot(flags: u32, pid: u32) -> isize;
        pub fn Thread32First(snapshot: isize, entry: *mut ThreadEntry32) -> i32;
        pub fn Thread32Next(snapshot: isize, entry: *mut ThreadEntry32) -> i32;
        pub fn CloseHandle(handle: isize) -> i32;
        pub fn OpenThread(access: u32, inherit: i32, id: u32) -> isize;
        pub fn SuspendThread(thread: isize) -> u32;
        pub fn ResumeThread(thread: isize) -> u32;
        pub fn GetThreadContext(thread: isize, context: *mut Context) -> i32;
        pub fn GetCurrentProcessId() -> u32;
        pub fn GetCurrentThreadId() -> u32;
        pub fn VirtualQuery(address: usize, info: *mut MemoryBasicInformation, length: usize) -> usize;
        pub fn GetModuleFileNameW(module: usize, name: *mut u16, size: u32) -> u32;
    }
}

/// starts sampling when HAIKU_SAMPLE is set. points are the recompiled
/// code's entries (recompiled::Library::code_points).
pub fn start(points: Vec<(usize, u32, bool)>) {
    let Some(path) = std::env::var_os("HAIKU_SAMPLE") else { return };
    let window = std::env::var("HAIKU_SAMPLE_FRAMES").ok().and_then(|v| {
        let (a, b) = v.split_once('-')?;
        Some((a.trim().parse::<u64>().ok()?, b.trim().parse::<u64>().ok()?))
    });
    #[cfg(windows)]
    {
        let handle = std::thread::Builder::new().name("vowperf-sampler".into()).spawn(move || sample(window)).ok();
        *SAMPLER.lock().unwrap() = handle;
        POINTS.lock().unwrap().replace((points, path.into()));
    }
    #[cfg(not(windows))]
    {
        let _ = (points, path, window);
    }
}

static POINTS: Mutex<Option<(Vec<(usize, u32, bool)>, std::path::PathBuf)>> = Mutex::new(None);

#[cfg(windows)]
fn sample(window: Option<(u64, u64)>) -> Tally {
    use win::*;
    let mut tally = Tally::default();
    let me = unsafe { GetCurrentThreadId() };
    let pid = unsafe { GetCurrentProcessId() };
    let mut threads: Vec<(u32, isize)> = Vec::new();
    let mut refreshed = std::time::Instant::now() - std::time::Duration::from_secs(10);
    let mut context = Box::new(Context([0; 1232]));
    let mut rips: Vec<(u32, usize)> = Vec::with_capacity(256);
    while !STOP.load(Ordering::Relaxed) {
        std::thread::sleep(std::time::Duration::from_micros(500));
        if let Some((from, to)) = window {
            let frame = FRAME.load(Ordering::Relaxed);
            if frame < from || frame > to {
                continue;
            }
        }
        if refreshed.elapsed() > std::time::Duration::from_millis(500) {
            refreshed = std::time::Instant::now();
            for &(_, handle) in &threads {
                unsafe { CloseHandle(handle) };
            }
            threads.clear();
            unsafe {
                let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
                if snapshot != -1 {
                    let mut entry = ThreadEntry32 { size: std::mem::size_of::<ThreadEntry32>() as u32, usage: 0, thread_id: 0, owner_process_id: 0, base_pri: 0, delta_pri: 0, flags: 0 };
                    let mut more = Thread32First(snapshot, &mut entry) != 0;
                    while more {
                        if entry.owner_process_id == pid && entry.thread_id != me {
                            let handle = OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, 0, entry.thread_id);
                            if handle != 0 {
                                threads.push((entry.thread_id, handle));
                            }
                        }
                        more = Thread32Next(snapshot, &mut entry) != 0;
                    }
                    CloseHandle(snapshot);
                }
            }
        }
        let started = std::time::Instant::now();
        let predicting = crate::info_ui::read(28).unwrap_or(0) != 0;
        rips.clear();
        for &(id, handle) in &threads {
            unsafe {
                if SuspendThread(handle) == u32::MAX {
                    continue;
                }
                context.0[0x30..0x34].copy_from_slice(&CONTEXT_CONTROL.to_le_bytes());
                let ok = GetThreadContext(handle, &mut *context) != 0;
                ResumeThread(handle);
                if ok && rips.len() < rips.capacity() {
                    let rip = usize::from_le_bytes(context.0[0xF8..0x100].try_into().unwrap());
                    rips.push((id, rip));
                }
            }
        }
        tally.taken += 1;
        for &(id, rip) in &rips {
            *tally.hits.entry((id, rip, predicting)).or_insert(0) += 1;
        }
        tally.nanos_in_sampler += started.elapsed().as_nanos() as u64;
    }
    for &(_, handle) in &threads {
        unsafe { CloseHandle(handle) };
    }
    tally
}

#[cfg(windows)]
fn module_of(rip: usize, names: &mut HashMap<usize, String>) -> (usize, String) {
    use win::*;
    let mut info = MemoryBasicInformation { base_address: 0, allocation_base: 0, allocation_protect: 0, partition_id: 0, region_size: 0, state: 0, protect: 0, kind: 0 };
    let base = if unsafe { VirtualQuery(rip, &mut info, std::mem::size_of::<MemoryBasicInformation>()) } != 0 { info.allocation_base } else { 0 };
    let name = names
        .entry(base)
        .or_insert_with(|| {
            let mut buf = [0u16; 512];
            let n = unsafe { GetModuleFileNameW(base, buf.as_mut_ptr(), buf.len() as u32) } as usize;
            if n == 0 {
                "?".to_owned()
            } else {
                let full = String::from_utf16_lossy(&buf[..n]);
                full.rsplit(['\\', '/']).next().unwrap_or(&full).to_owned()
            }
        })
        .clone();
    (base, name)
}

/// stops sampling and writes the report.
pub fn report() {
    let Some(handle) = SAMPLER.lock().unwrap().take() else { return };
    STOP.store(true, Ordering::Relaxed);
    let Ok(tally) = handle.join() else { return };
    let Some((points, path)) = POINTS.lock().unwrap().take() else { return };
    #[cfg(windows)]
    {
        let mut names = HashMap::new();
        // by place: guest function or module+offset; per thread
        let mut by_place: HashMap<(String, bool), u64> = HashMap::new();
        // moflexdec: and by thread, to tell the threads' places apart
        let mut by_thread_place: HashMap<(u32, String), u64> = HashMap::new();
        let mut by_thread: HashMap<u32, (u64, u64)> = HashMap::new();
        let mut by_module: HashMap<(String, bool), u64> = HashMap::new();
        let mut waits = 0u64;
        let mut total = 0u64;
        for (&(thread, rip, predicting), &count) in &tally.hits {
            let (base, module) = module_of(rip, &mut names);
            let lower = module.to_ascii_lowercase();
            if lower == "ntdll.dll" || lower == "kernel32.dll" || lower == "kernelbase.dll" || lower == "win32u.dll" {
                waits += count;
                by_thread.entry(thread).or_insert((0, 0)).1 += count;
                continue;
            }
            total += count;
            by_thread.entry(thread).or_insert((0, 0)).0 += count;
            *by_module.entry((module.clone(), predicting)).or_insert(0) += count;
            let place = match points.partition_point(|&(host, _, _)| host <= rip) {
                0 => None,
                i => Some(points[i - 1]),
            };
            let name = match place {
                Some((host, guest, patch)) if (lower == "game.dll" || lower == "patch.dll") && rip - host < 1 << 20 => {
                    format!("{} 0x{guest:08X}", if patch { "patch" } else { "game " })
                }
                _ => format!("{module}+0x{:X}", rip - base),
            };
            *by_thread_place.entry((thread, name.clone())).or_insert(0) += count;
            *by_place.entry((name, predicting)).or_insert(0) += count;
        }
        let mut text = format!(
            "# vowperf sampler: {} rounds, {} samples in code, {} waiting; {:.1} ms in the sampler\n",
            tally.taken,
            total,
            waits + tally.waits,
            tally.nanos_in_sampler as f64 / 1e6
        );
        text += "# threads (id: samples in code, waiting)\n";
        let mut threads: Vec<_> = by_thread.into_iter().collect();
        threads.sort_by_key(|&(_, (busy, _))| std::cmp::Reverse(busy));
        for (id, (busy, wait)) in threads {
            text += &format!("thread {id}\t{busy}\t{wait}\n");
        }
        text += "# modules (predicting = info_ui.c's prediction ran)\n";
        let mut modules: Vec<_> = by_module.into_iter().collect();
        modules.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
        for ((module, predicting), count) in modules {
            text += &format!("module\t{count}\t{}\t{module}\n", if predicting { "predicting" } else { "-" });
        }
        text += "# places\n";
        let mut places: Vec<_> = by_place.into_iter().collect();
        places.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
        for ((place, predicting), count) in places.into_iter().take(400) {
            text += &format!("place\t{count}\t{}\t{place}\n", if predicting { "predicting" } else { "-" });
        }
        text += "# places by thread (moflexdec)
";
        let mut tplaces: Vec<_> = by_thread_place.into_iter().collect();
        tplaces.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
        for ((thread, place), count) in tplaces.into_iter().take(600) {
            text += &format!("tplace	{thread}	{count}	{place}
");
        }
        let _ = std::fs::write(&path, text);
    }
    #[cfg(not(windows))]
    {
        let _ = (tally, points, path);
    }
}
