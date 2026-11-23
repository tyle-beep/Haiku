//! oneexe: the game's code, recompiled on the player's PC from their own
//! dump into a module of its own (data/code/game.dll on Windows, game.so
//! where Haiku runs natively), loaded and bound to
//! Haiku.exe at start.
//!
//! Haiku.exe holds the engine and Haiku's own overrides, compiled when
//! Haiku is built (build.rs, hosted_build.rs). The module holds the game's
//! code as the recompiler wrote it, with a small function in each
//! override's place that goes on through a slot (3dsrecomp hosted.rs).
//! Binding, before any of the game's code runs:
//!
//! - each override's slot gets Haiku.exe's override, found by its address
//!   (the overrides are written against the USA build: an EUR or JPN
//!   module's addresses are the region's, through the region's map,
//!   haiku_setup::regions);
//! - each override's original (the module's generated function, for
//!   RECOMP_ORIGINAL) goes to `haiku_originals`;
//! - each game address the overrides use (`haiku_ga_*`) gets the region's;
//! - each generated function they call by name (`haiku_gf_*`) the module's;
//! - the overrides' CALL_THEN tails reach the module's tails registry
//!   (`recomp_imports`);
//! - the amiibo icons, made from the dump, are copied from the module.
//!
//! A module is only used when it says it was built for this Haiku.exe:
//! its stamp (haiku_setup::module::stamp) names the recompiler's sources,
//! the overrides' addresses and named functions, the hosted tables' layout
//! and the region map; data/code/game.txt says it too, read before the
//! module is loaded. Anything else (no module, another stamp, a module of
//! another game than data/ holds) has the setup build it again.

use std::ffi::c_void;
use std::path::{Path, PathBuf};

use haiku_setup::module::{self as spec_module, Spec};

include!(concat!(env!("OUT_DIR"), "/hosted_interface.rs"));

/// what build.rs found in the overrides, for the setup's module build and
/// the stamp
pub fn spec() -> Spec {
    Spec { overrides: OVERRIDES, functions: FUNCTIONS, recompiler: RECOMPILER, interface: INTERFACE }
}

#[repr(C)]
struct HostedOverride {
    address: u32,
    code: haiku_setup::recomp::abi::Code,
}

#[repr(C)]
struct HostedAddress {
    address: u32,
    at: *mut u32,
}

#[repr(C)]
struct HostedFunction {
    address: u32,
    at: *mut Option<haiku_setup::recomp::abi::Code>,
}

/// recomp.h's RecompImports (RECOMP_IMPORTED), the tails registry
#[repr(C)]
#[derive(Clone, Copy)]
struct Imports {
    tails_pending: *mut u32,
    tails_counting: *mut u8,
    tail_defer: *const c_void,
    tails_due: *const c_void,
    tail_count: *const c_void,
}

// the module's tables (3dsrecomp hosted.rs)
#[repr(C)]
struct ModuleOverride {
    module: *const std::ffi::c_char,
    address: u32,
    slot: *mut Option<haiku_setup::recomp::abi::Code>,
    original: Option<haiku_setup::recomp::abi::Code>,
}

#[repr(C)]
struct ModuleFunction {
    address: u32,
    code: Option<haiku_setup::recomp::abi::Code>,
}

extern "C" {
    // haiku_hosted.c (hosted_build.rs)
    static haiku_hosted_override_count: u32;
    static haiku_hosted_overrides: [HostedOverride; 1];
    static haiku_hosted_address_count: u32;
    static haiku_hosted_addresses: [HostedAddress; 1];
    static haiku_hosted_function_count: u32;
    static haiku_hosted_functions: [HostedFunction; 1];
    static mut haiku_originals: [Option<haiku_setup::recomp::abi::Code>; 1];
    static mut recomp_imports: Imports;
    static mut amiibo_icons: [[u16; 2304]; 4];
    // eur_region.c: the title the code is made from
    static mut haiku_code_title: u64;
}

/// the folder the module and its note are in
pub fn dir(root: &Path) -> PathBuf {
    spec_module::dir(root)
}

/// where the module is: HAIKU_GAME_MODULE (a test or a bench root, which
/// has the exe alone), else data/code/game.dll on Windows, game.so natively
pub fn find(root: &Path) -> PathBuf {
    match std::env::var_os("HAIKU_GAME_MODULE").map(PathBuf::from) {
        Some(path) if path.is_dir() => path.join(spec_module::file()),
        Some(path) => path,
        None => dir(root).join(spec_module::file()),
    }
}

/// a module loaded and bound, its tables for the runtime
pub struct Bound {
    pub title: u64,
    pub path: PathBuf,
    pub linked: zakuro::Linked,
    pub tails_forget: unsafe extern "C" fn(u32),
    pub tails_keep: unsafe extern "C" fn(*mut zakuro_core::state::keep::Tail, u32) -> u32,
    pub tails_put: unsafe extern "C" fn(*const zakuro_core::state::keep::Tail, u32),
    pub imports: zakuro_core::codemods::Imports,
}

#[cfg(windows)]
mod sys {
    use std::ffi::c_void;
    #[link(name = "kernel32")]
    extern "system" {
        pub fn LoadLibraryExW(name: *const u16, file: *mut c_void, flags: u32) -> *mut c_void;
        pub fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
        pub fn SetErrorMode(mode: u32) -> u32;
        pub fn GetLastError() -> u32;
    }
    pub const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x0000_0008;
    pub const SEM_FAILCRITICALERRORS: u32 = 0x0001;
    pub const SEM_NOOPENFILEERRORBOX: u32 = 0x8000;
}

#[cfg(windows)]
struct Library(*mut c_void);

#[cfg(not(windows))]
struct Library(libloading::Library);

impl Library {
    #[cfg(windows)]
    fn open(path: &Path) -> Result<Library, String> {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: a NUL-terminated path; no dialog if it can't be loaded
        unsafe {
            let before = sys::SetErrorMode(sys::SEM_FAILCRITICALERRORS | sys::SEM_NOOPENFILEERRORBOX);
            let handle = sys::LoadLibraryExW(wide.as_ptr(), std::ptr::null_mut(), sys::LOAD_WITH_ALTERED_SEARCH_PATH);
            let error = sys::GetLastError();
            sys::SetErrorMode(before);
            if handle.is_null() {
                return Err(format!("{} can't be loaded (error {error})", path.display()));
            }
            Ok(Library(handle))
        }
    }

    #[cfg(not(windows))]
    fn open(path: &Path) -> Result<Library, String> {
        // SAFETY: the module is our own compiled code
        unsafe { libloading::Library::new(path) }.map(Library).map_err(|e| format!("{} can't be loaded ({e})", path.display()))
    }

    fn symbol(&self, name: &str) -> Result<*mut c_void, String> {
        #[cfg(windows)]
        {
            let c = format!("{name}\0");
            // SAFETY: our module, a NUL-terminated name
            let at = unsafe { sys::GetProcAddress(self.0, c.as_ptr()) };
            if at.is_null() {
                return Err(format!("the game's module has no {name}"));
            }
            Ok(at)
        }
        #[cfg(not(windows))]
        {
            let raw = format!("{name}\0");
            // SAFETY: our module, whose symbols the caller checks
            unsafe { self.0.get::<*mut c_void>(raw.as_bytes()) }.map(|s| *s).map_err(|e| format!("the game's module has no {name} ({e})"))
        }
    }
}

/// updscreen: the module at path loaded (not bound) on the calling thread,
/// ahead of `load`, which then finds it loaded: a module just written is
/// read whole (and scanned) on its first load, which can take seconds the
/// window's thread shouldn't wait for.
pub fn preload(path: &Path) -> Result<(), String> {
    Library::open(path).map(|_| ())
}

/// the module at path, checked against this exe and the game, loaded and
/// bound. The library stays loaded for the life of the process.
pub fn load(path: &Path, data_title: Option<u64>) -> Result<Bound, String> {
    let spec = spec();
    // the note first: a module of another stamp is never loaded at all
    let note = spec_module::Note::read(&path.with_file_name(spec_module::NOTE)).ok_or_else(|| format!("{} has no note beside it", path.display()))?;
    let region = haiku_setup::regions::Region::of(note.title).ok_or_else(|| format!("the module is of an unknown game ({:016X})", note.title))?;
    let stamp = spec.stamp(region);
    if note.stamp != stamp {
        return Err(format!("the module was built for another Haiku ({}), this one needs {stamp}", note.stamp));
    }
    if let Some(title) = data_title.filter(|&t| t != note.title) {
        return Err(format!("the module is of {:016X}, data/ holds {title:016X}", note.title));
    }
    let started = std::time::Instant::now();
    let library = Library::open(path)?;
    let get = |name: &str| library.symbol(name);
    // SAFETY: the symbols of a module 3dsrecomp generated with hosted
    // overrides, whose layout recomp_hosted_version names
    unsafe {
        let version = *(get("recomp_hosted_version")? as *const u32);
        if version != haiku_setup::recomp::hosted::VERSION {
            return Err(format!("the module's tables are version {version}, this Haiku reads {}", haiku_setup::recomp::hosted::VERSION));
        }
        let built = std::ffi::CStr::from_ptr(get("recomp_hosted_stamp")? as *const std::ffi::c_char).to_string_lossy().into_owned();
        if built != stamp {
            return Err(format!("the module says it was built for {built}, this Haiku needs {stamp}"));
        }
        let title = *(get("recomp_hosted_title")? as *const u64);
        if title != note.title {
            return Err(format!("the module is of {title:016X}, its note says {:016X}", note.title));
        }
        let map = haiku_setup::regions::map(region)?;
        let to_region = |us: u32| -> Result<u32, String> {
            match &map {
                None => Ok(us),
                Some(map) => map.get(&us).copied().ok_or_else(|| format!("0x{us:08X} has no {} address in Haiku's map", region.tag())),
            }
        };
        // the overrides, into their slots
        let count = *(get("recomp_hosted_override_count")? as *const u32) as usize;
        let rows = std::slice::from_raw_parts(get("recomp_hosted_overrides")? as *const ModuleOverride, count);
        let mut by_address = std::collections::HashMap::with_capacity(count);
        for row in rows {
            if row.module.is_null() {
                by_address.insert(row.address, row);
            }
        }
        let ours = std::slice::from_raw_parts(std::ptr::addr_of!(haiku_hosted_overrides).cast::<HostedOverride>(), haiku_hosted_override_count as usize);
        let originals = std::ptr::addr_of_mut!(haiku_originals).cast::<Option<haiku_setup::recomp::abi::Code>>();
        let mut bound = 0;
        for (k, item) in ours.iter().enumerate() {
            let at = to_region(item.address)?;
            let row = by_address.remove(&at).ok_or_else(|| format!("the module has no place for the override of 0x{:08X} (0x{at:08X})", item.address))?;
            *row.slot = Some(item.code);
            *originals.add(k) = Some(row.original.unwrap_or(no_original));
            bound += 1;
        }
        if let Some(address) = by_address.keys().next() {
            return Err(format!("the module has a place for an override at 0x{address:08X} this Haiku doesn't have"));
        }
        // the game's addresses the overrides use
        let addresses = std::slice::from_raw_parts(std::ptr::addr_of!(haiku_hosted_addresses).cast::<HostedAddress>(), haiku_hosted_address_count as usize);
        for item in addresses {
            *item.at = to_region(item.address)?;
        }
        // the generated functions they call by name
        let count = *(get("recomp_hosted_function_count")? as *const u32) as usize;
        let rows = std::slice::from_raw_parts(get("recomp_hosted_functions")? as *const ModuleFunction, count);
        let functions = std::slice::from_raw_parts(std::ptr::addr_of!(haiku_hosted_functions).cast::<HostedFunction>(), haiku_hosted_function_count as usize);
        for item in functions {
            let at = to_region(item.address)?;
            let code = rows.iter().find(|row| row.address == at).and_then(|row| row.code).ok_or_else(|| format!("the module has no function at 0x{at:08X} (0x{:08X})", item.address))?;
            *item.at = Some(code);
        }
        // the tails registry
        let imports = *(get("recomp_hosted_imports")? as *const Imports);
        std::ptr::write(std::ptr::addr_of_mut!(recomp_imports), imports);
        // the amiibo icons, made from the dump
        if let Ok(icons) = get("haiku_module_amiibo_icons") {
            let icons = *(icons as *const *const u16);
            if !icons.is_null() {
                std::ptr::copy_nonoverlapping(icons, std::ptr::addr_of_mut!(amiibo_icons).cast::<u16>(), 4 * 2304);
            }
        }
        std::ptr::write(std::ptr::addr_of_mut!(haiku_code_title), title);

        let linked = zakuro::Linked::new(
            title,
            *(get("recomp_abi")? as *const u32),
            get("recomp_entries")? as *const c_void,
            *(get("recomp_entry_count")? as *const u32),
            get("recomp_modules")? as *const c_void,
            *(get("recomp_module_count")? as *const u32),
        )
        .with_origins(*(get("recomp_generation")? as *const u32), get("recomp_origins")? as *const c_void);
        let codemods = zakuro_core::codemods::Imports {
            tails_pending: imports.tails_pending,
            tails_counting: imports.tails_counting,
            tail_defer: std::mem::transmute::<*const c_void, _>(imports.tail_defer),
            tails_due: std::mem::transmute::<*const c_void, _>(imports.tails_due),
            tail_count: std::mem::transmute::<*const c_void, _>(imports.tail_count),
        };
        let result = Bound {
            title,
            path: path.to_owned(),
            linked,
            tails_forget: std::mem::transmute::<*mut c_void, _>(get("recomp_tails_forget")?),
            tails_keep: std::mem::transmute::<*mut c_void, _>(get("recomp_tails_keep")?),
            tails_put: std::mem::transmute::<*mut c_void, _>(get("recomp_tails_put")?),
            imports: codemods,
        };
        // the module stays loaded with the process
        std::mem::forget(library);
        eprintln!(
            "oneexe: the game's code from {} ({:016X}, {}), {bound} overrides, {} addresses, {} functions bound in {} ms",
            path.display(),
            title,
            region.tag(),
            addresses.len(),
            functions.len(),
            started.elapsed().as_millis()
        );
        Ok(result)
    }
}

/// an original the module has no generated code for (the static build
/// would not have compiled a call to it)
unsafe extern "C" fn no_original(ctx: *mut haiku_setup::recomp::abi::Context) {
    // SAFETY: the context the code was called with
    let at = unsafe { (*ctx).r[15] };
    panic!("an override called its original at {at:08X}, which the game's module has no code for");
}
