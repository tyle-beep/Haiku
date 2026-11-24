//! cheats: the Options menu's Cheats tab, drawn by options.rs in the
//! game's style like every other tab: the tab lists four groups (Units,
//! Battle, Items & Resources, Story & Supports); each opens a page of rows
//! (a value pill changed with left and right, or a row pressed with A); a
//! row that chooses from the game's own lists (a unit, a class, a skill, a
//! character, a chapter) opens a list page, scrolled like the Keyboard
//! Controls page. Actions that change the save ask first, with the game's
//! Yes/No dialog look. What a row does is in crate::cheats.

use std::sync::Mutex;

use egui::Color32;

use super::kit::{Art, Canvas, TEXT_CREAM, TEXT_DARK};
use super::options::{Effect, Item, Page, PANEL_H, PANEL_Y, ROW, WIDTH};
use super::Nav;
use crate::cheats::{self as host, Action, STAT_NAMES};
use crate::settings::Settings;

pub const GROUPS: [&str; 5] = ["Units", "Battle", "Items & Resources", "Story & Supports", "Save States"]; // pulse: Save States (fates_ui/states.rs)

/// the rows, by id (Item::Cheat(id))
mod id {
    pub const GROUP: u16 = 900; // 900 + group
    pub const UNIT: u16 = 10;
    pub const CLASS: u16 = 11;
    pub const LEVEL: u16 = 12;
    pub const EXP: u16 = 13;
    pub const STAT: u16 = 14; // 14..21
    pub const MOV: u16 = 22;
    pub const CAPS: u16 = 23;
    pub const MAX_LEVEL: u16 = 24;
    pub const RANKS: u16 = 25;
    pub const SKILL: u16 = 26;
    pub const JOIN: u16 = 27;
    pub const CRIT: u16 = 40;
    pub const HIT: u16 = 41;
    pub const MISS: u16 = 42;
    pub const GOD: u16 = 43;
    pub const OHKO: u16 = 44;
    pub const AGAIN: u16 = 45;
    pub const MOVE: u16 = 46;
    pub const VEINS: u16 = 47;
    pub const TURN: u16 = 48;
    pub const EXP_MULT: u16 = 49;
    pub const GROWTH: u16 = 50;
    pub const WEAPONS: u16 = 51;
    pub const VEIN_REUSE: u16 = 52; // cheats2
    pub const CAPTURE: u16 = 53; // cheats2
    pub const GOLD: u16 = 60;
    pub const CONVOY: u16 = 61;
    pub const SEALS: u16 = 62;
    pub const CASTLE: u16 = 63;
    pub const BUILDINGS: u16 = 64; // cheats2
    pub const GIVE_ITEM: u16 = 65; // castlegifts
    pub const LOG: u16 = 80;
    pub const SUPPORT_A: u16 = 81;
    pub const SUPPORT_B: u16 = 82;
    pub const SUPPORT_RANK: u16 = 83;
    pub const SUPPORT_SET: u16 = 84;
    pub const CHAPTER: u16 = 85;
    pub const PICK: u16 = 1000; // 1000 + index in the list
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PickKind {
    Unit,
    Class,
    Skill,
    SkillSlot,
    Character,
    SupportA,
    SupportB,
    Chapter,
    Item, // castlegifts
}

/// what the tab remembers between frames
pub struct CheatUi {
    pub pick: Option<PickKind>,
    /// the list shown, and what each entry stands for
    pub labels: Vec<(String, Option<String>)>,
    pub values: Vec<u32>,
    /// the page and row a list was opened from
    pub from: (u8, usize),
    pub skill: u16,
    pub support: [u32; 2],
    pub support_rank: u32,
    /// a question waiting: its text, the answer chosen, and the action
    pub confirm: Option<(String, bool, Action)>,
}

impl Default for CheatUi {
    fn default() -> Self {
        CheatUi { pick: None, labels: Vec::new(), values: Vec::new(), from: (0, 0), skill: 0, support: [0, 0], support_rank: 3, confirm: None }
    }
}

static UI: Mutex<Option<CheatUi>> = Mutex::new(None);

pub fn with_ui<R>(f: impl FnOnce(&mut CheatUi) -> R) -> R {
    let mut guard = UI.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(CheatUi::default))
}

pub fn groups() -> Vec<Item> {
    (0..GROUPS.len() as u16).map(|g| Item::Cheat(id::GROUP + g)).collect()
}

pub fn items(group: u8) -> Vec<Item> {
    use id::*;
    if group == super::states::GROUP {
        return super::states::items(); // pulse
    }
    let list: Vec<u16> = match group {
        0 => {
            let mut v = vec![UNIT, CLASS, LEVEL, EXP];
            v.extend((0..8).map(|i| STAT + i));
            v.extend([MOV, CAPS, MAX_LEVEL, RANKS, SKILL, JOIN]);
            v
        }
        1 => vec![CRIT, HIT, MISS, GOD, OHKO, AGAIN, MOVE, VEINS, VEIN_REUSE, CAPTURE, TURN, EXP_MULT, GROWTH, WEAPONS],
        2 => vec![GOLD, GIVE_ITEM, CONVOY, SEALS, CASTLE, BUILDINGS], // castlegifts: GIVE_ITEM
        _ => vec![LOG, SUPPORT_A, SUPPORT_B, SUPPORT_RANK, SUPPORT_SET, CHAPTER],
    };
    list.into_iter().map(Item::Cheat).collect()
}

pub fn pick_items() -> Vec<Item> {
    let n = with_ui(|ui| ui.labels.len());
    if n == 0 {
        return vec![Item::Cheat(id::PICK)];
    }
    (0..n as u16).map(|i| Item::Cheat(id::PICK + i)).collect()
}

pub fn title(page: Page) -> &'static str {
    match page {
        Page::Cheats(g) => GROUPS.get(g as usize).copied().unwrap_or("Cheats"),
        Page::Pick => match with_ui(|ui| ui.pick) {
            Some(PickKind::Unit) => "Choose a Unit",
            Some(PickKind::Class) => "Change Class",
            Some(PickKind::Skill) => "Give a Skill",
            Some(PickKind::SkillSlot) => "Where the Skill Goes",
            Some(PickKind::Character) => "Add a Character",
            Some(PickKind::SupportA) | Some(PickKind::SupportB) => "Choose a Unit",
            Some(PickKind::Chapter) => "Chapter Select",
            Some(PickKind::Item) => "Add an Item", // castlegifts
            None => "Cheats",
        },
        _ => "Cheats",
    }
}

fn on_off(v: bool) -> Option<String> {
    Some(if v { "On" } else { "Off" }.to_owned())
}

fn chosen_unit() -> Option<host::UnitView> {
    host::with_state(|s| s.roster.iter().find(|u| u.addr == s.chosen).cloned())
}

/// a row's label, value and help line
pub fn row(cid: u16, settings: &Settings) -> (String, Option<String>, String) {
    let c = &settings.pc.cheats;
    let s = |t: &str| t.to_owned();
    let unit = chosen_unit();
    let view = host::with_state(|s| s.view);
    let no_game = "Load a save first: the cheats work on the game being played.";
    let unit_help = |t: &str| if unit.is_some() { t.to_owned() } else { no_game.to_owned() };
    if super::states::is_ours(cid) {
        return super::states::row(cid, settings); // pulse
    }
    if cid >= id::GROUP && cid < id::GROUP + GROUPS.len() as u16 {
        let g = (cid - id::GROUP) as usize;
        let help = [
            "Change a unit's class, stats, level, weapon ranks and skills, or add any character to your army.",
            "Critical hits, hit rates, damage, movement, growth and EXP, in the game's own battle calculation.",
            "Gold, any item, the convoy, seals and My Castle's resources.", // castlegifts: any item
            "Every support conversation in the Support Log, support levels, and chapter select.",
            "Save and load the whole game at any moment, in eight slots, with quick save and quick load keys.", // pulse
        ][g];
        return (s(GROUPS[g]), None, s(help));
    }
    if cid >= id::PICK {
        let i = (cid - id::PICK) as usize;
        return with_ui(|ui| match ui.labels.get(i) {
            Some((label, value)) => (label.clone(), value.clone(), pick_help(ui.pick)),
            None => (s("Nothing to choose here"), None, s(no_game)),
        });
    }
    if (id::STAT..id::STAT + 8).contains(&cid) {
        let i = (cid - id::STAT) as usize;
        let value = unit.as_ref().filter(|u| view.unit == u.addr).map(|_| format!("{}  /  {}", view.stats[i], view.caps[i]));
        let help = if c.ignore_caps {
            format!("{}: the unit's own {}, before weapons and skills. Over the cap raises the cap for this unit (up to 99).", STAT_NAMES[i], STAT_NAMES[i])
        } else {
            format!("{}: the unit's own {}, before weapons and skills, up to the class's cap.", STAT_NAMES[i], STAT_NAMES[i])
        };
        return (s(STAT_NAMES[i]), value, unit_help(&help));
    }
    match cid {
        id::UNIT => (
            s("Unit"),
            Some(unit.as_ref().map_or_else(|| s("None"), |u| u.name.clone())),
            unit_help("The unit the rows below change. Press to choose from your army."),
        ),
        id::CLASS => (
            s("Class"),
            unit.as_ref().map(|u| u.class_name.clone()),
            unit_help("Press to change the class, as a seal does: the level stays, stats follow the class, new weapon types start at E."),
        ),
        id::LEVEL => (s("Level"), unit.as_ref().map(|u| format!("{}  /  {}", u.level, if view.unit == u.addr { view.level_cap } else { 0 })), unit_help("The level, without growing stats. Max Level levels up with the game's own growth rolls.")),
        id::EXP => (s("EXP"), unit.as_ref().map(|u| u.exp.to_string()), unit_help("The EXP toward the next level (100 levels up).")),
        id::MOV => (s("Mov"), unit.as_ref().filter(|u| view.unit == u.addr).map(|_| view.mov.to_string()), unit_help("Movement, before skills and pair-ups.")),
        id::CAPS => (s("Ignore Stat Caps"), on_off(c.ignore_caps), s("Let stats go past the class's caps, up to 99, by raising this unit's own caps.")),
        id::MAX_LEVEL => (s("Max Level"), None, unit_help("Level up to the class's cap, with the game's own level-up rolls.")),
        id::RANKS => (s("Max Weapon Ranks"), None, unit_help("Every weapon type the class uses at its highest rank.")),
        id::SKILL => (s("Give Skill"), None, unit_help("Any skill, in one of the five slots or as an extra skill beyond them.")),
        id::JOIN => (s("Add Character"), None, s("Any character from any route, the children and the amiibo heroes, joining as My Castle's joins do.")),
        id::CRIT => (
            s("100% Critical"),
            Some(s(match c.crit {
                1 => "Your Units",
                2 => "Everyone",
                _ => "Off",
            })),
            s(match c.crit {
                2 => "Every attack by anyone, enemies too, is a critical hit. The forecast shows it.",
                _ => "Every attack by your units and allies is a critical hit. The forecast shows it.",
            }),
        ),
        id::HIT => (s("100% Hit (Your Units)"), on_off(c.always_hit), s("Your units and allies never miss. The forecast shows it.")),
        id::MISS => (s("Enemies Always Miss"), on_off(c.enemies_miss), s("Enemy attacks never hit your units or allies. The forecast shows it.")),
        id::GOD => (s("No Damage Taken"), on_off(c.no_damage), s("Enemy attacks do no damage to your units or allies.")),
        id::OHKO => (s("One-Hit Kills"), on_off(c.one_hit_kill), s("A hit from one of your units or allies defeats the enemy.")),
        id::AGAIN => (s("Move Again"), on_off(c.move_again), s("Your units can act again after acting. End the turn from the map menu.")),
        id::MOVE => (s("Unlimited Movement"), on_off(c.unlimited_move), s("Your units can move anywhere they could walk to.")),
        id::VEINS => (s("Dragon Veins: Anyone"), on_off(c.dragon_veins), s("Any of your units can use Dragon Veins, not only royals.")),
        id::VEIN_REUSE => (
            s("Dragon Veins: Reuse"),
            on_off(c.vein_reuse),
            s("A Dragon Vein you used is ready again at your next turn, so each vein works once a turn. Its effect happens again each time."),
        ),
        id::CAPTURE => (
            s("Capture: No Prison"),
            on_off(c.capture_recruit),
            s("All your units can Capture, Prison or not. A captured enemy joins your army at once, as when scouted in the Prison."),
        ),
        id::TURN => (s("Freeze Turn Count"), on_off(c.freeze_turns), s("The turn count stays the same. Maps won by surviving turns can't be won while this is on.")),
        id::EXP_MULT => (s("EXP Multiplier"), Some(format!("{}\u{d7}", c.exp_multiplier.max(1))), s("Your units gain this many times the EXP from battles and staves.")),
        id::GROWTH => (s("Perfect Level-Ups"), on_off(c.perfect_growth), s("Every stat goes up on each level-up, as far as its cap allows.")),
        id::WEAPONS => (s("Any Class, Any Weapon"), on_off(c.every_weapon), s("Your units can use every weapon type. Use Max Weapon Ranks to wield the strongest.")),
        id::GOLD => (s("Max Gold"), host::with_state(|s| s.in_game.then(|| format!("{}", s.gold))), s("Set your gold to 999,999.")),
        // castlegifts
        id::GIVE_ITEM => (s("Add an Item"), None, s("Any item the game has, one at a time, into the convoy: the online Battle and Visit Bonuses too.")),
        id::CONVOY => (s("Fill the Convoy"), None, s("99 of every item in the convoy.")),
        id::CASTLE => (s("Max My Castle Resources"), None, s("99 of every ore and food, and 99 Dragon Vein Points to build and upgrade with.")),
        id::BUILDINGS => (
            s("Max Buildings"),
            None,
            s("Level up every building to its highest level with the castle's own upgrade, free of Dragon Vein Points. Use it in My Castle; press A after each one."),
        ),
        id::SEALS => (s("Unlimited Seals"), on_off(c.unlimited_seals), s("Heart, Partner, Friendship and Eternal Seals are not used up.")),
        id::LOG => (
            s("Support Log: Everything"),
            on_off(c.support_log),
            s("The Support Log lists every conversation the game has, for any pair and rank, either Corrin. Open it in My Castle's Records Hall."),
        ),
        id::SUPPORT_A | id::SUPPORT_B => {
            let which = (cid - id::SUPPORT_A) as usize;
            let addr = with_ui(|ui| ui.support[which]);
            let name = host::with_state(|s| s.roster.iter().find(|u| u.addr == addr).map(|u| u.name.clone()));
            (s(if which == 0 { "Support: First Unit" } else { "Support: Second Unit" }), Some(name.unwrap_or_else(|| s("None"))), s("Press to choose from your army."))
        }
        id::SUPPORT_RANK => {
            let r = with_ui(|ui| ui.support_rank);
            (s("Support: Rank"), Some(s(["None", "C", "B", "A", "S"][r.min(4) as usize])), s("The rank to set. S only between a pair that can marry, as the game allows."))
        }
        id::SUPPORT_SET => (s("Set Support Rank"), None, s("Set the two units' support to the rank chosen, both ways, without the conversations.")),
        id::CHAPTER => (
            s("Chapter Select"),
            None,
            s("Start any chapter from My Castle, as Next Battle does. Save to a new slot afterwards: your progress changes."),
        ),
        _ => (s(""), None, s("")),
    }
}

fn pick_help(kind: Option<PickKind>) -> String {
    match kind {
        Some(PickKind::Class) => "Any class, other routes', special and DLC classes too. A class for the other gender becomes its counterpart.",
        Some(PickKind::Skill) => "Any skill the game has.",
        Some(PickKind::SkillSlot) => "A slot replaces the skill there; Extra adds it beyond the five.",
        Some(PickKind::Character) => "They join with their own class, level and items, as when they join in the story.",
        Some(PickKind::Chapter) => "The chapter starts from its opening and Battle Preparations. Choose in My Castle.",
        Some(PickKind::Item) => "One more of it goes to the convoy each time you press, up to 99. The note says which Inbox reward it was online.", // castlegifts
        _ => "Press to choose.",
    }
    .to_owned()
}

pub fn changes(cid: u16) -> bool {
    if super::states::is_ours(cid) {
        return super::states::changes(cid); // pulse
    }
    use id::*;
    matches!(cid, LEVEL | EXP | MOV | CAPS | CRIT | HIT | MISS | GOD | OHKO | AGAIN | MOVE | VEINS | VEIN_REUSE | CAPTURE | TURN | EXP_MULT | GROWTH | WEAPONS | SEALS | LOG | SUPPORT_RANK)
        || (STAT..STAT + 8).contains(&cid)
}

/// a value changed with left and right (dir -1 or 1)
pub fn adjust(cid: u16, dir: i32, settings: &mut Settings) -> Option<Effect> {
    if super::states::is_ours(cid) {
        return super::states::adjust(cid, dir, settings); // pulse
    }
    let c = &mut settings.pc.cheats;
    let d = if dir == 0 { 1 } else { dir };
    let unit = chosen_unit();
    let view = host::with_state(|s| s.view);
    if (id::STAT..id::STAT + 8).contains(&cid) {
        let u = unit?;
        if view.unit != u.addr {
            return None;
        }
        let i = (cid - id::STAT) as usize;
        host::request(Action::SetStat { unit: u.addr, stat: i, value: view.stats[i] + d });
        host::with_state(|s| s.view.stats[i] = (view.stats[i] + d).clamp(0, if c.ignore_caps { 99 } else { view.caps[i] }));
        return None;
    }
    match cid {
        id::LEVEL => {
            let u = unit?;
            host::request(Action::SetLevel { unit: u.addr, value: u.level as i32 + d });
            None
        }
        id::EXP => {
            let u = unit?;
            host::request(Action::SetExp { unit: u.addr, value: (u.exp as i32 + d).rem_euclid(100) });
            None
        }
        id::MOV => {
            let u = unit?;
            host::request(Action::SetMove { unit: u.addr, value: view.mov + d });
            None
        }
        id::SUPPORT_RANK => {
            with_ui(|ui| ui.support_rank = (ui.support_rank as i32 + d).rem_euclid(5) as u32);
            None
        }
        _ => {
            match cid {
                id::CAPS => c.ignore_caps = !c.ignore_caps,
                id::CRIT => c.crit = (c.crit as i32 + d).rem_euclid(3) as u8,
                id::HIT => c.always_hit = !c.always_hit,
                id::MISS => c.enemies_miss = !c.enemies_miss,
                id::GOD => c.no_damage = !c.no_damage,
                id::OHKO => c.one_hit_kill = !c.one_hit_kill,
                id::AGAIN => c.move_again = !c.move_again,
                id::MOVE => c.unlimited_move = !c.unlimited_move,
                id::VEINS => c.dragon_veins = !c.dragon_veins,
                id::VEIN_REUSE => c.vein_reuse = !c.vein_reuse,
                id::CAPTURE => c.capture_recruit = !c.capture_recruit,
                id::TURN => c.freeze_turns = !c.freeze_turns,
                id::GROWTH => c.perfect_growth = !c.perfect_growth,
                id::WEAPONS => c.every_weapon = !c.every_weapon,
                id::SEALS => c.unlimited_seals = !c.unlimited_seals,
                id::LOG => c.support_log = !c.support_log,
                id::EXP_MULT => {
                    const STEPS: [u32; 4] = [1, 2, 5, 10];
                    let at = STEPS.iter().position(|v| *v == c.exp_multiplier.max(1)).unwrap_or(0) as i32;
                    c.exp_multiplier = STEPS[(at + d).rem_euclid(4) as usize];
                }
                _ => return None,
            }
            Some(Effect::Apply)
        }
    }
}

/// what pressing a row does in the menu
pub enum Press {
    /// a page opens, with its first row
    Open(Page),
    /// back to the page the list came from, on its row
    Back(Page, usize),
    Nothing,
}

fn open_pick(kind: PickKind, from: (u8, usize)) -> Press {
    let (labels, values): (Vec<(String, Option<String>)>, Vec<u32>) = host::with_state(|s| match kind {
        PickKind::Unit | PickKind::SupportA | PickKind::SupportB => {
            s.roster.iter().map(|u| ((u.name.clone(), Some(format!("{} Lv {}", u.class_name, u.level))), u.addr)).unzip()
        }
        PickKind::Class => {
            let female = s.roster.iter().find(|u| u.addr == s.chosen).is_some_and(|u| u.female);
            // one entry per class, in the unit's gender where there is one
            let mut seen = std::collections::HashSet::new();
            let mut out = Vec::new();
            for c in &s.classes {
                if c.female != female && s.classes.iter().any(|o| o.id == c.other && o.female == female) {
                    continue;
                }
                if !seen.insert(c.name.clone()) && c.female != female {
                    continue;
                }
                let tag = if c.female != female { Some(if c.female { "Women only".to_owned() } else { "Men only".to_owned() }) } else { Some(if c.promoted { "Promoted".to_owned() } else { "Base".to_owned() }) };
                out.push(((c.name.clone(), tag), c.id as u32));
            }
            out.into_iter().unzip()
        }
        PickKind::Skill => {
            let mut v: Vec<&host::SkillEntry> = s.skills.iter().collect();
            v.sort_by(|a, b| a.name.cmp(&b.name));
            v.dedup_by(|a, b| a.name == b.name);
            v.into_iter().map(|k| ((k.name.clone(), None), k.id as u32)).unzip()
        }
        PickKind::SkillSlot => {
            let unit = s.roster.iter().find(|u| u.addr == s.chosen);
            let mut out = Vec::new();
            for slot in 0..5 {
                let current = unit.map(|u| u.skills[slot]).unwrap_or(0);
                let name = s.skills.iter().find(|k| k.id == current).map_or_else(|| "Empty".to_owned(), |k| k.name.clone());
                out.push(((format!("Slot {}", slot + 1), Some(name)), slot as u32));
            }
            out.push((("Extra Skill".to_owned(), Some("Beyond the five".to_owned())), 5));
            out.into_iter().unzip()
        }
        PickKind::Character => s
            .persons
            .iter()
            .map(|p| {
                let note = if s.roster.iter().any(|u| u.person == p.addr) {
                    Some("In your army".to_owned())
                } else if p.guest {
                    Some("amiibo".to_owned())
                } else if p.child {
                    Some("Child".to_owned())
                } else {
                    None
                };
                ((p.name.clone(), note), p.id as u32)
            })
            .unzip(),
        PickKind::Chapter => s
            .chapters
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let route = match c.route {
                    0 => "Birthright",
                    1 => "Conquest",
                    2 => "Revelation",
                    _ => "",
                };
                let prefix = c.prefix.replace("Chapter ", "Ch. ").replace("Paralogue ", "Par. ");
                let label = if prefix.is_empty() { c.name.clone() } else { format!("{prefix}: {}", c.name) };
                ((label, (!route.is_empty()).then(|| route.to_owned())), i as u32)
            })
            .unzip(),
        // castlegifts: every item the game names, in its own order (by
        // kind), noted when the Inbox's online rewards gave it
        PickKind::Item => s.items.iter().map(|(id, name)| ((name.clone(), s.bonus_items.get(id).cloned()), *id as u32)).unzip(),
    });
    with_ui(|ui| {
        ui.pick = Some(kind);
        ui.labels = labels;
        ui.values = values;
        ui.from = from;
    });
    Press::Open(Page::Pick)
}

/// A on a row (or on a list's entry)
pub fn press(cid: u16, page: Page, row: usize) -> Press {
    let group = match page {
        Page::Cheats(g) => g,
        _ => 0,
    };
    if cid >= id::GROUP && cid < id::GROUP + GROUPS.len() as u16 {
        return Press::Open(Page::Cheats((cid - id::GROUP) as u8));
    }
    if super::states::is_ours(cid) {
        return Press::Nothing; // pulse: options.rs passes the settings to states::press
    }
    let unit = chosen_unit();
    let in_game = host::with_state(|s| s.in_game);
    if cid >= id::PICK {
        let i = (cid - id::PICK) as usize;
        let (kind, value, from, label) = with_ui(|ui| (ui.pick, ui.values.get(i).copied(), ui.from, ui.labels.get(i).map(|l| l.0.clone()).unwrap_or_default()));
        let Some(value) = value else { return Press::Back(Page::Cheats(from.0), from.1) };
        let back = Press::Back(Page::Cheats(from.0), from.1);
        match kind {
            Some(PickKind::Unit) => host::with_state(|s| s.chosen = value),
            Some(PickKind::SupportA) => with_ui(|ui| ui.support[0] = value),
            Some(PickKind::SupportB) => with_ui(|ui| ui.support[1] = value),
            Some(PickKind::Class) => {
                if let Some(u) = unit {
                    host::request(Action::ChangeClass { unit: u.addr, class: value as u16 });
                }
            }
            Some(PickKind::Skill) => {
                with_ui(|ui| ui.skill = value as u16);
                return open_pick(PickKind::SkillSlot, from);
            }
            Some(PickKind::SkillSlot) => {
                let skill = with_ui(|ui| ui.skill);
                if let Some(u) = unit {
                    host::request(Action::GiveSkill { unit: u.addr, skill, slot: value as usize });
                }
            }
            Some(PickKind::Character) => {
                let text = format!("Add {label} to your army? They join as they would in the story. Save to keep them.");
                with_ui(|ui| ui.confirm = Some((text, true, Action::AddCharacter { person: value as u16 })));
            }
            Some(PickKind::Item) => {
                // castlegifts: stays on the list, so more can be added
                host::request(Action::GiveItem { item: value as u16 });
                return Press::Nothing;
            }
            Some(PickKind::Chapter) => {
                let in_castle = host::with_state(|s| s.in_castle);
                if !in_castle {
                    host::with_state(|s| s.notices.push_back("Chapter Select works from My Castle.".to_owned()));
                    return back;
                }
                let text = format!(
                    "Go to {label}? The chapter starts now. Your story progress will change, so save to a NEW slot afterwards to keep your current save."
                );
                with_ui(|ui| ui.confirm = Some((text, false, Action::Chapter { index: value as usize })));
            }
            None => {}
        }
        return back;
    }
    let from = (group, row);
    if !in_game && cid != id::CAPS {
        host::with_state(|s| s.notices.push_back("Load a save first.".to_owned()));
        return Press::Nothing;
    }
    match cid {
        id::UNIT => open_pick(PickKind::Unit, from),
        id::CLASS => open_pick(PickKind::Class, from),
        id::SKILL => open_pick(PickKind::Skill, from),
        id::JOIN => open_pick(PickKind::Character, from),
        id::SUPPORT_A => open_pick(PickKind::SupportA, from),
        id::SUPPORT_B => open_pick(PickKind::SupportB, from),
        id::CHAPTER => open_pick(PickKind::Chapter, from),
        id::GIVE_ITEM => open_pick(PickKind::Item, from), // castlegifts
        id::MAX_LEVEL => {
            if let Some(u) = unit {
                host::request(Action::MaxLevel { unit: u.addr });
            }
            Press::Nothing
        }
        id::RANKS => {
            if let Some(u) = unit {
                host::request(Action::MaxRanks { unit: u.addr });
            }
            Press::Nothing
        }
        id::GOLD => {
            host::request(Action::MaxGold);
            Press::Nothing
        }
        id::CASTLE => {
            host::request(Action::MaxCastle);
            Press::Nothing
        }
        id::BUILDINGS => {
            let in_castle = host::with_state(|s| s.in_castle);
            if !in_castle {
                host::with_state(|s| s.notices.push_back("Max Buildings works from My Castle.".to_owned()));
                return Press::Nothing;
            }
            let text = "Level up every building in My Castle to its highest level? Shops and their stock follow. This can't be undone, so save to a NEW slot afterwards to keep your current save.";
            with_ui(|ui| ui.confirm = Some((text.to_owned(), false, Action::MaxBuildings)));
            Press::Nothing
        }
        id::CONVOY => {
            with_ui(|ui| ui.confirm = Some(("Put 99 of every item in the convoy?".to_owned(), true, Action::FillConvoy)));
            Press::Nothing
        }
        id::SUPPORT_SET => {
            let ([a, b], rank) = with_ui(|ui| (ui.support, ui.support_rank));
            if a == 0 || b == 0 || a == b {
                host::with_state(|s| s.notices.push_back("Choose two different units first.".to_owned()));
            } else {
                host::request(Action::SetSupport { a, b, level: rank });
            }
            Press::Nothing
        }
        _ => Press::Nothing,
    }
}

// ---- the Yes/No question --------------------------------------------------------

const CONFIRM_W: f32 = 300.0;

pub fn confirming() -> bool {
    with_ui(|ui| ui.confirm.is_some())
}

fn layout(canvas: &Canvas, text: &str) -> (f32, f32, f32, Vec<String>, [f32; 2]) {
    let lines = canvas.kit.wrap_even(text, CONFIRM_W - 36.0, 0.9);
    let height = lines.len() as f32 * 15.0 + 22.0 + 2.0 * 19.0 + 10.0;
    let x = (WIDTH - CONFIRM_W) / 2.0;
    let y = PANEL_Y + (PANEL_H - height) / 2.0;
    let answers = y + 12.0 + lines.len() as f32 * 15.0 + 6.0;
    (x, y, height, lines, [answers, answers + 19.0])
}

/// the question takes the input while it is up
pub fn confirm_nav(canvas: &Canvas, input: &egui::InputState, nav: &Nav) {
    let Some((text, yes, action)) = with_ui(|ui| ui.confirm.clone()) else { return };
    let mut yes = yes;
    let mut answer = None;
    if nav.up || nav.down || nav.left || nav.right {
        yes = !yes;
    }
    if nav.back || nav.close || input.pointer.secondary_clicked() {
        answer = Some(false);
    } else if nav.confirm {
        answer = Some(yes);
    } else if let Some(pos) = input.pointer.hover_pos() {
        let (x, _, _, _, rows) = layout(canvas, &text);
        let (px, py) = canvas.unit_of(pos);
        for (index, top) in rows.iter().enumerate() {
            if (x + 30.0..x + CONFIRM_W - 30.0).contains(&px) && (*top..top + 19.0).contains(&py) {
                if input.pointer.delta() != egui::Vec2::ZERO {
                    yes = index == 0;
                }
                if input.pointer.primary_clicked() {
                    answer = Some(index == 0);
                }
            }
        }
    }
    with_ui(|ui| {
        if let Some(c) = &mut ui.confirm {
            c.1 = yes;
        }
        if answer.is_some() {
            ui.confirm = None;
        }
    });
    if answer == Some(true) {
        host::request(action);
    }
}

pub fn draw_confirm(canvas: &Canvas, now: f64) {
    let Some((text, yes, _)) = with_ui(|ui| ui.confirm.clone()) else { return };
    let white = Color32::WHITE;
    canvas.fill(0.0, 0.0, WIDTH, super::options::HEIGHT, Color32::from_black_alpha(90));
    let (x, y, height, lines, rows) = layout(canvas, &text);
    canvas.nine(Art::Dialog, x, y, CONFIRM_W, height, [22.0, 22.0, 22.0, 22.0], 0.5, white);
    for (index, line) in lines.iter().enumerate() {
        canvas.text_centered(x + CONFIRM_W / 2.0, y + 11.0 + index as f32 * 15.0, line, 0.9, TEXT_DARK);
    }
    let bob = ((now * 6.0).sin() * 1.5) as f32;
    for (index, (top, label)) in rows.iter().zip(["Yes", "No"]).enumerate() {
        let chosen = (index == 0) == yes;
        if chosen {
            canvas.three(Art::Highlight, x + 70.0, *top - 1.0, CONFIRM_W - 140.0, ROW + 1.0, [10.0, 10.0], white);
            canvas.sprite(Art::Hand, x + 50.0 + bob, top + 2.5, white);
        }
        canvas.text_centered(x + CONFIRM_W / 2.0, top + 2.0, label, 0.95, if chosen { TEXT_CREAM } else { TEXT_DARK });
    }
}

/// snapshots: a list or a question, as if chosen
#[allow(dead_code)]
pub fn snapshot_pick(kind: PickKind) {
    let _ = open_pick(kind, (0, 0));
}

#[allow(dead_code)]
pub fn snapshot_confirm(text: &str) {
    with_ui(|ui| ui.confirm = Some((text.to_owned(), false, Action::MaxGold)));
}

#[allow(dead_code)]
pub fn snapshot_reset() {
    with_ui(|ui| ui.confirm = None);
}
