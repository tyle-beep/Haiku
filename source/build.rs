use std::env::var;

#[path = "icon.rs"]
mod icon; // builder
#[path = "hosted_build.rs"]
mod hosted_build; // oneexe

fn main() {
    let dir = var("CARGO_MANIFEST_DIR").unwrap();
    let family = var("CARGO_CFG_TARGET_FAMILY").unwrap_or_default();
    let env = var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    // oneexe: the game's code is no longer linked in. The player's PC
    // recompiles it into a module of its own (data/code/game.dll, made by
    // the setup in this exe), which Haiku.exe loads and binds at start
    // (src/module.rs, src/hosted.rs). Haiku's own overrides are compiled
    // here, into Haiku.exe, with every game address a variable the binding
    // fills for the module's region (hosted_build.rs).
    hosted(&dir);
    // updater: Haiku's version (source/VERSION, which dev/builder/package.sh
    // can set with HAIKU_RELEASE_VERSION), for the updater and the log
    println!("cargo:rerun-if-changed={dir}/VERSION");
    println!("cargo:rerun-if-env-changed=HAIKU_RELEASE_VERSION");
    let version = var("HAIKU_RELEASE_VERSION").ok().filter(|v| !v.trim().is_empty())
        .or_else(|| std::fs::read_to_string(format!("{dir}/VERSION")).ok())
        .map(|v| v.trim().to_owned())
        .unwrap_or_default();
    assert!(!version.is_empty() && version.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)), "source/VERSION must hold Haiku's version, like 1.4 ({version:?})");
    println!("cargo:rustc-env=HAIKU_RELEASE_VERSION={version}");
    // native-final: FATES_LINK_MAP=<file> writes the linker's map, which
    // lists every function linked into Haiku.exe (dev/native-final/linked.py)
    println!("cargo:rerun-if-env-changed=FATES_LINK_MAP");
    if let Ok(map) = var("FATES_LINK_MAP") {
        if env != "msvc" {
            println!("cargo:rustc-link-arg-bins=-Wl,-Map={map}");
        }
    }
    if family == "unix" {
        println!("cargo:rustc-link-lib=m");
    }
    // codemods: the overrides by address and file, for the Mods screen's
    // notes about a code mod changing a function Haiku has its own version of
    let overrides = format!("{dir}/../overrides");
    println!("cargo:rerun-if-changed={overrides}");
    let mut names = String::from("pub const OVERRIDE_NAMES: &[(u32, &str)] = &[\n");
    if let Ok(entries) = std::fs::read_dir(&overrides) {
        let mut paths: Vec<_> = entries.flatten().map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|ext| ext == "c")).collect();
        paths.sort();
        for path in paths {
            let file = path.file_name().unwrap().to_string_lossy().into_owned();
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            for part in text.split("RECOMP_OVERRIDE(").skip(1) {
                let literal = part.split(')').next().unwrap_or("").trim();
                if let Some(hex) = literal.strip_prefix("0x").or_else(|| literal.strip_prefix("0X")) {
                    if let Ok(address) = u32::from_str_radix(hex, 16) {
                        names += &format!("    (0x{address:08X}, {file:?}),\n");
                    }
                }
            }
        }
    }
    names += "];\n";
    let out = var("OUT_DIR").unwrap();
    std::fs::write(format!("{out}/override_names.rs"), names).unwrap();
    // the code runs on the main thread, whose stack is 8 MiB on Linux and
    // 1 MiB on Windows unless it asks for more
    if family == "windows" {
        let stack = 8 << 20;
        match env.as_str() {
            "msvc" => println!("cargo:rustc-link-arg-bins=/STACK:{stack}"),
            _ => println!("cargo:rustc-link-arg-bins=-Wl,--stack,{stack}"),
        }
    }
    // the program's name, Haiku, in its version information (Explorer,
    // Task Manager, the taskbar), compiled with the toolchain's windres
    // builder: the exe's icon is made here from assets/icon/haiku.png
    // (icon.rs), the same picture main.rs gives the window
    println!("cargo:rerun-if-changed=haiku.rc");
    println!("cargo:rerun-if-changed=icon.rs");
    println!("cargo:rerun-if-changed=../assets/icon/haiku.png");
    if family == "windows" && env == "gnu" {
        let out = var("OUT_DIR").unwrap();
        let png = std::path::PathBuf::from(format!("{dir}/../assets/icon/haiku.png"));
        if let Err(error) = icon::write_ico(&png, std::path::Path::new(&format!("{out}/haiku.ico"))) {
            panic!("Haiku's icon: {error}");
        }
        let object = format!("{out}/haiku-rc.o");
        let windres = var("WINDRES").unwrap_or_else(|_| "windres".to_owned());
        let built = std::process::Command::new(&windres)
            .args([&format!("{dir}/haiku.rc"), "--include-dir", &out, "-O", "coff", "-o", &object])
            .status();
        match built {
            Ok(status) if status.success() => println!("cargo:rustc-link-arg-bins={object}"),
            Ok(status) => println!("cargo:warning=windres failed ({status}); Haiku.exe has no version information"),
            Err(error) => println!("cargo:warning=no windres ({error}); Haiku.exe has no version information"),
        }
    }
}

/// oneexe: Haiku's overrides, compiled into Haiku.exe (hosted_build.rs).
fn hosted(dir: &str) {
    let out = std::path::PathBuf::from(var("OUT_DIR").unwrap());
    let overrides = std::path::Path::new(dir).join("..").join("overrides");
    let recomp = std::path::Path::new(dir).join("..").join("vendor").join("3dsrecomp");
    println!("cargo:rerun-if-changed={}", overrides.display());
    let scan = hosted_build::scan(&overrides);
    for (name, _) in &scan.files {
        println!("cargo:rerun-if-changed={}", overrides.join(name).display());
    }
    let ov = out.join("ov");
    let _ = std::fs::remove_dir_all(&ov);
    std::fs::create_dir_all(&ov).unwrap();
    std::fs::copy(recomp.join("abi").join("recomp.h"), ov.join("recomp.h")).unwrap();
    std::fs::write(ov.join("overrides.h"), hosted_build::overrides_h(&scan)).unwrap();
    let mut sources = vec!["haiku_hosted.c".to_owned()];
    std::fs::write(ov.join("haiku_hosted.c"), hosted_build::hosted_c(&scan)).unwrap();
    for (name, text) in &scan.files {
        std::fs::write(ov.join(name), text).unwrap();
        sources.push(name.clone());
    }
    // the flags 3dsrecomp compiles generated code and overrides with
    // (compile.rs FLAGS): no fused multiply-add, as the guest rounds
    const FLAGS: &[&str] = &["-O2", "-ffp-contract=off", "-fno-math-errno", "-w", "-DRECOMP_IMPORTED=1"];
    let mut rerun = Vec::new();
    let recompiler = hosted_build::recompiler_hash(&recomp, &mut rerun);
    for path in rerun {
        println!("cargo:rerun-if-changed={path}");
    }
    std::fs::write(out.join("hosted_interface.rs"), hosted_build::interface_rs(&scan, recompiler, FLAGS)).unwrap();
    println!("cargo:rerun-if-env-changed=HAIKU_CC");
    let cc = var("HAIKU_CC").unwrap_or_else(|_| "gcc".to_owned());
    let jobs: usize = var("NUM_JOBS").ok().and_then(|n| n.parse().ok()).unwrap_or(4).clamp(1, 8);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let failed = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(source) = sources.get(i) else { break };
                let object = ov.join(source).with_extension("o");
                let mut compile = std::process::Command::new(&cc);
                compile.args(FLAGS);
                if cfg!(unix) {
                    // Linux links the exe -pie, so its objects are
                    // position-independent (Windows needs none of that)
                    compile.arg("-fPIC");
                }
                let output = compile
                    .arg(format!("-ffile-prefix-map={}=haiku", ov.display()))
                    .arg("-I")
                    .arg(&ov)
                    .arg("-c")
                    .arg(ov.join(source))
                    .arg("-o")
                    .arg(&object)
                    .output();
                match output {
                    Ok(o) if o.status.success() => {}
                    Ok(o) => failed.lock().unwrap().push(format!("{source}: {}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr))),
                    Err(e) => failed.lock().unwrap().push(format!("{source}: {cc} doesn't run ({e})")),
                }
            });
        }
    });
    let failed = failed.into_inner().unwrap();
    if !failed.is_empty() {
        panic!("compiling Haiku's overrides (oneexe) failed:
{}", failed.join("
"));
    }
    // an archive, linked where librecomp.a was (before the C runtime the
    // overrides call), every object of it kept: the exe reaches the
    // overrides through haiku_hosted.c's tables
    let archive = out.join("libhaikuov.a");
    let _ = std::fs::remove_file(&archive);
    let ar = var("HAIKU_AR").unwrap_or_else(|_| "ar".to_owned());
    let status = std::process::Command::new(&ar)
        .arg("rcs")
        .arg(&archive)
        .args(sources.iter().map(|source| ov.join(source).with_extension("o")))
        .status()
        .unwrap_or_else(|e| panic!("{ar} doesn't run ({e})"));
    assert!(status.success(), "archiving Haiku's overrides failed");
    println!("cargo:rerun-if-env-changed=HAIKU_AR");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static:+whole-archive=haikuov");
}

