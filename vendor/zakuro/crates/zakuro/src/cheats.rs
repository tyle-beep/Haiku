//! cheats: the Cheats tab of the PC Options menu (fates_ui/cheats.rs draws
//! it), and the host side of overrides/cheats.c.
//!
//! Everything a cheat reads comes from the running game's own memory: the
//! army (UnitPool, the forces), the class, item, person, skill and chapter
//! tables the game loaded, and the names from the game's own English message
//! file. A change is made one of two ways:
//!
//! - **in the game's own functions** (overrides/cheats.c, at the top of
//!   Proc::Exec on the game's thread): a class change (Unit::ClassChange, as
//!   a Heart Seal does), a character joining (My Castle's join:
//!   Unit::CreateForCastleJoin, Transfer to the army, the Unit Logbook), a
//!   support level (Unit::SetRelianceLevel both ways), level-ups
//!   (Unit::LevelUp, the game's own rolls), the convoy (Transporter::Add),
//!   a chapter (Next Battle's way: GameUserData +0xC, MainSeq::SetResult);
//! - **in the unit's own saved fields**, between two frames, where the game
//!   has no function for it: a stat's gain (Unit +0xC8), its cap offset
//!   (+0xD0/+0xD8, what the castle's statues use), level (+0xF1), EXP
//!   (+0xF2), HP (+0xF3), Mov (+0x12C), weapon experience (+0xE8), skills
//!   (+0xFE and the learned pool at +0x3F), gold (GameUserData +0x38).
//!
//! Switches (battle, growth, movement, the Support Log...) are kept in the
//! settings (`[pc.cheats]`) and written into overrides/cheats.c's words.
//! With every switch off and no action taken, nothing here writes to the
//! game: every override runs the game's own code alone.
//!
//! online2: a unit a cheat changes (class, stats, level, EXP, Mov, weapon
//! ranks, skills, supports, an item, a join; and with the switches, a
//! level-up with perfect growths, EXP gained with a multiplier, a captured
//! unit that joins) gets a mark nobody sees, for online battles only: bit 31
//! of its state's upper word (Unit +0xC), which the game neither reads nor
//! clears and saves with the unit. It stays once set. See overrides/online.c
//! and dev/reports/online2.md.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};
use zakuro_core::memory::Memory;
use zakuro_core::System;

/// indices into overrides/cheats.c's `cheats_words`.
pub mod word {
    pub const FLAGS: usize = 0;
    pub const CRIT: usize = 1;
    pub const EXP_MULT: usize = 2;
    pub const MOVE: usize = 3;
    pub const REQ: usize = 4;
    pub const ARG: usize = 5;
    pub const RESULT: usize = 13;
    pub const DONE: usize = 14;
    pub const RUNS: usize = 15;
    pub const VIEW_UNIT: usize = 17;
    pub const VIEW_TICK: usize = 18;
    pub const STATS: usize = 20;
    pub const SEALS: usize = 40;
    pub const SKILL_COUNT: usize = 48;
    /// cheats2: the two Capture skills' ids (49, 50)
    pub const CAPTURE_SKILL: usize = 49;
    /// cheats2: counters (52: Capture target searches)
    pub const STATS2: usize = 52;
    pub const SKILLS: usize = 64;
    pub const VIEW: usize = 300;
    pub const ITEMS: usize = 512;
    pub const COUNT: usize = 1024;
}

pub mod flag {
    pub const HIT: u32 = 0x1;
    pub const MISS: u32 = 0x2;
    pub const GOD: u32 = 0x4;
    pub const OHKO: u32 = 0x8;
    pub const MOVE_AGAIN: u32 = 0x10;
    pub const MOVE: u32 = 0x20;
    pub const GROWTH: u32 = 0x40;
    pub const WEAPONS: u32 = 0x80;
    pub const VEINS: u32 = 0x100;
    pub const LOG: u32 = 0x200;
    pub const SEALS: u32 = 0x400;
    pub const TURN: u32 = 0x800;
    pub const EXP: u32 = 0x1000;
    /// cheats2: a Dragon Vein used is ready again at the next player phase
    pub const VEIN_REUSE: u32 = 0x2000;
    /// cheats2: the player's units can Capture, and a captured unit joins at once
    pub const CAPTURE: u32 = 0x4000;
}

pub mod op {
    pub const CLASS: u32 = 1;
    pub const JOIN: u32 = 2;
    pub const JOIN_GUEST: u32 = 3;
    pub const SUPPORT: u32 = 4;
    pub const LEVEL_UP: u32 = 5;
    pub const CONVOY: u32 = 6;
    pub const CHAPTER: u32 = 7;
    pub const ICON: u32 = 8;
    /// cheats2: one building's level-up, through My Castle's own sequence
    pub const BUILDING: u32 = 10;
    /// cheats2, tests: an item into a unit's hands
    pub const ITEM: u32 = 11;
}

/// the switches, saved in the settings as `[pc.cheats]`. All off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CheatSettings {
    /// 0 off, 1 the player's units, 2 everyone
    pub crit: u8,
    pub always_hit: bool,
    pub enemies_miss: bool,
    pub no_damage: bool,
    pub one_hit_kill: bool,
    pub move_again: bool,
    pub unlimited_move: bool,
    pub perfect_growth: bool,
    pub every_weapon: bool,
    pub dragon_veins: bool,
    pub support_log: bool,
    pub unlimited_seals: bool,
    pub freeze_turns: bool,
    /// 1, 2, 5 or 10 (0 reads as 1)
    pub exp_multiplier: u32,
    pub ignore_caps: bool,
    /// extra skills beyond the five: "PID|skill id" (by the person's PID,
    /// so they follow the character from save to save)
    pub extra_skills: Vec<String>,
    /// cheats2: Dragon Veins can be used again, once each player phase
    pub vein_reuse: bool,
    /// cheats2: Capture without the Prison; captured units join the army
    pub capture_recruit: bool,
}

impl CheatSettings {
    pub fn flags(&self) -> u32 {
        let mut f = 0;
        let on = |b: bool, bit: u32| if b { bit } else { 0 };
        f |= on(self.always_hit, flag::HIT);
        f |= on(self.enemies_miss, flag::MISS);
        f |= on(self.no_damage, flag::GOD);
        f |= on(self.one_hit_kill, flag::OHKO);
        f |= on(self.move_again, flag::MOVE_AGAIN);
        f |= on(self.unlimited_move, flag::MOVE);
        f |= on(self.perfect_growth, flag::GROWTH);
        f |= on(self.every_weapon, flag::WEAPONS);
        f |= on(self.dragon_veins, flag::VEINS);
        f |= on(self.support_log, flag::LOG);
        f |= on(self.unlimited_seals, flag::SEALS);
        f |= on(self.freeze_turns, flag::TURN);
        f |= on(self.exp_multiplier > 1, flag::EXP);
        f |= on(self.vein_reuse, flag::VEIN_REUSE);
        f |= on(self.capture_recruit, flag::CAPTURE);
        f
    }

    pub fn any(&self) -> bool {
        self.flags() != 0 || self.crit != 0 || !self.extra_skills.is_empty()
    }
}

#[derive(Clone, Copy)]
pub struct Hooks {
    pub words: fn() -> *mut u32,
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();

pub fn set_hooks(hooks: Hooks) {
    let _ = HOOKS.set(hooks);
}

pub fn read(index: usize) -> u32 {
    let Some(hooks) = HOOKS.get() else { return 0 };
    if index >= word::COUNT {
        return 0;
    }
    // SAFETY: a word of the game build's array; a u32 is never torn
    unsafe { std::ptr::read_volatile((hooks.words)().add(index)) }
}

pub fn write(index: usize, value: u32) {
    if let Some(hooks) = HOOKS.get() {
        if index < word::COUNT {
            // SAFETY: as above
            unsafe { std::ptr::write_volatile((hooks.words)().add(index), value) }
        }
    }
}

pub fn available() -> bool {
    HOOKS.get().is_some()
}

// ---- game addresses (USA Special Edition v2) --------------------------------

const UNIT_POOL: u32 = 0x006D_9F40;
/// online2: the mark, in the unit's state word's upper half (Unit +0xC)
pub const CHEATED_WORD: u32 = 0x0C;
pub const CHEATED_BIT: u32 = 0x8000_0000;
const FORCES: u32 = 0x006D_9F44;
const UNIT_SIZE: u32 = 0x140;
const GAME_USER: u32 = 0x006D_A248;
const SITUATION: u32 = 0x006D_7ED0;
const CASTLE_MAIN: u32 = 0x006D_1B70;
const FREE_MOVE: u32 = 0x006D_2A10;
const PERSON_LIST: u32 = 0x006D_3B28;
const JOB_LIST: u32 = 0x006D_3990 + 0x10;
const ITEM_LIST: u32 = 0x006D_38D0;
const SKILLS: u32 = 0x006D_3C5C;
const CHAPTERS: u32 = 0x006D_379C;
const TRANSPORTER: u32 = 0x006D_95E8;
/// castlegifts: AchieveBonus's lists (AchieveBonus::Initialize): +4 the
/// Battle Bonuses', +8 the Visit Bonuses', each {entries, count} of
/// {threshold, item id}
const ACHIEVE_BONUS: u32 = 0x006D_363C;
const CASTLE: u32 = 0x006D_1998; // Castle*: +8 -> the CastleUserData*
const GEMSTONES: u32 = 0x006D_1F50; // castle::Gemstone: +4 -> the count
const FOODS: u32 = 0x006D_1F18; // castle::Food: +4 -> the count

pub const STAT_NAMES: [&str; 8] = ["HP", "Str", "Mag", "Skl", "Spd", "Lck", "Def", "Res"];
pub const WEAPON_NAMES: [&str; 8] = ["Sword", "Lance", "Axe", "Dagger", "Bow", "Tome", "Staff", "Stone"];
/// a weapon type's S rank, in weapon experience (the game's caps are 0x60 C, 0xA1 B... 0xFB S)
pub const RANK_S: u8 = 0xFB;

fn r32(m: &mut Memory, a: u32) -> u32 {
    if a == 0 || !m.is_readable(a, 4) {
        return 0;
    }
    let mut b = [0u8; 4];
    m.read_bytes(a, &mut b);
    u32::from_le_bytes(b)
}
/// the word a pointer word points at
fn r32p(m: &mut Memory, a: u32) -> u32 {
    let p = r32(m, a);
    r32(m, p)
}
fn r16(m: &mut Memory, a: u32) -> u16 {
    if a == 0 || !m.is_readable(a, 2) {
        return 0;
    }
    let mut b = [0u8; 2];
    m.read_bytes(a, &mut b);
    u16::from_le_bytes(b)
}
fn r8(m: &mut Memory, a: u32) -> u8 {
    if a == 0 || !m.is_readable(a, 1) {
        return 0;
    }
    let mut b = [0u8; 1];
    m.read_bytes(a, &mut b);
    b[0]
}
fn w8(m: &mut Memory, a: u32, v: u8) {
    if a != 0 && m.is_readable(a, 1) {
        m.write_bytes(a, &[v]);
    }
}
fn w16(m: &mut Memory, a: u32, v: u16) {
    if a != 0 && m.is_readable(a, 2) {
        m.write_bytes(a, &v.to_le_bytes());
    }
}
fn w32(m: &mut Memory, a: u32, v: u32) {
    if a != 0 && m.is_readable(a, 4) {
        m.write_bytes(a, &v.to_le_bytes());
    }
}
/// the string a pointer field points at
fn cstr_at(m: &mut Memory, field: u32) -> Vec<u8> {
    let a = r32(m, field);
    cstr(m, a)
}

fn cstr(m: &mut Memory, a: u32) -> Vec<u8> {
    let mut out = Vec::new();
    if a == 0 {
        return out;
    }
    for i in 0..96 {
        let b = r8(m, a + i);
        if b == 0 {
            break;
        }
        out.push(b);
    }
    out
}

// ---- what the menu shows ------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct UnitView {
    pub addr: u32,
    pub person: u32,
    pub pid: Vec<u8>,
    pub person_id: u16,
    pub name: String,
    pub female: bool,
    pub job: u32,
    pub class_id: u16,
    pub class_name: String,
    pub level: u8,
    pub exp: u8,
    pub hp: u8,
    pub force: u8,
    pub wexp: [u8; 8],
    pub skills: [u16; 5],
    /// online2: the cheats changed it (never shown in the menu)
    pub marked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct ClassEntry {
    pub id: u16,
    pub addr: u32,
    pub name: String,
    pub female: bool,
    pub other: u16,
    pub promoted: bool,
    pub wexp_max: [u8; 8],
}

#[derive(Clone, Debug, Default)]
pub struct PersonEntry {
    pub id: u16,
    pub addr: u32,
    pub pid: Vec<u8>,
    pub name: String,
    pub female: bool,
    /// a unique guest (amiibo heroes, DLC characters): joins as their own scripts do
    pub guest: bool,
    pub child: bool,
}

#[derive(Clone, Debug, Default)]
pub struct SkillEntry {
    pub id: u16,
    pub name: String,
}

#[derive(Clone, Debug, Default)]
pub struct ChapterEntry {
    pub addr: u32,
    pub cid: String,
    pub name: String,
    pub prefix: String,
    /// the route it belongs to (0 Birthright, 1 Conquest, 2 Revelation), or 0xFF for more
    pub route: u8,
    /// the routes it is on (bit 0 Birthright, 1 Conquest, 2 Revelation; 0 before the choice)
    pub routes: u8,
    pub kind: u8,
}

/// the stats of the chosen unit, as the game computes them (cheats.c)
#[derive(Clone, Copy, Debug, Default)]
pub struct StatView {
    pub unit: u32,
    pub stats: [i32; 8],
    pub caps: [i32; 8],
    pub mov: i32,
    pub level_cap: i32,
    pub max_hp: i32,
}

/// a one-shot action the menu asked for
#[derive(Clone, Debug)]
pub enum Action {
    ChangeClass { unit: u32, class: u16 },
    SetStat { unit: u32, stat: usize, value: i32 },
    SetMove { unit: u32, value: i32 },
    SetLevel { unit: u32, value: i32 },
    SetExp { unit: u32, value: i32 },
    MaxLevel { unit: u32 },
    MaxRanks { unit: u32 },
    GiveSkill { unit: u32, skill: u16, slot: usize },
    AddCharacter { person: u16 },
    SetSupport { a: u32, b: u32, level: u32 },
    MaxGold,
    FillConvoy,
    MaxCastle,
    /// cheats2: every building to its highest level, one by one
    MaxBuildings,
    /// castlegifts: one more of an item in the convoy
    GiveItem { item: u16 },
    Chapter { index: usize },
}

#[derive(Default)]
pub struct State {
    pub loaded: bool,
    pub english: HashMap<Vec<u8>, String>,
    pub classes: Vec<ClassEntry>,
    pub persons: Vec<PersonEntry>,
    pub skills: Vec<SkillEntry>,
    pub chapters: Vec<ChapterEntry>,
    pub items: Vec<(u16, String)>,
    /// castlegifts: the items My Castle's Inbox > Rewards gives, and how
    /// ("Battle Bonus", "Visit Bonus")
    pub bonus_items: HashMap<u16, String>,
    pub seals: Vec<u16>,
    /// cheats2: the Capture skills (SEID_白夜の捕縛術, SEID_暗夜の拘束術)
    pub capture_skills: Vec<u16>,
    /// cheats2: Max Buildings under way
    buildings: Option<BuildingRun>,
    pub roster: Vec<UnitView>,
    pub view: StatView,
    /// the unit the menu has chosen (an address in the unit pool)
    pub chosen: u32,
    pub menu_open: bool,
    pub in_game: bool,
    pub in_castle: bool,
    pub on_map: bool,
    pub gold: u32,
    pub route: u8,
    pending: VecDeque<Action>,
    /// a request handed to cheats.c and what to say once it is done
    waiting: Option<(u32, String)>,
    /// online2: the units the request changes, marked once it is done, and
    /// the person a join brings (its unit is marked once it is there)
    mark_after: Vec<u32>,
    join_after: Option<u32>,
    /// messages for the menu's toast
    pub notices: VecDeque<String>,
    frame: u64,
    settings: CheatSettings,
}

/// cheats2: Max Buildings, one building's level-up after another
#[derive(Default, Clone)]
struct BuildingRun {
    /// the Dragon Vein Points before, given back at the end (the cheat costs nothing)
    points: u32,
    upgraded: u32,
    /// frames since the last level-up sequence ended
    idle: u32,
    /// how often each building was asked for, to give up on one that does not change
    asked: HashMap<u16, u32>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

pub fn with_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(State::default))
}

/// the menu asks for an action; it runs on the next frames.
pub fn request(action: Action) {
    with_state(|s| s.pending.push_back(action));
}

/// puts the switches to use, at start and after each change.
pub fn apply(pc: &crate::settings::PcSettings) {
    if !available() {
        return;
    }
    let mut c = pc.cheats.clone();
    // tests: FATES_CHEATS=crit,hit,miss,god,ohko,again,move,growth,weapons,veins,log,seals,turn,exp10
    if let Ok(list) = std::env::var("FATES_CHEATS") {
        for item in list.split(',').map(str::trim) {
            match item {
                "crit" => c.crit = 1,
                "crit-all" => c.crit = 2,
                "hit" => c.always_hit = true,
                "miss" => c.enemies_miss = true,
                "god" => c.no_damage = true,
                "ohko" => c.one_hit_kill = true,
                "again" => c.move_again = true,
                "move" => c.unlimited_move = true,
                "growth" => c.perfect_growth = true,
                "weapons" => c.every_weapon = true,
                "veins" => c.dragon_veins = true,
                "log" => c.support_log = true,
                "seals" => c.unlimited_seals = true,
                "turn" => c.freeze_turns = true,
                "caps" => c.ignore_caps = true,
                "reuse" => c.vein_reuse = true,
                "capture" => c.capture_recruit = true,
                other => {
                    if let Some(m) = other.strip_prefix("exp").and_then(|v| v.parse().ok()) {
                        c.exp_multiplier = m;
                    }
                }
            }
        }
    }
    write_switches(&c);
    write(word::MOVE, 30);
    with_state(|s| {
        s.settings = c;
        if s.loaded {
            extra_skill_words(s);
        }
    });
}

/// the switches the game reads; all off while an online battle is up.
fn write_switches(c: &CheatSettings) {
    // online: both consoles must work out the same battle from the same
    // moves, so a cheat that changes the battle's numbers would part them:
    // the switches are off while the game's (online) Local Battle is up
    if zakuro_core::netplay::battle() {
        write(word::FLAGS, 0);
        write(word::CRIT, 0);
        write(word::EXP_MULT, 100);
        // online2: and the extra skills (Unit::IsEquipSkill answers them on
        // this console only)
        write(word::SKILL_COUNT, 0);
        return;
    }
    write(word::FLAGS, c.flags());
    write(word::CRIT, c.crit.min(2) as u32);
    write(word::EXP_MULT, c.exp_multiplier.max(1) * 100);
}

/// online: the Local Battle began or ended: the switches off, or back.
pub fn online_changed(on: bool) {
    if !available() {
        return;
    }
    let c = with_state(|s| s.settings.clone());
    log::info!("cheats: {} for the online battle", if on { "off" } else { "back on" });
    write_switches(&c);
    // online2: the extra skills come back with the rest
    if !on {
        with_state(|s| {
            if s.loaded {
                extra_skill_words(s);
            }
        });
    }
}

/// online2: marks a unit a cheat changed (see the module's notes).
fn mark(m: &mut Memory, unit: u32) {
    if unit == 0 || r32(m, unit + 0x9C) == 0 {
        return;
    }
    let state = r32(m, unit + CHEATED_WORD);
    if state & CHEATED_BIT == 0 {
        w32(m, unit + CHEATED_WORD, state | CHEATED_BIT);
        log::info!("cheats: unit {unit:08x} is marked as changed by cheats");
    }
}

/// online2: whether a unit carries the mark.
pub fn marked(m: &mut Memory, unit: u32) -> bool {
    r32(m, unit + CHEATED_WORD) & CHEATED_BIT != 0
}

/// the counts for the end of a headless run.
pub fn summary() -> Option<String> {
    HOOKS.get()?;
    let r = read;
    Some(format!(
        "cheats: flags {:#x} crit {}, requests {} done; crits {}, hits {}, damage {}, growths {}, exp {}, seals kept {}, turns held {}, log answers {} (of {} lookups, last from {:08x}; {} persons asked, {} listed), moves again {}; cheats2: veins used {} made ready {}, capture searches {}, captures {} joined {} to the Jail {} (last {:08x}), buildings {}",
        r(word::FLAGS),
        r(word::CRIT),
        r(word::DONE),
        r(word::STATS),
        r(word::STATS + 1),
        r(word::STATS + 2),
        r(word::STATS + 3),
        r(word::STATS + 4),
        r(word::STATS + 5),
        r(word::STATS + 6),
        r(word::STATS + 7),
        r(word::STATS + 9),
        r(word::STATS + 10),
        r(word::STATS + 11),
        r(word::STATS + 12),
        r(word::STATS + 8),
        r(word::STATS + 19),
        r(word::STATS + 13),
        r(word::STATS2),
        r(word::STATS + 18),
        r(word::STATS + 14),
        r(word::STATS + 15),
        r(word::STATS + 16),
        r(word::STATS + 17)
    ))
}

// ---- loading the game's tables ------------------------------------------------

fn load_tables(s: &mut State, system: &mut System) {
    if s.loaded {
        return;
    }
    let m = &mut system.memory;
    // the person table must be there (a game has started)
    let pnode = r32(m, zakuro_core::region::us(PERSON_LIST));
    let jnode = r32(m, zakuro_core::region::us(JOB_LIST));
    let inode = r32(m, zakuro_core::region::us(ITEM_LIST));
    if pnode == 0 || jnode == 0 || inode == 0 || r32(m, zakuro_core::region::us(SKILLS)) == 0 || r32(m, zakuro_core::region::us(CHAPTERS)) == 0 {
        return;
    }
    if s.english.is_empty() {
        let rom = system.title.as_ref().and_then(zakuro_core::randomizer::rom::Rom::of);
        let Some(rom) = rom else { return };
        for path in ["m/@E/GameData.bin.lz", "m/@E/Skill.bin.lz", "m/@E/System.bin.lz", "m/@U/GameData.bin.lz", "m/@U/Skill.bin.lz", "m/@U/System.bin.lz", "m/GameData.bin.lz", "m/Skill.bin.lz", "m/System.bin.lz"] { // eur: @U, the EUR English; jpprep: m/, the Japanese
            if let Some(file) = rom.read(path).and_then(|f| zakuro_core::randomizer::rom::lz11(&f)) {
                for (k, v) in zakuro_core::randomizer::rom::messages(&file) {
                    s.english.entry(k).or_insert(v);
                }
            }
        }
        log::info!("cheats: {} English names read", s.english.len());
    }
    let m = &mut system.memory;
    let english = &s.english;
    let name_of = |m: &mut Memory, field: u32| -> String {
        let key = cstr_at(m, field);
        english.get(&key).cloned().unwrap_or_default()
    };

    // classes: the first table of Job's list (GameData's)
    let table = r32(m, jnode);
    let count = r16(m, table + 6) as u32;
    s.classes.clear();
    for i in 0..count {
        let at = table + 8 + i * 0x80;
        let flags = r32(m, at);
        let flags2 = r32(m, at + 4);
        let id = r16(m, at + 0x18);
        let name = name_of(m, at + 0x10);
        let mut wexp_max = [0u8; 8];
        for (k, v) in wexp_max.iter_mut().enumerate() {
            *v = r8(m, at + 0x44 + k as u32);
        }
        // not offered: nobody (0), the Outrealm class (128), monsters
        // (byte 3 bit 3: Faceless, Stoneborn, dragons, the kings) and
        // dragon forms (byte 4 bit 5)
        let monster = (flags >> 24) & 8 != 0 || flags2 & 0x20 != 0;
        if id == 0 || name.is_empty() || monster || name == "Outrealm Class" || name == "Lancer" {
            continue;
        }
        s.classes.push(ClassEntry { id, addr: at, name, female: flags & 1 != 0, other: r16(m, at + 0x70), promoted: flags & 8 != 0, wexp_max });
    }

    // persons: the first table of Person's list (GameData's)
    let table = r32(m, pnode);
    let count = r16(m, table + 4) as u32;
    s.persons.clear();
    let mut seen_names: HashMap<String, usize> = HashMap::new();
    for i in 0..count {
        let at = table + 0x10 + i * 0x98;
        let pid = cstr_at(m, at + 8);
        let fid = cstr_at(m, at + 0xC);
        let name = name_of(m, at + 0x14);
        let id = r16(m, at + 0x24);
        // playable characters have a portrait and a name; the first three
        // entries are nobody and the avatar's
        if i < 3 || name.is_empty() || fid.is_empty() || !pid.starts_with(b"PID_") {
            continue;
        }
        // captured and bond units (PID_捕獲_, PID_絆_) are not characters
        if pid.starts_with(b"PID_\x95\xDF") || pid.starts_with(b"PID_\xE3\x4A") {
            continue;
        }
        let female = r8(m, at) & 1 != 0;
        let parent = r16(m, at + 0x2A);
        let child = parent != 0 && r16(m, at + 0x82) != 0xFFFF;
        // amiibo heroes, as JoinGuest brings them (after Anna in the table)
        let guest = i > 71 && !child;
        let label = match seen_names.get(&name) {
            // Kana and the like have one entry per gender
            Some(_) => format!("{} ({})", name, if female { "F" } else { "M" }),
            None => name.clone(),
        };
        seen_names.insert(name.clone(), s.persons.len());
        s.persons.push(PersonEntry { id, addr: at, pid, name: label, female, guest, child });
    }
    // the first of a pair of same-named entries gets its gender too
    let dup: Vec<String> = s.persons.iter().filter(|p| p.name.ends_with(" (F)") || p.name.ends_with(" (M)")).map(|p| p.name[..p.name.len() - 4].to_string()).collect();
    for p in s.persons.iter_mut() {
        if dup.contains(&p.name) {
            p.name = format!("{} ({})", p.name, if p.female { "F" } else { "M" });
        }
    }

    // skills
    let table = r32(m, zakuro_core::region::us(SKILLS));
    let count = r32(m, zakuro_core::region::us(SKILLS) + 8).min(400);
    s.skills.clear();
    s.capture_skills.clear();
    for i in 1..count {
        let at = table + i * 32;
        let name = name_of(m, at + 4);
        let id = r16(m, at + 0x10);
        // cheats2: the Prison's Capture skills, by their SEID (what the
        // map's Capture command asks Unit::IsEquipSkill for)
        let seid = cstr_at(m, at);
        if id != 0 && (seid == b"SEID_\x94\x92\x96\xe9\x82\xcc\x95\xdf\x94\x9b\x8fp" || seid == b"SEID_\x88\xc3\x96\xe9\x82\xcc\x8dS\x91\xa9\x8fp") {
            s.capture_skills.push(id);
        }
        if name.is_empty() || id == 0 {
            continue;
        }
        s.skills.push(SkillEntry { id, name });
    }

    // items
    let table = r32(m, inode);
    let count = r16(m, table + 6) as u32;
    s.items.clear();
    s.seals.clear();
    for i in 0..count {
        let at = table + 8 + i * 0x68;
        let id = r16(m, at + 0x14);
        let iid = cstr_at(m, at + 8);
        let name = name_of(m, at + 0xC);
        if id == 0 || name.is_empty() {
            continue;
        }
        if matches!(name.as_str(), "Heart Seal" | "Partner Seal" | "Friendship Seal" | "Eternal Seal") {
            s.seals.push(id);
        }
        let _ = iid;
        s.items.push((id, name));
    }

    // castlegifts: the Inbox's Battle and Visit Bonuses, for the Add Item
    // list's notes (True Ganglari is "Ganglari" like the story's sword)
    s.bonus_items.clear();
    let achieve = zakuro_core::region::us(ACHIEVE_BONUS);
    for (word, kind) in [(4u32, "Battle Bonus"), (8, "Visit Bonus")] {
        let list = r32(m, achieve + word);
        let (entries, count) = (r32(m, list), r32(m, list + 4).min(256));
        for i in 0..count {
            let id = r32(m, entries + 8 * i + 4) as u16;
            if id != 0 {
                let note = s.bonus_items.entry(id).or_default();
                if note.is_empty() {
                    *note = kind.to_string();
                } else if !note.contains(kind) {
                    *note = "Battle & Visit Bonus".to_string();
                }
            }
        }
    }

    // chapters
    let table = r32(m, zakuro_core::region::us(CHAPTERS));
    let count = r32(m, zakuro_core::region::us(CHAPTERS) + 4).min(256);
    s.chapters.clear();
    for i in 0..count {
        let at = table + i * 0x1C;
        let cid = String::from_utf8_lossy(&cstr_at(m, at)).into_owned();
        let Some(rest) = cid.strip_prefix("CID_") else { continue };
        let mut key = b"MCID_".to_vec();
        key.extend_from_slice(rest.as_bytes());
        let name = english.get(&key).cloned().unwrap_or_default();
        let mut pkey = key.clone();
        pkey.extend_from_slice(b"_PREFIX");
        let prefix = english.get(&pkey).cloned().unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        // story chapters (kind 0), paralogues (1) and the Hero Battles (7);
        // not My Castle's own maps, invasions, the ending or the test map
        let kind = r8(m, at + 9);
        if !matches!(kind, 0 | 1 | 7) || cid.starts_with("CID_Z") {
            continue;
        }
        let routes = r8(m, at + 0x14) & 7;
        let route = match routes {
            1 => 0,
            2 => 1,
            4 => 2,
            _ => 0xFF,
        };
        s.chapters.push(ChapterEntry { addr: at, cid, name, prefix, route, routes, kind });
    }
    s.loaded = !s.classes.is_empty() && !s.persons.is_empty();
    log::info!(
        "cheats: the game's tables: {} classes, {} characters, {} skills, {} items ({} seals), {} chapters",
        s.classes.len(),
        s.persons.len(),
        s.skills.len(),
        s.items.len(),
        s.seals.len(),
        s.chapters.len()
    );
    for (i, seal) in s.seals.iter().take(8).enumerate() {
        write(word::SEALS + i, *seal as u32);
    }
    for (i, skill) in s.capture_skills.iter().take(2).enumerate() {
        write(word::CAPTURE_SKILL + i, *skill as u32);
    }
    log::info!("cheats: Capture skills {:?}", s.capture_skills);
}

fn unit_list(m: &mut Memory, force: u32) -> Vec<u32> {
    let forces = r32(m, zakuro_core::region::us(FORCES));
    let mut out = Vec::new();
    if forces == 0 {
        return out;
    }
    let mut unit = r32(m, forces + force * 12);
    while unit != 0 && out.len() < 256 {
        out.push(unit);
        unit = r32(m, unit + 0x14);
    }
    out
}

fn refresh_roster(s: &mut State, m: &mut Memory) {
    let pool = r32(m, zakuro_core::region::us(UNIT_POOL));
    let mut roster = Vec::new();
    if pool != 0 {
        for force in [0u32, 3] {
            for unit in unit_list(m, force) {
                let person = r32(m, unit + 0x9C);
                let job = r32(m, unit + 0xA0);
                if person == 0 || job == 0 {
                    continue;
                }
                let pid = cstr_at(m, person + 8);
                let person_id = r16(m, person + 0x24);
                let mut name = s.english.get(&cstr_at(m, person + 0x14)).cloned().unwrap_or_else(|| String::from_utf8_lossy(&pid).into_owned());
                // the avatar's own name (unit::Edit +0, wchar_t)
                let edit = r32(m, unit + 0xB0);
                if edit != 0 {
                    let mut units = Vec::new();
                    for i in 0..16 {
                        let c = r16(m, edit + 2 * i);
                        if c == 0 {
                            break;
                        }
                        units.push(c);
                    }
                    let own = String::from_utf16_lossy(&units);
                    if !own.trim().is_empty() && own.chars().all(|c| !c.is_control()) {
                        name = own;
                    }
                }
                let class_id = r16(m, job + 0x18);
                let class_name = s.english.get(&cstr_at(m, job + 0x10)).cloned().unwrap_or_default();
                let mut wexp = [0u8; 8];
                for (k, v) in wexp.iter_mut().enumerate() {
                    *v = r8(m, unit + 0xE8 + k as u32);
                }
                let mut skills = [0u16; 5];
                for (k, v) in skills.iter_mut().enumerate() {
                    *v = r16(m, unit + 0xFE + 2 * k as u32);
                }
                roster.push(UnitView {
                    addr: unit,
                    person,
                    pid,
                    person_id,
                    name,
                    female: r8(m, person) & 1 != 0,
                    job,
                    class_id,
                    class_name,
                    level: r8(m, unit + 0xF1),
                    exp: r8(m, unit + 0xF2),
                    hp: r8(m, unit + 0xF3),
                    force: force as u8,
                    wexp,
                    skills,
                    marked: marked(m, unit),
                });
            }
        }
    }
    s.roster = roster;
    if s.chosen == 0 || !s.roster.iter().any(|u| u.addr == s.chosen) {
        s.chosen = s.roster.first().map_or(0, |u| u.addr);
    }
    let gud = r32(m, zakuro_core::region::us(GAME_USER));
    s.in_game = gud != 0 && pool != 0 && !s.roster.is_empty();
    s.gold = if gud != 0 { r32(m, gud + 0x38) } else { 0 };
    s.route = if gud != 0 { r8(m, gud + 0x2E) } else { 0 };
    s.on_map = r32(m, zakuro_core::region::us(SITUATION)) != 0;
    s.in_castle = r32(m, zakuro_core::region::us(CASTLE_MAIN)) != 0 && r32(m, zakuro_core::region::us(FREE_MOVE)) != 0 && !s.on_map;
}

fn read_view(s: &mut State) {
    let mut v = StatView { unit: read(word::VIEW + 19), ..StatView::default() };
    for i in 0..8 {
        v.stats[i] = read(word::VIEW + i) as i32;
        v.caps[i] = read(word::VIEW + 8 + i) as i32;
    }
    v.mov = read(word::VIEW + 16) as i32;
    v.level_cap = read(word::VIEW + 17) as i32;
    v.max_hp = read(word::VIEW + 18) as i32;
    s.view = v;
}

fn extra_skill_words(s: &State) {
    // online2: none while the online battle is up (write_switches)
    if zakuro_core::netplay::battle() {
        write(word::SKILL_COUNT, 0);
        return;
    }
    let mut words = Vec::new();
    for entry in &s.settings.extra_skills {
        let Some((pid, skill)) = entry.split_once('|') else { continue };
        let Ok(skill) = skill.trim().parse::<u16>() else { continue };
        if let Some(p) = s.persons.iter().find(|p| p.pid == pid.as_bytes()) {
            words.push((p.id as u32) << 16 | skill as u32);
        }
    }
    for (i, w) in words.iter().take(256).enumerate() {
        write(word::SKILLS + i, *w);
    }
    write(word::SKILL_COUNT, words.len().min(256) as u32);
}

/// each frame, between two of the game's: the menu's view, the actions.
pub fn frame(system: &mut System) {
    if !available() {
        return;
    }
    with_state(|s| {
        s.frame += 1;
        // cheats2: Capture needs the tables (its skills' ids)
        let busy = s.menu_open || !s.pending.is_empty() || s.waiting.is_some() || !s.settings.extra_skills.is_empty() || s.buildings.is_some() || (s.settings.capture_recruit && !s.loaded);
        if !busy {
            if read(word::VIEW_UNIT) != 0 {
                write(word::VIEW_UNIT, 0);
            }
            return;
        }
        if !s.loaded {
            load_tables(s, system);
            if s.loaded {
                extra_skill_words(s);
            }
        }
        if s.menu_open || !s.pending.is_empty() {
            refresh_roster(s, &mut system.memory);
            read_view(s);
            write(word::VIEW_UNIT, if s.menu_open { s.chosen } else { 0 });
            write(word::VIEW_TICK, read(word::VIEW_TICK).wrapping_add(1));
        } else if read(word::VIEW_UNIT) != 0 {
            write(word::VIEW_UNIT, 0);
        }
        // a request cheats.c has finished
        if let Some((done, what)) = s.waiting.clone() {
            if read(word::DONE) != done {
                let result = read(word::RESULT);
                // online2: the units it changed, when the game did it
                let succeeded = result != 0 && result != 0xFFFF_FFFF && !matches!(result, 2 | 3);
                let marks = std::mem::take(&mut s.mark_after);
                let join = s.join_after.take();
                if succeeded {
                    for unit in marks {
                        mark(&mut system.memory, unit);
                    }
                    if let Some(person) = join {
                        refresh_roster(s, &mut system.memory);
                        let units: Vec<u32> = s.roster.iter().filter(|u| u.person == person).map(|u| u.addr).collect();
                        for unit in units {
                            mark(&mut system.memory, unit);
                        }
                    }
                }
                let text = match result {
                    // castlegifts: Add Item (one item: 2 when it was added)
                    r if what.starts_with("Convoy: ") => {
                        if r >= 2 {
                            what["Convoy: ".len()..].to_string()
                        } else {
                            "The convoy is full.".to_string()
                        }
                    }
                    // cheats2: one of Max Buildings' steps; the last one says it
                    _ if what.starts_with("My Castle: a building") => String::new(),
                    0xFFFF_FFFF => format!("{what}: the game stopped it."),
                    0 => format!("{what}: not possible here."),
                    2 if what.starts_with("Chapter") => "Chapter Select works from My Castle.".to_string(),
                    2 => format!("{what}: already in your army."),
                    3 => format!("{what}: your army is full."),
                    _ => format!("{what}: done."),
                };
                if text.is_empty() {
                    s.waiting = None;
                    return;
                }
                log::info!("cheats: {text}");
                println!("cheats: frame {} {text}", s.frame);
                s.notices.push_back(text);
                s.waiting = None;
                refresh_roster(s, &mut system.memory);
            } else {
                return;
            }
        }
        if let Some(action) = s.pending.pop_front() {
            run(s, system, action);
        } else if s.buildings.is_some() {
            buildings_step(s, &mut system.memory);
        }
    });
}

// ---- cheats2: Max My Castle Buildings ----------------------------------------------

const BUILDINGS: u32 = 0x006D_19FC; // castle::CastleBuilding: +0 index by unique index (u16), +8 the records (count at +0)
const BUILD_SEQ: u32 = 0x006D_29AC; // the building sequence running, or 0

/// a building's record (castle::CastleBuilding::Data, 0x68 bytes: +0 unique
/// index, +2 level, +3 type, +0x4C its Dragon Vein Point cost)
fn building_record(m: &mut Memory, uid: u32) -> u32 {
    let (index, base) = (r32(m, zakuro_core::region::us(BUILDINGS)), r32(m, zakuro_core::region::us(BUILDINGS) + 8));
    if index == 0 || base == 0 || uid >= 0x208 {
        return 0;
    }
    let slot = r16(m, index + uid * 2) as u32;
    if slot >= r16(m, base) as u32 {
        return 0;
    }
    base + slot * 0x68 + 4
}

/// the placed buildings that have a next level (unique index, level): the
/// Build menu's own test (castle::cut::CanListUpForLevelUp) without its
/// chapter and statue requirements: not terrain, below level 3, built, and
/// the next record (unique index + 1) of the same type and the next level
/// not built yet
fn building_candidates(m: &mut Memory) -> Vec<(u16, u8)> {
    let mut out = Vec::new();
    let castle = r32(m, zakuro_core::region::us(CASTLE));
    let data = if castle != 0 { r32(m, castle + 8) } else { 0 };
    let cud = r32(m, data);
    let dispos = if data != 0 { r32(m, data + 4) } else { 0 };
    if cud == 0 || dispos == 0 {
        return out;
    }
    let built = |m: &mut Memory, uid: u32| r8(m, cud + 0x46 + (uid >> 3)) & (1 << (uid & 7)) != 0;
    let mut d = r32(m, dispos + 4);
    let mut guard = 0;
    while d != 0 && guard < 128 {
        guard += 1;
        let uid = r32(m, d + 0x18) & 0xFFFF;
        d = r32(m, d + 0x10);
        let rec = building_record(m, uid);
        if rec == 0 || uid > 500 || r16(m, rec) as u32 != uid {
            continue;
        }
        let (level, kind) = (r8(m, rec + 2), r8(m, rec + 3));
        if level >= 3 || !built(m, uid) {
            continue;
        }
        let next = building_record(m, uid + 1);
        if next == 0 || r16(m, next) as u32 != uid + 1 || r8(m, next + 3) != kind || r8(m, next + 2) != level + 1 || built(m, uid + 1) {
            continue;
        }
        out.push((uid as u16, level));
    }
    out
}

fn castle_points(m: &mut Memory) -> (u32, u32) {
    let castle = r32(m, zakuro_core::region::us(CASTLE));
    let cud = if castle != 0 { r32p(m, castle + 8) } else { 0 };
    (cud, if cud != 0 { r32(m, cud + 0xD0) } else { 0 })
}

/// the next building's level-up, once the last one's sequence has ended
fn buildings_step(s: &mut State, m: &mut Memory) {
    let Some(mut run) = s.buildings.take() else { return };
    if r32(m, zakuro_core::region::us(BUILD_SEQ)) != 0 {
        run.idle = 0;
        s.buildings = Some(run);
        return;
    }
    run.idle += 1;
    if run.idle < 20 {
        s.buildings = Some(run);
        return;
    }
    let free_roam = r32(m, zakuro_core::region::us(CASTLE_MAIN)) != 0 && r32(m, zakuro_core::region::us(FREE_MOVE)) != 0 && r32(m, zakuro_core::region::us(SITUATION)) == 0;
    let next = building_candidates(m).into_iter().find(|(uid, _)| run.asked.get(uid).copied().unwrap_or(0) < 2);
    match next {
        Some((uid, level)) if free_roam => {
            // the level-up sequence takes the building's cost from the
            // points: enough is lent for it, and the points the player had
            // are put back at the end
            let (cud, _) = castle_points(m);
            if cud != 0 {
                w32(m, cud + 0xD0, 999 * 100);
            }
            *run.asked.entry(uid).or_insert(0) += 1;
            run.upgraded += 1;
            run.idle = 0;
            println!("cheats: frame {} building {uid} level {level} -> {}", s.frame, level + 1);
            s.buildings = Some(run);
            hand_marking(s, op::BUILDING, &[uid as u32], "My Castle: a building's level-up".to_string(), &[]);
        }
        _ => {
            // done (or free roam has ended): the points come back
            let (cud, _) = castle_points(m);
            if cud != 0 {
                w32(m, cud + 0xD0, run.points);
            }
            let left = building_candidates(m).len();
            let text = if left == 0 {
                format!("My Castle: {} level-ups, every building at its highest level.", run.upgraded)
            } else {
                format!("My Castle: {} level-ups; {left} could not be done from here.", run.upgraded)
            };
            notice(s, text);
        }
    }
}

fn hand(s: &mut State, op: u32, args: &[u32], what: String) {
    for (i, a) in args.iter().enumerate() {
        write(word::ARG + i, *a);
    }
    s.waiting = Some((read(word::DONE), what));
    write(word::REQ, op);
}

/// online2: hand, marking these units once it is done
fn hand_marking(s: &mut State, op: u32, args: &[u32], what: String, units: &[u32]) {
    s.mark_after = units.to_vec();
    s.join_after = None;
    hand(s, op, args, what);
}

fn notice(s: &mut State, text: String) {
    log::info!("cheats: {text}");
    println!("cheats: frame {} {text}", s.frame);
    s.notices.push_back(text);
}

fn unit_name(s: &State, unit: u32) -> String {
    s.roster.iter().find(|u| u.addr == unit).map_or_else(|| "The unit".to_string(), |u| u.name.clone())
}

fn run(s: &mut State, system: &mut System, action: Action) {
    let m = &mut system.memory;
    match action {
        Action::ChangeClass { unit, class } => {
            let Some(view) = s.roster.iter().find(|u| u.addr == unit).cloned() else { return };
            let Some(mut entry) = s.classes.iter().find(|c| c.id == class).cloned() else { return };
            // the class's version for the unit's gender
            if entry.female != view.female {
                match s.classes.iter().find(|c| c.id == entry.other && c.female == view.female) {
                    Some(other) => entry = other.clone(),
                    None => {
                        notice(s, format!("{} can't be a {}.", view.name, entry.name));
                        return;
                    }
                }
            }
            hand_marking(s, op::CLASS, &[unit, entry.addr], format!("{} is now a {}", view.name, entry.name), &[unit]);
        }
        Action::SetStat { unit, stat, value } => {
            if s.view.unit != unit || stat >= 8 {
                return;
            }
            let current = s.view.stats[stat];
            let mut cap = s.view.caps[stat];
            let value = value.clamp(0, 99);
            let ignore = s.settings.ignore_caps;
            let value = if ignore { value } else { value.min(cap) };
            if value > cap {
                // the unit's own cap offset (+0xD0, kept at least +0xD8,
                // which Unit::UpdateLimitOffset leaves), as the castle's
                // statues raise it
                let raise = (value - cap) as i32;
                for at in [0xD0u32, 0xD8] {
                    let old = r8(m, unit + at + stat as u32) as i8 as i32;
                    w8(m, unit + at + stat as u32, (old + raise).clamp(-128, 127) as i8 as u8);
                }
                cap = value;
            }
            let _ = cap;
            let gain = r8(m, unit + 0xC8 + stat as u32) as i8 as i32;
            let new_gain = (gain + value - current).clamp(-128, 127);
            w8(m, unit + 0xC8 + stat as u32, new_gain as i8 as u8);
            mark(m, unit); // online2
            if stat == 0 {
                // current HP follows the maximum
                w8(m, unit + 0xF3, value.max(1) as u8);
            }
            s.view.stats[stat] = value;
        }
        Action::SetMove { unit, value } => {
            if s.view.unit != unit {
                return;
            }
            let delta = value.clamp(0, 15) - s.view.mov;
            let bonus = r8(m, unit + 0x12C) as i8 as i32;
            w8(m, unit + 0x12C, (bonus + delta).clamp(-10, 15) as i8 as u8);
            mark(m, unit); // online2
        }
        Action::SetLevel { unit, value } => {
            let cap = if s.view.unit == unit && s.view.level_cap > 0 { s.view.level_cap } else { 40 };
            let cap = if s.settings.ignore_caps { 99 } else { cap };
            w8(m, unit + 0xF1, value.clamp(1, cap) as u8);
            mark(m, unit); // online2
        }
        Action::SetExp { unit, value } => {
            w8(m, unit + 0xF2, value.clamp(0, 99) as u8);
            mark(m, unit); // online2
        }
        Action::MaxLevel { unit } => {
            let name = unit_name(s, unit);
            hand_marking(s, op::LEVEL_UP, &[unit, 99], format!("{name} reached the level cap"), &[unit]);
        }
        Action::MaxRanks { unit } => {
            let job = r32(m, unit + 0xA0);
            for k in 0..8u32 {
                let cap = r8(m, job + 0x44 + k);
                let cap = if s.settings.every_weapon { RANK_S } else { cap };
                if cap > r8(m, unit + 0xE8 + k) {
                    w8(m, unit + 0xE8 + k, cap);
                }
            }
            mark(m, unit); // online2
            let name = unit_name(s, unit);
            notice(s, format!("{name}'s weapon ranks are at their highest."));
        }
        Action::GiveSkill { unit, skill, slot } => {
            let name = unit_name(s, unit);
            let skill_name = s.skills.iter().find(|k| k.id == skill).map_or(String::new(), |k| k.name.clone());
            // learned (the personal skill pool, +0x3F, as a skill learned by level)
            let byte = unit + 0x3F + (skill as u32 >> 3);
            let bits = r8(m, byte) | 1 << (skill & 7);
            w8(m, byte, bits);
            mark(m, unit); // online2
            if slot < 5 {
                w16(m, unit + 0xFE + 2 * slot as u32, skill);
                notice(s, format!("{name} has {skill_name} equipped."));
            } else {
                let pid = String::from_utf8_lossy(&r32_pid(m, unit)).into_owned();
                let entry = format!("{pid}|{skill}");
                if !s.settings.extra_skills.contains(&entry) {
                    s.settings.extra_skills.push(entry);
                    EXTRA_CHANGED.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                extra_skill_words(s);
                notice(s, format!("{name} has {skill_name} as an extra skill."));
            }
        }
        Action::AddCharacter { person } => {
            let Some(p) = s.persons.iter().find(|p| p.id == person).cloned() else { return };
            let op = if p.guest { op::JOIN_GUEST } else { op::JOIN };
            hand_marking(s, op, &[p.addr], format!("{} joined your army", p.name), &[]);
            s.join_after = Some(p.addr); // online2
        }
        Action::SetSupport { a, b, level } => {
            let (na, nb) = (unit_name(s, a), unit_name(s, b));
            let rank = ["none", "C", "B", "A", "S"].get(level as usize).copied().unwrap_or("?");
            hand_marking(s, op::SUPPORT, &[a, b, level], format!("{na} and {nb}'s support is {rank}"), &[a, b]);
        }
        Action::MaxGold => {
            let gud = r32(m, zakuro_core::region::us(GAME_USER));
            if gud != 0 {
                w32(m, gud + 0x38, 999_999);
                notice(s, "Gold is 999,999.".to_string());
            }
        }
        Action::MaxCastle => {
            // castle::CastleUserData: ores at *(+0xD4) and their "found"
            // bits at +0xDC, food at *(+0xD8) and its bits at +0xDE (as
            // AddGemstoneNum / AddFoodNum keep them, 99 at most); Dragon
            // Vein Points at +0xD0 (what a building's upgrade costs)
            let castle = r32(m, zakuro_core::region::us(CASTLE));
            let cud = if castle != 0 { r32p(m, castle + 8) } else { 0 };
            let gems = r32p(m, zakuro_core::region::us(GEMSTONES) + 4).min(32);
            let foods = r32p(m, zakuro_core::region::us(FOODS) + 4).min(32);
            if cud == 0 || gems == 0 {
                notice(s, "My Castle's resources: load a save first.".to_string());
                return;
            }
            let (gem_counts, food_counts) = (r32(m, cud + 0xD4), r32(m, cud + 0xD8));
            for i in 0..gems {
                w8(m, gem_counts + i, 99);
                let bit = cud + 0xDC + (i >> 3);
                let v = r8(m, bit) | 1 << (i & 7);
                w8(m, bit, v);
            }
            for i in 0..foods {
                w8(m, food_counts + i, 99);
                let bit = cud + 0xDE + (i >> 3);
                let v = r8(m, bit) | 1 << (i & 7);
                w8(m, bit, v);
            }
            let points = r32(m, cud + 0xD0);
            w32(m, cud + 0xD0, points.max(99 * 100));
            notice(s, format!("My Castle: 99 of every ore ({gems}) and food ({foods}), 99 Dragon Vein Points."));
        }
        Action::MaxBuildings => {
            if !s.in_castle {
                notice(s, "Max Buildings works from My Castle.".to_string());
                return;
            }
            let todo = building_candidates(m);
            if todo.is_empty() {
                notice(s, "My Castle: every building is already at its highest level.".to_string());
                return;
            }
            let (_, points) = castle_points(m);
            println!("cheats: frame {} Max Buildings: {} to level up {:?}, {} points", s.frame, todo.len(), todo, points);
            s.buildings = Some(BuildingRun { points, idle: 100, ..BuildingRun::default() });
        }
        Action::FillConvoy => {
            let ids: Vec<u16> = s.items.iter().map(|(id, _)| *id).collect();
            let room = word::COUNT - word::ITEMS - 1;
            for (i, id) in ids.iter().take(room).enumerate() {
                write(word::ITEMS + 1 + i, *id as u32);
            }
            write(word::ITEMS, ids.len().min(room) as u32);
            let _ = r32(m, zakuro_core::region::us(TRANSPORTER));
            hand_marking(s, op::CONVOY, &[99], "The convoy has 99 of every item".to_string(), &[]);
        }
        // castlegifts: one more of the item in the convoy (99 at most), with
        // the game's own Item::New and Transporter::Add (cheats.c's convoy)
        Action::GiveItem { item } => {
            let name = s.items.iter().find(|(id, _)| *id == item).map_or_else(|| "The item".to_string(), |(_, n)| n.clone());
            let table = r32(m, zakuro_core::region::us(TRANSPORTER));
            let have = (0..500).find(|k| r16(m, table + k * 8) == item).map_or(0, |k| r32(m, table + k * 8 + 4));
            if table == 0 {
                notice(s, "Load a save first.".to_string());
                return;
            }
            if have >= 99 {
                notice(s, format!("The convoy already has 99 of {name}."));
                return;
            }
            write(word::ITEMS + 1, item as u32);
            write(word::ITEMS, 1);
            hand_marking(s, op::CONVOY, &[have + 1], format!("Convoy: {name} sent to the convoy ({} there).", have + 1), &[]);
        }
        Action::Chapter { index } => {
            let Some(c) = s.chapters.get(index).cloned() else { return };
            // the save's route follows a chapter of another route (the
            // first route it is on), so its story, units and maps fit;
            // a chapter on the current route leaves it as it is
            let route = if c.routes == 0 || c.routes & (1 << s.route.min(2)) != 0 { 0xFF } else { c.routes.trailing_zeros() };
            hand_marking(s, op::CHAPTER, &[c.addr, route], format!("Chapter Select: {}", c.name), &[]);
        }
    }
}

fn r32_pid(m: &mut Memory, unit: u32) -> Vec<u8> {
    let person = r32(m, unit + 0x9C);
    cstr_at(m, person + 8)
}

/// extra skills changed: the settings need saving (the app takes it)
pub static EXTRA_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// the extra skills as the settings should keep them
pub fn extra_skills() -> Vec<String> {
    with_state(|s| s.settings.extra_skills.clone())
}

pub fn take_notice() -> Option<String> {
    with_state(|s| s.notices.pop_front())
}

pub fn set_menu_open(open: bool) {
    with_state(|s| s.menu_open = open);
}

// ---- tests: FATES_CHEAT_AT="<frame>:<command>[;...]" on a headless run ------

struct Scripted {
    frame: u64,
    command: String,
}

static SCRIPT: Mutex<Option<Vec<Scripted>>> = Mutex::new(None);

fn parse_script() -> Vec<Scripted> {
    let Ok(text) = std::env::var("FATES_CHEAT_AT") else { return Vec::new() };
    text.split(';')
        .filter_map(|part| {
            let (frame, command) = part.split_once(':')?;
            Some(Scripted { frame: frame.trim().parse().ok()?, command: command.trim().to_string() })
        })
        .collect()
}

/// a headless frame: the test commands due, then the frame's work.
pub fn headless_frame(system: &mut System, frame: u64) {
    if !available() {
        return;
    }
    let due: Vec<String> = {
        let mut guard = SCRIPT.lock().unwrap_or_else(|e| e.into_inner());
        let list = guard.get_or_insert_with(parse_script);
        let (now, later): (Vec<Scripted>, Vec<Scripted>) = std::mem::take(list).into_iter().partition(|s| s.frame <= frame);
        *list = later;
        now.into_iter().map(|s| s.command).collect()
    };
    with_state(|s| s.frame = frame.saturating_sub(1));
    for command in due {
        with_state(|s| {
            s.menu_open = true;
        });
        frame_and_load(system);
        script_command(system, &command, frame);
    }
    frame_and_load(system);
}

fn frame_and_load(system: &mut System) {
    frame(system);
}

/// a test command, as the menu would ask it: "unit <name>", "class <name>",
/// "stat <Str> <value>", "level <n>", "exp <n>", "move <n>", "maxlevel",
/// "ranks", "skill <name> [slot]", "join <name>", "support <a> <b> <C|B|A|S>",
/// "gold", "convoy", "chapter <CID>", "dump", "close"
fn script_command(system: &mut System, command: &str, frame: u64) {
    let mut words = command.splitn(2, ' ');
    let verb = words.next().unwrap_or("");
    let rest = words.next().unwrap_or("").trim().to_string();
    with_state(|s| {
        refresh_roster(s, &mut system.memory);
        read_view(s);
        let find_unit = |s: &State, name: &str| s.roster.iter().find(|u| u.name.eq_ignore_ascii_case(name)).map(|u| u.addr);
        match verb {
            "unit" => {
                if let Some(u) = find_unit(s, &rest) {
                    s.chosen = u;
                    write(word::VIEW_UNIT, u);
                }
            }
            "class" => {
                let class = s.classes.iter().find(|c| c.name.eq_ignore_ascii_case(&rest)).map(|c| c.id);
                if let Some(class) = class {
                    let unit = s.chosen;
                    s.pending.push_back(Action::ChangeClass { unit, class });
                }
            }
            "stat" => {
                let mut p = rest.split_whitespace();
                let (name, value) = (p.next().unwrap_or(""), p.next().and_then(|v| v.parse().ok()).unwrap_or(0));
                if let Some(stat) = STAT_NAMES.iter().position(|n| n.eq_ignore_ascii_case(name)) {
                    let unit = s.chosen;
                    s.pending.push_back(Action::SetStat { unit, stat, value });
                } else if name.eq_ignore_ascii_case("mov") {
                    let unit = s.chosen;
                    s.pending.push_back(Action::SetMove { unit, value });
                }
            }
            "level" => {
                let unit = s.chosen;
                s.pending.push_back(Action::SetLevel { unit, value: rest.parse().unwrap_or(1) });
            }
            "exp" => {
                let unit = s.chosen;
                s.pending.push_back(Action::SetExp { unit, value: rest.parse().unwrap_or(0) });
            }
            "maxlevel" => {
                let unit = s.chosen;
                s.pending.push_back(Action::MaxLevel { unit });
            }
            "ranks" => {
                let unit = s.chosen;
                s.pending.push_back(Action::MaxRanks { unit });
            }
            "skill" => {
                let (name, slot) = match rest.rsplit_once(' ') {
                    Some((n, slot)) if slot.parse::<usize>().is_ok() => (n.to_string(), slot.parse().unwrap_or(5)),
                    _ => (rest.clone(), 5),
                };
                if let Some(skill) = s.skills.iter().find(|k| k.name.eq_ignore_ascii_case(&name)).map(|k| k.id) {
                    let unit = s.chosen;
                    s.pending.push_back(Action::GiveSkill { unit, skill, slot });
                }
            }
            "join" => {
                if let Some(person) = s.persons.iter().find(|p| p.name.eq_ignore_ascii_case(&rest)).map(|p| p.id) {
                    s.pending.push_back(Action::AddCharacter { person });
                } else {
                    println!("cheats: frame {frame} no character {rest}");
                }
            }
            "support" => {
                let parts: Vec<&str> = rest.split_whitespace().collect();
                if parts.len() == 3 {
                    let level = ["none", "c", "b", "a", "s"].iter().position(|r| r.eq_ignore_ascii_case(parts[2])).unwrap_or(0) as u32;
                    if let (Some(a), Some(b)) = (find_unit(s, parts[0]), find_unit(s, parts[1])) {
                        s.pending.push_back(Action::SetSupport { a, b, level });
                    }
                }
            }
            "gold" => s.pending.push_back(Action::MaxGold),
            "convoy" => s.pending.push_back(Action::FillConvoy),
            // castlegifts: `give ID` (an item id: names repeat), one more in the convoy
            "give" => match rest.trim().parse::<u16>() {
                Ok(item) => s.pending.push_back(Action::GiveItem { item }),
                Err(_) => println!("cheats: frame {frame} give wants an item id, not {rest}"),
            },
            "castle" => s.pending.push_back(Action::MaxCastle),
            // cheats2
            "buildings" => s.pending.push_back(Action::MaxBuildings),
            "item" => {
                let id = s.items.iter().find(|(_, n)| n.eq_ignore_ascii_case(&rest)).map(|(id, _)| *id);
                match id {
                    Some(id) => {
                        let unit = s.chosen;
                        hand_marking(s, op::ITEM, &[unit, id as u32], format!("{rest} given"), &[unit]);
                    }
                    None => println!("cheats: frame {frame} no item {rest}"),
                }
            }
            "points" => {
                let (cud, _) = castle_points(&mut system.memory);
                w32(&mut system.memory, cud + 0xD0, rest.parse::<u32>().unwrap_or(0) * 100);
            }
            "chapter" => {
                if let Some(index) = s.chapters.iter().position(|c| c.cid.eq_ignore_ascii_case(&rest) || c.name.eq_ignore_ascii_case(&rest)) {
                    s.pending.push_back(Action::Chapter { index });
                }
            }
            "close" => s.menu_open = false,
            // online2 tests: mark the chosen unit, or every unit of the army
            "mark" => {
                let units: Vec<u32> = if rest == "all" { s.roster.iter().map(|u| u.addr).collect() } else { vec![s.chosen] };
                for unit in units {
                    mark(&mut system.memory, unit);
                }
                refresh_roster(s, &mut system.memory);
            }
            // tests: My Castle's Records Hall, as a visit opens it
            "records" => {
                let level = rest.parse().unwrap_or(1);
                hand_marking(s, 9, &[level], "Records Hall".to_string(), &[]);
            }
            // jpn tests: My Room's petting view (the Japanese build's Live2D
            // skinship) for a unit: "live2d <unit> [support point 0-4]"
            "live2d" => {
                let (name, point) = match rest.rsplit_once(' ') {
                    Some((n, p)) if p.parse::<u32>().is_ok() => (n.to_string(), p.parse().unwrap_or(0)),
                    _ => (rest.clone(), 0),
                };
                if !zakuro_core::region::is_jpn() {
                    println!("cheats: frame {frame} live2d: only the Japanese game has the petting view");
                } else if let Some(u) = find_unit(s, &name) {
                    // Live2DHandler::Create in the Japanese build (0x001CA624; not in the USA one)
                    hand_marking(s, 12, &[u, point, 0x001C_A624], format!("My Room's petting view for {name}"), &[]);
                } else {
                    println!("cheats: frame {frame} live2d: no unit {name}");
                }
            }
            "chapters" => {
                for c in &s.chapters {
                    println!("cheats:   chapter {} {} {}: {} (routes {})", c.cid, c.kind, c.prefix, c.name, c.routes);
                }
            }
            _ => {}
        }
        if verb == "dump" || verb == "unit" {
            dump(s, frame);
            dump_castle(&mut system.memory);
            dump_cheats2(&mut system.memory);
        }
    });
}

/// cheats2: the map's tricks (Dragon Veins are type 0xB), the castle's
/// buildings and the Jail
fn dump_cheats2(m: &mut Memory) {
    let trick = r32(m, zakuro_core::region::us(0x006D_7FEC));
    let mut d = if trick != 0 { r32(m, trick) } else { 0 };
    let mut guard = 0;
    while d != 0 && guard < 64 {
        guard += 1;
        println!(
            "cheats:   trick {d:08x} type {:#x} at {},{} size {}x{} state {:#x} d {} e {} name {}",
            r8(m, d + 0xC),
            r8(m, d + 8) as i8,
            r8(m, d + 9) as i8,
            r8(m, d + 0xA) as i8,
            r8(m, d + 0xB) as i8,
            r8(m, d + 0x16),
            r8(m, d + 0xD) as i8,
            r8(m, d + 0xE) as i8,
            String::from_utf8_lossy(&cstr_at(m, d + 0x18))
        );
        d = r32(m, d);
    }
    // every unit on the map, with its place (+0xF4, +0xF5), HP, class and items
    for force in [0u32, 1, 2] {
        for unit in unit_list(m, force) {
            let person = r32(m, unit + 0x9C);
            let job = r32(m, unit + 0xA0);
            let items: Vec<String> = (0..5).map(|i| format!("{:x}", r32(m, unit + 0x108 + 4 * i))).collect();
            println!(
                "cheats:   unit {unit:08x} f{force} {} {} at {},{} HP{} flags {:08x} items {}",
                String::from_utf8_lossy(&cstr_at(m, person + 8)),
                String::from_utf8_lossy(&cstr_at(m, job + 8)),
                r8(m, unit + 0xF4) as i8,
                r8(m, unit + 0xF5) as i8,
                r8(m, unit + 0xF3),
                r32(m, unit + 8),
                items.join(" ")
            );
        }
    }
    let castle = r32(m, zakuro_core::region::us(CASTLE));
    let data = if castle != 0 { r32(m, castle + 8) } else { 0 };
    let dispos = if data != 0 { r32(m, data + 4) } else { 0 };
    if dispos != 0 {
        let mut d = r32(m, dispos + 4);
        let mut list = Vec::new();
        let mut guard = 0;
        while d != 0 && guard < 128 {
            guard += 1;
            let uid = r32(m, d + 0x18) & 0xFFFF;
            let rec = building_record(m, uid);
            list.push(format!("{uid}:t{}L{}@{},{}", r8(m, rec + 3), r8(m, rec + 2), r8(m, d + 0x1C), r8(m, d + 0x1D)));
            d = r32(m, d + 0x10);
        }
        println!("cheats:   buildings {} {}; to level up {:?}", list.len(), list.join(" "), building_candidates(m));
    }
    let jail = if data != 0 { r32(m, data + 0x14) } else { 0 };
    if jail != 0 {
        println!("cheats:   jail capacity {} count {}", r16(m, jail), r32(m, jail + 8));
    }
}

fn dump_castle(m: &mut Memory) {
    let castle = r32(m, zakuro_core::region::us(CASTLE));
    let cud = if castle != 0 { r32p(m, castle + 8) } else { 0 };
    if cud == 0 {
        return;
    }
    let gems = r32p(m, zakuro_core::region::us(GEMSTONES) + 4).min(32);
    let foods = r32p(m, zakuro_core::region::us(FOODS) + 4).min(32);
    let (g, f) = (r32(m, cud + 0xD4), r32(m, cud + 0xD8));
    let gv: Vec<u8> = (0..gems).map(|i| r8(m, g + i)).collect();
    let fv: Vec<u8> = (0..foods).map(|i| r8(m, f + i)).collect();
    println!("cheats:   castle {cud:08x} points {} / {} ores {gv:?} food {fv:?}", r32(m, cud + 0xCC), r32(m, cud + 0xD0));
}

fn dump(s: &State, frame: u64) {
    println!("cheats: frame {frame} roster {} units, gold {}, route {}, castle {}, map {}", s.roster.len(), s.gold, s.route, s.in_castle, s.on_map);
    for u in &s.roster {
        println!(
            "cheats:   {:08x} f{} {:<12} {:<16} Lv{:>2} Exp{:>2} HP{:>2} wexp {:?} skills {:?}{}",
            u.addr,
            u.force,
            u.name,
            u.class_name,
            u.level,
            u.exp,
            u.hp,
            u.wexp,
            u.skills,
            if u.marked { " (marked)" } else { "" }
        );
    }
    let v = &s.view;
    if v.unit != 0 || read(word::VIEW_UNIT) != 0 {
        println!("cheats:   view {:08x} (asked {:08x}) stats {:?} caps {:?} mov {} level cap {} max hp {}", v.unit, read(word::VIEW_UNIT), v.stats, v.caps, v.mov, v.level_cap, v.max_hp);
    }
}

/// the names the menu lists, for tests
#[allow(dead_code)]
pub fn names() -> BTreeMap<&'static str, usize> {
    with_state(|s| {
        let mut out = BTreeMap::new();
        out.insert("classes", s.classes.len());
        out.insert("persons", s.persons.len());
        out.insert("skills", s.skills.len());
        out.insert("chapters", s.chapters.len());
        out
    })
}
