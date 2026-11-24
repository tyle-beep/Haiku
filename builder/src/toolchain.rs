//! The C compiler and linker that come with Haiku Setup (a subset of
//! w64devkit's GCC 16.2: gcc, cc1, as, ld, ar and the MinGW-w64 runtime),
//! the same compiler the developers build with, so the recompiled code is
//! the same code.
//!
//! The MinGW tools open files through the ANSI code page, so they can't
//! open a path with letters outside it (a folder named in Japanese, or a
//! user name with an accent). Haiku Setup therefore runs them in the folder
//! they work in, with relative ASCII names, and gives them an ASCII path to
//! themselves and to their temporary folder: the path as it is when it is
//! ASCII, else (only the temporary folder) its 8.3 short name, else a
//! junction, removed again when the build ends, in C:\Users\Public.

use std::path::{Path, PathBuf};

use crate::log;

pub fn is_ascii(path: &Path) -> bool {
    path.to_str().is_some_and(|s| s.is_ascii())
}

#[cfg(windows)]
pub fn short_path(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    #[link(name = "kernel32")]
    extern "system" {
        fn GetShortPathNameW(long: *const u16, short: *mut u16, len: u32) -> u32;
    }
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
    let mut buf = vec![0u16; 1024];
    // SAFETY: wide is NUL-terminated, buf holds len characters
    let n = unsafe { GetShortPathNameW(wide.as_ptr(), buf.as_mut_ptr(), buf.len() as u32) } as usize;
    if n == 0 || n >= buf.len() {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_wide(&buf[..n])))
}

#[cfg(not(windows))]
pub fn short_path(_: &Path) -> Option<PathBuf> {
    None
}

/// an ASCII way to name dir (which exists), and the junction made for it,
/// which `release` removes. The junction keeps dir's own name, inside a
/// folder of this run's (C:\Users\Public\HaikuSetup-<pid>): gcc finds its
/// sysroot as the folder named w64devkit beside its own.
/// gcc finds its own files from the long path of gcc.exe (it asks Windows
/// for its module's name), so a short name doesn't help it: the toolchain
/// takes a junction (`short` false), the temporary folder a short name.
pub fn ascii_dir(dir: &Path, short: bool) -> Result<(PathBuf, Option<PathBuf>), String> {
    if is_ascii(dir) {
        return Ok((dir.to_path_buf(), None));
    }
    // HAIKU_SETUP_NO_SHORT=1 skips the short name (to test the junction)
    let short = if !short || std::env::var("HAIKU_SETUP_NO_SHORT").is_ok_and(|v| v == "1") { None } else { short_path(dir) };
    if let Some(short) = short.filter(|p| is_ascii(p)) {
        log::line(format!("using the short path {} for {}", short.display(), dir.display()));
        return Ok((short, None));
    }
    // oneexe: the fallback made from the system drive rather than written
    // whole: compiled into Haiku.exe, the literal path became instruction
    // immediates that the release's personal audit reads as a home path
    let fallback = || PathBuf::from(format!("{}\\", std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_owned()))).join("Users").join("Public");
    let public = std::env::var_os("PUBLIC").map(PathBuf::from).filter(|p| is_ascii(p) && p.is_dir()).unwrap_or_else(fallback);
    let holder = public.join(format!("HaikuSetup-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&holder);
    let link = holder.join(dir.file_name().unwrap_or_default());
    if link.exists() {
        release(&link);
    }
    let mut mklink = crate::spawn::command("cmd");
    mklink.args(["/c", "mklink", "/J"]).arg(&link).arg(dir);
    let status = crate::spawn::run(mklink, "mklink");
    if status.is_ok_and(|s| s.success()) && link.is_dir() {
        log::line(format!("using the junction {} for {}", link.display(), dir.display()));
        return Ok((link.clone(), Some(link)));
    }
    Err("The folder Haiku is in has letters the compiler can't read, and no other way to name it could be made. Move the Haiku folder to a path with only English letters and numbers, such as C:\\Games\\Haiku, and start Haiku.exe again.".into())
}

/// removes a junction made by ascii_dir (the link only, never what it
/// points at), and the folder holding it once that is empty.
pub fn release(link: &Path) {
    let mut rmdir = crate::spawn::command("cmd");
    rmdir.args(["/c", "rmdir"]).arg(link);
    let _ = crate::spawn::run(rmdir, "rmdir");
    if let Some(holder) = link.parent().filter(|h| h.file_name().is_some_and(|n| n.to_string_lossy().starts_with("HaikuSetup-"))) {
        let _ = std::fs::remove_dir(holder);
    }
}

/// the bundled toolchain, ready to run.
pub struct Toolchain {
    pub bin: PathBuf,
    pub gcc: PathBuf,
    pub ar: PathBuf,
    pub tmp: PathBuf,
    junctions: Vec<PathBuf>,
}

impl Drop for Toolchain {
    fn drop(&mut self) {
        for link in &self.junctions {
            release(link);
        }
    }
}

impl Toolchain {
    /// the toolchain in setup/w64devkit (or, for a development run, the one
    /// HAIKU_SETUP_TOOLCHAIN names), its temporary folder in work. The
    /// folder has to be called w64devkit: gcc finds its sysroot by that
    /// name, beside its own folder.
    pub fn prepare(setup: &Path, work: &Path) -> Result<Toolchain, String> {
        let dir = std::env::var_os("HAIKU_SETUP_TOOLCHAIN").map(PathBuf::from).unwrap_or_else(|| setup.join("w64devkit"));
        if !dir.join("bin").join("gcc.exe").is_file() {
            return Err(format!("The compiler that comes with Haiku is missing ({}). Extract the whole Haiku download again.", dir.join("bin").display()));
        }
        let mut junctions = Vec::new();
        let (dir, link) = ascii_dir(&dir, false)?;
        junctions.extend(link);
        let tmp = work.join("tmp");
        std::fs::create_dir_all(&tmp).map_err(|e| format!("{}: {e}", tmp.display()))?;
        let (tmp, link) = ascii_dir(&tmp, true)?;
        junctions.extend(link);
        let bin = dir.join("bin");
        Ok(Toolchain { gcc: bin.join("gcc.exe"), ar: bin.join("ar.exe"), bin, tmp, junctions })
    }

    /// the environment the tools run in: this process's, which the
    /// recompiler's compile step passes on to every compiler.
    pub fn enter(&self, jobs: usize) {
        let path = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![self.bin.clone()];
        paths.extend(std::env::split_paths(&path));
        std::env::set_var("PATH", std::env::join_paths(paths).unwrap_or_default());
        std::env::set_var("CC", &self.gcc);
        std::env::set_var("AR", &self.ar);
        std::env::set_var("TMP", &self.tmp);
        std::env::set_var("TEMP", &self.tmp);
        std::env::set_var("TMPDIR", &self.tmp);
        std::env::set_var("RECOMP_RELATIVE", "1");
        std::env::set_var("RECOMP_JOBS", jobs.to_string());
        // nothing of the developer's own C setup may leak in
        for var in ["C_INCLUDE_PATH", "CPATH", "LIBRARY_PATH", "GCC_EXEC_PREFIX", "COMPILER_PATH", "CFLAGS", "LDFLAGS"] {
            std::env::remove_var(var);
        }
    }
}

/// how many compilers to run at once at most: HAIKU_SETUP_JOBS, else from
/// the cores Haiku may use (compile32: cc.rs starts fewer when the memory
/// free doesn't hold them).
pub fn jobs() -> usize {
    if let Some(n) = std::env::var("HAIKU_SETUP_JOBS").ok().and_then(|v| v.trim().parse::<usize>().ok()).filter(|&n| n > 0) {
        return n;
    }
    jobs_for(crate::cc::cores())
}

/// buildslow (issue #13): all but two of the cores left a 2-core PC one
/// compiler (a Celeron J3355 compiled 255 files one by one, its slowest
/// beside nothing). The compilers run below normal, so what the player
/// does still comes first: a PC with 1-2 cores uses them all, 3-4 all but
/// one, more all but two.
pub fn jobs_for(cores: usize) -> usize {
    match cores {
        0..=2 => cores.max(1),
        3..=4 => cores - 1,
        _ => cores - 2,
    }
}
