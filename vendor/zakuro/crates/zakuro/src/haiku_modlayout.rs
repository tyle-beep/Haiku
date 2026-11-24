//! modfix: mods laid out the ways players get them (GitHub issue #4), which
//! haiku_mods reads besides its own layouts. See dev/reports/modfix.md.
//!
//! - **RomFS folders with no romfs/ around them.** Many mods are a zip of
//!   the game's own folders: Nyx Veil Fix is `bu/` and `bx/`, Expanded
//!   Fates is `GameData/`, `m/`, `castle/`... at its top, Project Re-Fates
//!   keeps its files in a `Mod files/` folder beside a Luma `luma/` folder
//!   and a Citra `citra/unit/` copy. A folder whose subfolders are named as
//!   the game's RomFS's top folders, with game files in them, is a RomFS
//!   root ([`romfs_root`]); [`loose_roots`] finds them near the top of a
//!   mod, never in a title id folder, a `romfs/`, or a folder named for
//!   optional files.
//! - **Title id folders deeper down.** Unofficial Gay Fates is a folder of
//!   readmes, optional files and `Unofficial Gay Fates/<title id>/`, one per
//!   release of the game; [`title_in`] finds the running game's a few
//!   folders down (as `luma/titles/<id>`, `titles/<id>`, `load/mods/<id>`
//!   or `<id>`), the shallowest first.
//! - **Several code patches, one to pick.** Gay Fates' USA folder has
//!   `code.ips` and `old code patch - ....ips`, and a readme saying to pick
//!   one. [`code_choices`] lists them, the default first: `code.ips` (what
//!   Luma3DS loads, and what such readmes mean), else the one the readme
//!   names. The mod's `code_patch` option (mods.toml) picks another.
//! - **A romfs/ a folder or two down** (modlayout, GitHub #16). The Selena
//!   hair mod is a readme, `Mod/romfs/`, `HD Textures (Citra)/` and
//!   `Extras.../`. A mod's folder with nothing of its own and one folder
//!   below holding romfs/ uses that one ([`nested_content`]); optional
//!   folders are never taken, and several plain ones are left to the player
//!   (the Mods screen says so).
//!
//! The loader takes content folders holding `romfs/`, so a mod with loose
//! roots is laid out once, as hard links (a copy where they can't be made),
//! in a folder of the cache ([`lay_out`]): the roots in order, its own
//! `romfs/` last, so that wins.

use std::path::{Path, PathBuf};

/// the folders at the top of the game's RomFS (the same in the USA, EUR and
/// Japanese builds), lower case.
pub const ROMFS_TOP: [&str; 30] = [
    "gamedata", "globalmap", "scripts", "asset", "ba", "bp", "bs", "bu", "bx", "castle", "common", "debug", "effect", "face", "field", "font", "icon", "live2d", "m", "map", "movie", "sample", "scene", "shaders",
    "sky", "sound", "talk", "tut", "ui", "unit",
];
/// and the files there.
pub const ROMFS_TOP_FILES: [&str; 2] = ["params.bin.lz", "texturecoordinate.bin"];

/// folders that are never a loose RomFS root nor looked into for one: what
/// haiku_mods reads itself, and the other releases' and tools' folders.
/// modsweep: and a Mac zip's __MACOSX/ copies
const NOT_ROOTS: [&str; 13] = ["romfs", "romfs_ext", "exefs", "luma", "titles", "load", "textures", "dlc", "exe", "rom", "_downloads", "paragon imports", "__macosx"];

/// how deep below a mod's folder roots and title folders are looked for.
const DEPTH: usize = 2;

/// a title id folder's name: 16 hex digits of an application (00040000...).
pub fn is_title_id(name: &str) -> bool {
    name.len() == 16 && name.to_ascii_lowercase().starts_with("00040000") && u64::from_str_radix(name, 16).is_ok()
}

/// a folder a player has to choose to use (Gay Fates' "Optional Files",
/// "Compatibility Patches"), never taken in by itself.
fn optional(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("optional") || name.contains("compatib") || name.contains("addon") || name.contains("add-on") || name.contains("extras")
}

fn subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut dirs: Vec<(String, PathBuf)> = entries.flatten().filter(|entry| entry.path().is_dir()).map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path())).collect();
    dirs.sort();
    dirs
}

/// whether a folder under dir (a few levels at most) has a file the game
/// could read: anything but pictures and text, which texture packs and
/// readmes are.
fn has_game_file(dir: &Path, depth: usize) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth < 8 && has_game_file(&path, depth + 1) {
                return true;
            }
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        if ![".png", ".dds", ".jpg", ".jpeg", ".txt", ".md", ".json", ".url", ".ini"].iter().any(|ext| name.ends_with(ext)) {
            return true;
        }
    }
    false
}

/// whether dir is the root of a RomFS as the game's is laid out: at least
/// as many of its folders are named as the game's top folders as not, and
/// those hold a file the game could read (a texture pack's folders, also
/// named ui/ or unit/ at times, hold only pictures).
pub fn romfs_root(dir: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(dir) else { return false };
    let (mut known, mut other, mut game_file) = (0, 0, false);
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let path = entry.path();
        if path.is_dir() {
            // a mod's own folder (romfs/, luma/...) is no RomFS itself
            if NOT_ROOTS.contains(&name.as_str()) || is_title_id(&name) {
                return false;
            }
            if ROMFS_TOP.contains(&name.as_str()) {
                known += 1;
                game_file = game_file || has_game_file(&path, 0);
            } else if !name.starts_with('.') {
                other += 1;
            }
        } else if ROMFS_TOP_FILES.contains(&name.as_str()) {
            known += 1;
            game_file = true;
        }
    }
    known > 0 && known >= other && game_file
}

/// the RomFS roots in a mod's folder with no romfs/ around them: the folder
/// itself, or folders up to DEPTH below it, in name order; none inside a
/// root, a title id folder, a romfs/ or what NOT_ROOTS and `optional` name.
pub fn loose_roots(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut level = vec![folder.to_path_buf()];
    for depth in 0..=DEPTH {
        let mut next = Vec::new();
        for dir in level {
            if romfs_root(&dir) {
                found.push(dir);
                continue;
            }
            if depth == DEPTH {
                continue;
            }
            for (name, path) in subdirs(&dir) {
                let lower = name.to_ascii_lowercase();
                if lower.starts_with('.') || NOT_ROOTS.contains(&lower.as_str()) || is_title_id(&name) || optional(&name) {
                    continue;
                }
                next.push(path);
            }
        }
        level = next;
    }
    found
}

fn find_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let exact = dir.join(name);
    if exact.is_dir() {
        return Some(exact);
    }
    std::fs::read_dir(dir).ok()?.flatten().find(|entry| entry.file_name().to_string_lossy().eq_ignore_ascii_case(name) && entry.path().is_dir()).map(|entry| entry.path())
}

/// the title id folder id in a mod: luma/titles/<id>, titles/<id>,
/// load/mods/<id> or <id>, in the folder or up to DEPTH below it (the
/// shallowest first; never in a romfs/, textures/ or optional folder).
pub fn title_in(folder: &Path, id: &str) -> Option<PathBuf> {
    let mut level = vec![folder.to_path_buf()];
    for depth in 0..=DEPTH {
        let mut next = Vec::new();
        for dir in &level {
            let luma = find_ci(dir, "luma").and_then(|luma| find_ci(&luma, "titles")).and_then(|titles| find_ci(&titles, id));
            let titles = || find_ci(dir, "titles").and_then(|titles| find_ci(&titles, id));
            let citra = || find_ci(dir, "load").and_then(|load| find_ci(&load, "mods")).and_then(|mods| find_ci(&mods, id));
            if let Some(found) = luma.or_else(titles).or_else(citra).or_else(|| find_ci(dir, id)) {
                return Some(found);
            }
            if depth < DEPTH {
                for (name, path) in subdirs(dir) {
                    let lower = name.to_ascii_lowercase();
                    if lower.starts_with('.') || NOT_ROOTS.contains(&lower.as_str()) || is_title_id(&name) || optional(&name) || romfs_root(&path) {
                        continue;
                    }
                    next.push(path);
                }
            }
        }
        level = next;
    }
    None
}

/// modlayout (#16): whether a mod's folder holds game files of its own,
/// which haiku_mods takes as they are: romfs/ and the folders beside it, a
/// Luma, Citra or title id folder, Team If's rom/, the game's RomFS folders
/// (here or a little below), or code.
fn has_own_layout(folder: &Path) -> bool {
    const OWN: [&str; 8] = ["romfs", "romfs_ext", "dlc", "exefs", "luma", "titles", "load", "rom"];
    if OWN.iter().any(|name| find_ci(folder, name).is_some()) || subdirs(folder).iter().any(|(name, _)| is_title_id(name)) {
        return true;
    }
    if crate::haiku_fanpatch::stops_unwrap(folder) || romfs_root(folder) || !loose_roots(folder).is_empty() {
        return true;
    }
    std::fs::read_dir(folder).is_ok_and(|entries| {
        entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            entry.path().is_file() && (name.ends_with(".ips") || name.ends_with(".bps") || name == "code.bin" || name == "exheader.bin")
        })
    })
}

/// modlayout: what [`nested_content`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Nested {
    /// the one folder holding a romfs/ (or romfs_ext/), used as the mod's
    /// content folder.
    pub content: Option<PathBuf>,
    /// when none was taken: the romfs/ folders there are, several (none of
    /// them optional), or optional ones only.
    pub left: Vec<PathBuf>,
    /// whether those left are optional ones only.
    pub optional_only: bool,
}

/// modlayout (#16): a mod whose folder holds nothing of its own (see
/// has_own_layout) but a folder one or two folders down holding a romfs/
/// (or a romfs_ext/), as the Selena hair mod's `Mod/romfs/` beside its `HD
/// Textures (Citra)/` and `Extras.../`: that folder is the mod's content
/// folder when it is the only one. Folders named for optional files (see
/// `optional`) are never taken, so with one plain folder and optional ones
/// the plain one is; several plain ones are the player's choice, and none
/// is taken (`left` says which). A Luma or title id folder down there is
/// title_in's, found before this.
pub fn nested_content(folder: &Path) -> Nested {
    if has_own_layout(folder) {
        return Nested::default();
    }
    let (mut plain, mut optionals) = (Vec::new(), Vec::new());
    let mut level: Vec<(PathBuf, bool)> = vec![(folder.to_path_buf(), false)];
    for depth in 1..=DEPTH {
        let mut next = Vec::new();
        for (dir, in_optional) in &level {
            for (name, path) in subdirs(dir) {
                let lower = name.to_ascii_lowercase();
                if lower.starts_with('.') || NOT_ROOTS.contains(&lower.as_str()) || is_title_id(&name) {
                    continue;
                }
                let is_optional = *in_optional || optional(&name);
                let romfs = ["romfs", "romfs_ext"].iter().find_map(|part| find_ci(&path, part).filter(|found| has_game_file(found, 0)));
                if let Some(romfs) = romfs {
                    if is_optional {
                        optionals.push(romfs);
                    } else {
                        plain.push(romfs);
                    }
                    continue;
                }
                if depth < DEPTH {
                    next.push((path, is_optional));
                }
            }
        }
        level = next;
    }
    match plain.as_slice() {
        [only] => Nested { content: only.parent().map(Path::to_path_buf), left: Vec::new(), optional_only: false },
        [] => Nested { content: None, optional_only: !optionals.is_empty(), left: optionals },
        _ => Nested { content: None, left: plain, optional_only: false },
    }
}

/// FNV-1a, for the cache folders' names.
struct Fnv(u64);

impl Fnv {
    fn add(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= *byte as u64;
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
        }
        self.0 ^= 0xFF;
        self.0 = self.0.wrapping_mul(0x0000_0100_0000_01B3);
    }
}

/// the files under dir, / separated, in their own case.
fn walk(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if path.is_dir() {
            if prefix.matches('/').count() < 16 {
                walk(&path, &format!("{prefix}{name}/"), out);
            }
        } else {
            out.push((format!("{prefix}{name}"), path));
        }
    }
}

/// what makes up a laid-out folder, bumped when that changes.
const LAYOUT_VERSION: u32 = 1;

/// the mod's files in one content folder the loader takes, in the cache:
/// romfs/ made of roots in order and then the mod's own content/romfs/
/// (later ones win where they share a path, in any case), with its
/// romfs_ext/ and dlc/ beside. made once for the files as they are (their
/// paths, sizes and times), then used as it is.
pub fn lay_out(id: &str, roots: &[PathBuf], content: &Path, cache: Option<&Path>) -> Result<PathBuf, String> {
    // (where it goes in the folder, the file)
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    let add = |files: &mut Vec<(String, PathBuf)>, dir: &Path, prefix: &str| {
        let mut found = Vec::new();
        walk(dir, "", &mut found);
        files.extend(found.into_iter().map(|(path, from)| (format!("{prefix}{path}"), from)));
    };
    // a root's own folders and files the game has at its top (not its
    // readme)
    for root in roots {
        let mut entries: Vec<_> = std::fs::read_dir(root).into_iter().flatten().flatten().collect();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lower = name.to_ascii_lowercase();
            if entry.path().is_dir() && ROMFS_TOP.contains(&lower.as_str()) {
                add(&mut files, &entry.path(), &format!("romfs/{name}/"));
            } else if ROMFS_TOP_FILES.contains(&lower.as_str()) {
                files.push((format!("romfs/{name}"), entry.path()));
            }
        }
    }
    for part in ["romfs", "romfs_ext", "dlc"] {
        if let Some(dir) = find_ci(content, part) {
            add(&mut files, &dir, &format!("{part}/"));
        }
    }
    // the later file of a path, in any case, wins
    let mut winners: std::collections::BTreeMap<String, (String, PathBuf)> = std::collections::BTreeMap::new();
    for (path, from) in files {
        winners.insert(path.to_lowercase(), (path, from));
    }
    let mut stamp = Fnv(0xcbf2_9ce4_8422_2325);
    stamp.add(&LAYOUT_VERSION.to_le_bytes());
    for (path, from) in winners.values() {
        let meta = std::fs::metadata(from).ok();
        let time = meta.as_ref().and_then(|meta| meta.modified().ok()).and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |time| time.as_nanos() as u64);
        stamp.add(path.as_bytes());
        stamp.add(&meta.map_or(0, |meta| meta.len()).to_le_bytes());
        stamp.add(&time.to_le_bytes());
    }
    let cache = cache.map(Path::to_path_buf).unwrap_or_else(|| std::env::temp_dir().join("haiku-mods")).join("layout");
    let name: String = id.chars().map(|c| if c.is_ascii_alphanumeric() || "-_.".contains(c) { c } else { '_' }).collect();
    let out = cache.join(format!("{name}-{:016x}", stamp.0));
    if out.join(".done").is_file() {
        return Ok(out);
    }
    let started = std::time::Instant::now();
    let _ = std::fs::remove_dir_all(&out);
    let (mut linked, mut copied) = (0, 0);
    for (path, from) in winners.values() {
        let to = out.join(path);
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|error| format!("could not make {}, {error}", parent.display()))?;
        }
        if std::fs::hard_link(from, &to).is_ok() {
            linked += 1;
        } else {
            std::fs::copy(from, &to).map_err(|error| format!("could not lay out {path}, {error}"))?;
            copied += 1;
        }
    }
    std::fs::write(out.join(".done"), format!("{} files of {} roots and {}\n", winners.len(), roots.len(), content.display())).map_err(|error| error.to_string())?;
    log::info!("mods: {id}'s files laid out in {} ({linked} linked, {copied} copied) in {} ms", out.display(), started.elapsed().as_millis());
    // older layouts of the same mod go
    if let Ok(entries) = std::fs::read_dir(&cache) {
        for entry in entries.flatten() {
            let other = entry.file_name().to_string_lossy().into_owned();
            if entry.path() != out && other.strip_prefix(&format!("{name}-")).is_some_and(|rest| rest.len() == 16 && u64::from_str_radix(rest, 16).is_ok()) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    Ok(out)
}

/// the cache's layouts that none of the mods found uses (their mod taken
/// out, or its files changed since).
pub fn prune(cache: &Path, live: &[PathBuf]) {
    let Ok(entries) = std::fs::read_dir(cache.join("layout")) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.join(".done").is_file() && !live.iter().any(|content| content == &path) {
            log::info!("mods: the layout {} goes, no mod uses it now", path.display());
            let _ = std::fs::remove_dir_all(&path);
        }
    }
}

/// modsweep: where a mod's code patch is: its content folder (or exefs/
/// there) when that has one, else the one folder beside its romfs/ named
/// for code that holds code.ips or code.bps (Fates+ 3.5 ships
/// "Code Patch/code.ips", and its readme says to copy it beside romfs/).
/// Several such folders (one per release, say) are a choice for the player
/// to make, so none is taken.
pub fn code_folder(content: &Path) -> PathBuf {
    let patch_in = |dir: &Path| ["code.ips", "code.bps"].iter().any(|name| find_file_ci(dir, name));
    let has_code = |dir: &Path| {
        patch_in(dir) || patch_in(&dir.join("exefs")) || std::fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|entry| {
            let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
            entry.path().is_file() && (name.ends_with(".ips") || name.ends_with(".bps") || name == "code.bin" || name == "exheader.bin")
        }))
    };
    if has_code(content) {
        return content.to_path_buf();
    }
    let found: Vec<PathBuf> = subdirs(content)
        .into_iter()
        .filter(|(name, path)| {
            let lower = name.to_ascii_lowercase();
            lower.contains("code") && !NOT_ROOTS.contains(&lower.as_str()) && !optional(name) && patch_in(path)
        })
        .map(|(_, path)| path)
        .collect();
    match found.as_slice() {
        [only] => {
            log::info!("mods: the code patch in {} is used, as if it were beside romfs/", only.display());
            only.clone()
        }
        _ => content.to_path_buf(),
    }
}

fn find_file_ci(dir: &Path, name: &str) -> bool {
    dir.join(name).is_file() || std::fs::read_dir(dir).is_ok_and(|entries| entries.flatten().any(|entry| entry.file_name().to_string_lossy().eq_ignore_ascii_case(name) && entry.path().is_file()))
}

/// the code patches a mod's code folder (its content folder or exefs/)
/// offers, when it has one besides code.ips and code.bps: the default
/// first, then the others in name order, and why the default is.
/// - code.ips (or code.bps) is the default: Luma3DS loads it, and a mod with
///   others says to rename the one you'd rather have to it;
/// - with neither, the one a readme beside them names, else the first.
#[derive(Debug, Clone, Default)]
pub struct CodeChoices {
    pub patches: Vec<PathBuf>,
    /// the patches code_files found (code.ips, code.bps), which the first
    /// choice stands for when there are any.
    pub standard: Vec<PathBuf>,
    /// the readme saying to pick one, by file name, if there is one.
    pub readme: Option<String>,
    /// the readme names the default.
    pub readme_default: bool,
}

pub fn code_choices(content: &Path) -> Option<CodeChoices> {
    let mut standard: Vec<PathBuf> = Vec::new();
    let mut others: Vec<PathBuf> = Vec::new();
    let mut readmes: Vec<PathBuf> = Vec::new();
    for dir in [content.join("exefs"), content.to_path_buf()] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).filter(|path| path.is_file()).collect();
        entries.sort();
        for path in entries {
            let name = path.file_name().map(|name| name.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
            if name == "code.ips" || name == "code.bps" {
                standard.push(path);
            } else if name.ends_with(".ips") || name.ends_with(".bps") {
                others.push(path);
            } else if name.ends_with(".txt") || name.ends_with(".md") {
                readmes.push(path);
            }
        }
    }
    if others.is_empty() {
        return None;
    }
    let file_name = |path: &PathBuf| path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
    let texts: Vec<(String, String)> = readmes.iter().filter_map(|path| Some((file_name(path), std::fs::read_to_string(path).ok()?.to_ascii_lowercase()))).collect();
    let mentions = |path: &PathBuf| {
        let name = file_name(path).to_ascii_lowercase();
        let stem = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem).to_owned();
        texts.iter().find(|(_, text)| text.contains(&name) || (stem.len() > 6 && text.contains(&stem))).map(|(readme, _)| readme.clone())
    };
    let mut patches: Vec<PathBuf> = Vec::new();
    // code.ips and code.bps together are one choice, as before; the first
    // stands for it
    if let Some(first) = standard.first() {
        patches.push(first.clone());
    } else if let Some(at) = others.iter().position(|path| mentions(path).is_some()) {
        patches.push(others.remove(at));
    }
    patches.extend(others);
    let readme = texts.iter().find(|(_, text)| text.contains("pick") || text.contains("choose") || text.contains("only one") || text.contains("rename")).map(|(name, _)| name.clone()).or_else(|| texts.first().map(|(name, _)| name.clone()));
    let readme_default = patches.first().is_some_and(|path| mentions(path).is_some());
    Some(CodeChoices { patches, standard: Vec::new(), readme, readme_default })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("haiku-modlayout-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn put(dir: &Path, path: &str, data: &[u8]) {
        let path = dir.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }

    #[test]
    fn romfs_roots_are_told_from_texture_packs_and_readmes() {
        let dir = folder("roots");
        put(&dir, "a/bu/uHead_F_x.bch.lz", b"x");
        put(&dir, "a/bx/acc.bch.lz", b"x");
        put(&dir, "b/UI/tex1_8x8_00000000000000AA_13.png", b"png");
        put(&dir, "c/GameData/GameData.bin.lz", b"x");
        put(&dir, "c/Docs/readme.txt", b"x");
        put(&dir, "d/notes/readme.txt", b"x");
        put(&dir, "e/params.bin.lz", b"x");
        assert!(romfs_root(&dir.join("a")));
        assert!(!romfs_root(&dir.join("b")), "a texture pack's ui/ holds pictures only");
        assert!(romfs_root(&dir.join("c")), "as many known folders as others");
        assert!(!romfs_root(&dir.join("d")));
        assert!(romfs_root(&dir.join("e")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn loose_roots_skip_optional_and_title_folders() {
        let dir = folder("loose");
        put(&dir, "Mod files/GameData/GameData.bin.lz", b"x");
        put(&dir, "Mod files/m/@E/GMap.bin.lz", b"x");
        put(&dir, "citra/unit/Head/x.bch.lz", b"x");
        put(&dir, "luma/titles/0004000000179800/romfs/unit/Head/x.bch.lz", b"x");
        put(&dir, "Optional Files/Extra/GameData/GameData.bin.lz", b"x");
        put(&dir, "0004000000179800/GameData/GameData.bin.lz", b"x");
        let roots = loose_roots(&dir);
        assert_eq!(roots, [dir.join("Mod files"), dir.join("citra")]);
        assert_eq!(title_in(&dir, "0004000000179800"), Some(dir.join("luma/titles/0004000000179800")));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_title_folder_a_few_folders_down_is_found() {
        let dir = folder("titles");
        put(&dir, "Pack/0004000000179800/code.ips", b"PATCHEOF");
        put(&dir, "Pack/000400000017A800/code.ips", b"PATCHEOF");
        put(&dir, "Optional Files/Thing/US/0004000000179800/romfs/a.bin", b"x");
        put(&dir, "readme.txt", b"x");
        assert_eq!(title_in(&dir, "0004000000179800"), Some(dir.join("Pack/0004000000179800")));
        assert_eq!(title_in(&dir, "000400000017A800"), Some(dir.join("Pack/000400000017A800")));
        assert_eq!(title_in(&dir, "000400000012DE00"), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn roots_are_laid_out_in_order_with_the_romfs_last() {
        let dir = folder("layout");
        put(&dir, "mod/Mod files/GameData/A.bin.lz", b"loose");
        put(&dir, "mod/Mod files/m/@E/B.bin.lz", b"loose");
        put(&dir, "mod/title/romfs/GameData/a.bin.lz", b"romfs");
        put(&dir, "mod/title/romfs_ext/x.bin.lz.ips", b"PATCHEOF");
        let out = lay_out("my mod", &[dir.join("mod/Mod files")], &dir.join("mod/title"), Some(&dir.join("cache"))).unwrap();
        assert!(out.starts_with(dir.join("cache/layout")));
        assert_eq!(std::fs::read(out.join("romfs/GameData/a.bin.lz")).unwrap(), b"romfs", "the mod's own romfs/ wins, in any case");
        assert!(!out.join("romfs/GameData/A.bin.lz").exists() || cfg!(windows));
        assert_eq!(std::fs::read(out.join("romfs/m/@E/B.bin.lz")).unwrap(), b"loose");
        assert!(out.join("romfs_ext/x.bin.lz.ips").is_file());
        // the same files: the same folder, not made again
        assert_eq!(lay_out("my mod", &[dir.join("mod/Mod files")], &dir.join("mod/title"), Some(&dir.join("cache"))).unwrap(), out);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_folder_of_code_patches_offers_them_code_ips_first() {
        let dir = folder("choices");
        put(&dir, "t/code.ips", b"PATCHEOF");
        put(&dir, "t/old code patch - Fixed Endings.ips", b"PATCHEOF");
        put(&dir, "t/read this - pick one code patch only.txt", b"The file named code.ips is new. Rename the old code patch to code.ips if you prefer it.");
        let choices = code_choices(&dir.join("t")).unwrap();
        let names: Vec<String> = choices.patches.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["code.ips", "old code patch - Fixed Endings.ips"]);
        assert_eq!(choices.readme.as_deref(), Some("read this - pick one code patch only.txt"));
        assert!(choices.readme_default);
        // one code.ips alone is no choice
        put(&dir, "u/code.ips", b"PATCHEOF");
        assert!(code_choices(&dir.join("u")).is_none());
        // without code.ips, the one the readme names
        put(&dir, "v/a-first.ips", b"PATCHEOF");
        put(&dir, "v/b-recommended.ips", b"PATCHEOF");
        put(&dir, "v/readme.txt", b"Use b-recommended.ips unless you want the other.");
        let names: Vec<String> = code_choices(&dir.join("v")).unwrap().patches.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["b-recommended.ips", "a-first.ips"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_code_patch_in_a_folder_of_its_own_is_found() {
        // modsweep: Fates+ 3.5: romfs/ and "Code Patch/code.ips" beside it
        let dir = folder("codefolder");
        put(&dir, "a/romfs/GameData/GameData.bin.lz", b"x");
        put(&dir, "a/Code Patch/code.ips", b"PATCHEOF");
        put(&dir, "a/Gamebanana Screenshots/x.png", b"x");
        assert_eq!(code_folder(&dir.join("a")), dir.join("a").join("Code Patch"));
        // a code patch beside romfs/ wins
        put(&dir, "b/romfs/m/x.bin.lz", b"x");
        put(&dir, "b/code.ips", b"PATCHEOF");
        put(&dir, "b/Code Patch/code.ips", b"PATCHEOF");
        assert_eq!(code_folder(&dir.join("b")), dir.join("b"));
        // one per release: the player's choice, none taken
        put(&dir, "c/romfs/m/x.bin.lz", b"x");
        put(&dir, "c/Code US/code.ips", b"PATCHEOF");
        put(&dir, "c/Code EU/code.ips", b"PATCHEOF");
        assert_eq!(code_folder(&dir.join("c")), dir.join("c"));
        // a folder not named for code is not looked into
        put(&dir, "d/romfs/m/x.bin.lz", b"x");
        put(&dir, "d/Extra/code.ips", b"PATCHEOF");
        assert_eq!(code_folder(&dir.join("d")), dir.join("d"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
