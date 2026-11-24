//! qol: quality-of-life options for Fates.
//!
//! - **Game speed** (`pc.game_speed`, 1x to 8x and Max): the game runs that
//!   many of its refreshes in each one of the console's. The frontend raises
//!   refreshes `speed` times as often (`frame_period`, used by both of its
//!   loops), and in real time the guest clock runs `speed` times as fast as
//!   the host's (`zakuro_core::pcos::rt::set_speed`), so the game's own
//!   clock, its VBlanks, its timers and its sound all stay together. The
//!   pictures still go out at the display's rate, each the newest (or, at
//!   2x on a fast display, the one in between) the game made. Sound: up to
//!   4x it is time-stretched (shorter, at its own pitch, the WSOLA the sound
//!   output already uses to ride out late frames); above that it is muted.
//!   The Game Speed key (keyboard `, a controller button bound to it)
//!   switches between 1x and `pc.game_speed_fast`.
//! - **Skip Story** (`pc.skip_story`): overrides/qol.c starts the game's own
//!   skip the moment a story event becomes skippable. While a skip runs (a
//!   black screen) the game runs at Max, so it passes quickly. qol3: the
//!   map's opening talk (before the first player phase) is skipped too; the
//!   battle's own talk and boss quotes are not (`opening_types`).
//! - **The SpotPass questions** after the avatar is made: always answered No
//!   by overrides/qol.c (qol3: SpotPass does nothing on PC, so there is no
//!   setting any more; `FATES_SPOTPASS_PROMPTS=1` asks them, for the old
//!   replay).
//! - qol3: **Exit to Desktop** in the title's Main Menu (overrides/mods.c),
//!   which asks with the options' own Exit to Desktop question.
//! - qol3: **The title screen's logo** is always the plain "Fire Emblem
//!   Fates" one (the game's Tlp_Title_P), not the last route's.
//! - **Boon and bane stats** (`pc.boon_bane_stats`): what the chosen boon
//!   and bane change, read from the game's own Personality table, shown by
//!   the interface (fates_ui/qol.rs) beside the avatar's menu.

use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crate::settings::PcSettings;

/// the words of overrides/qol.c the host reads and writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Word {
    SkipStory,
    SkipSpotpass,
    ShowChoices,
    SkipCutscenes,
    ChoiceMode,
    SkipsStarted,
    CutscenesSkipped,
    ChoicesShown,
    SpotpassAnswered,
    CharaDraws,
    CharaSelected,
    CharaSelectedAt,
    CharaBoon,
    CharaBane,
    CharaBoonText,
    CharaBaneText,
    MoviesSkipped,
    SkipTutorialMaps,
    SkipSupports,
    StoryTypes,
    CastleItems,
    DialogOpen,
    SkipOurs,
    QuitRequest,
    // qol3
    OpeningTypes,
    TitleExit,
    TitleExitRequest,
    TitlePlain,
    TitlePlainShown,
    // qol4 (see qol4.rs)
    TitleOptions,
    TitleOptionsRequest,
    MapItems,
    SoftReset,
    RestartRequest,
    Restarts,
    ExtReadQuiet,
    ExtReadHidden,
    // boot3 (overrides/boot3.c, see boot3.rs)
    ProcThreadGuard,
    ProcThreadHeld,
    ProcThreadSleeps,
    ProcThreadGaveUp,
    // transblur: overrides/transblur.c, the battle transitions' zoom blur
    TransitionBlur,
    TransitionUnblurred,
    // drawdist: overrides/drawdist.c, Full Draw Distance
    FullDrawDistance,
    // castlegifts: overrides/castlegifts.c, Online Rewards
    OnlineRewards,
    RewardsUnlocked,
    PresentsGiven,
    // bossmusic: overrides/bossmusic.c, Boss Music Stays, and its counts
    BossMusic,
    BossEngaged,
    BossKept,
    BossPlayed,
    BossHeld,
    BossBack,
}

/// what the game build exposes of overrides/qol.c, registered by the
/// program that links the recompiled code.
#[derive(Clone, Copy)]
pub struct Hooks {
    pub word: fn(Word) -> *mut u32,
}

static HOOKS: OnceLock<Hooks> = OnceLock::new();

/// called once by the program, before zakuro::run.
pub fn set_hooks(hooks: Hooks) {
    let _ = HOOKS.set(hooks);
}

pub fn read(word: Word) -> Option<u32> {
    // SAFETY: a plain word of the game build; the game's threads only write
    // it while they hold the baton, and a torn read cannot happen on a u32
    HOOKS.get().map(|hooks| unsafe { std::ptr::read_volatile((hooks.word)(word)) })
}

pub fn write(word: Word, value: u32) {
    if let Some(hooks) = HOOKS.get() {
        // SAFETY: as above
        unsafe { std::ptr::write_volatile((hooks.word)(word), value) }
    }
}

fn env_flag(name: &str) -> Option<bool> {
    std::env::var(name).ok().map(|value| !matches!(value.as_str(), "0" | "off" | "false" | ""))
}

/// whether the SpotPass questions are skipped: always (qol3: SpotPass does
/// nothing on PC, so it is no longer a setting), unless
/// FATES_SPOTPASS_PROMPTS=1 has them asked (the old full replay,
/// dev/os-save3-prompts.rec, answers them).
pub fn skips_spotpass() -> bool {
    !env_flag("FATES_SPOTPASS_PROMPTS").unwrap_or(false)
}

/// whether Skip Story is on: the setting, or FATES_SKIP_STORY.
pub fn skips_story(pc: &PcSettings) -> bool {
    env_flag("FATES_SKIP_STORY").unwrap_or(pc.skip_story)
}

/// transblur: whether the battle transitions keep the game's zoom blur:
/// the setting, or FATES_TRANSITION_BLUR.
pub fn transition_blur(pc: &PcSettings) -> bool {
    env_flag("FATES_TRANSITION_BLUR").unwrap_or(pc.transition_blur)
}

/// drawdist: whether grass and the field's objects are drawn at any
/// distance: the setting, or FATES_DRAWDIST.
pub fn full_draw_distance(pc: &PcSettings) -> bool {
    env_flag("FATES_DRAWDIST").unwrap_or(pc.full_draw_distance)
}

/// castlegifts: whether the online-only gifts are given (Online Rewards):
/// the setting, or FATES_ONLINE_REWARDS.
pub fn online_rewards(pc: &PcSettings) -> bool {
    env_flag("FATES_ONLINE_REWARDS").unwrap_or(pc.online_rewards)
}

/// bossmusic: whether a fought boss's theme stays on as the map's music:
/// the setting, or FATES_BOSS_MUSIC (read by overrides/bossmusic.c itself).
pub fn boss_music(pc: &PcSettings) -> bool {
    env_flag("FATES_BOSS_MUSIC").unwrap_or(pc.boss_music)
}

/// puts the game-level options to use, at start and after each change.
pub fn apply(pc: &PcSettings) {
    if HOOKS.get().is_none() {
        return;
    }
    let story = skips_story(pc);
    let spotpass = skips_spotpass();
    write(Word::SkipStory, story as u32);
    write(Word::SkipSpotpass, spotpass as u32);
    // for tests: FATES_QOL_CHOICES=0 lets a skip answer choices as the
    // game's own does; FATES_QOL_CUTSCENES=0 leaves 3D cutscenes to their
    // own Start; FATES_QOL_CHOICE_MODE=2 waits for the script's next part
    write(Word::ShowChoices, env_flag("FATES_QOL_CHOICES").unwrap_or(true) as u32);
    write(Word::SkipCutscenes, env_flag("FATES_QOL_CUTSCENES").unwrap_or(true) as u32);
    write(Word::SkipTutorialMaps, env_flag("FATES_QOL_TUTORIAL").unwrap_or(true) as u32);
    write(Word::StoryTypes, story_types());
    write(Word::OpeningTypes, opening_types()); // qol3: the map's opening talk
    // qol3: Exit to Desktop in the title's Main Menu (FATES_QOL_TITLE_EXIT=0 leaves it out)
    write(Word::TitleExit, env_flag("FATES_QOL_TITLE_EXIT").unwrap_or(true) as u32);
    // qol3: the title screen's logo is the plain Fates one, whatever route
    // was played last (FATES_QOL_TITLE_PLAIN=0 leaves the game's choice)
    // codemods2: off when a code mod changes Title::Create, whose title the
    // mod's own (FE1's Shadow Dragon logo): the mod's code wins
    let title_modded = zakuro_core::codemods::replaces(0x00420B74);
    write(Word::TitlePlain, (env_flag("FATES_QOL_TITLE_PLAIN").unwrap_or(true) && !title_modded) as u32);
    // the castle menu's Return to Title and Exit to Desktop (FATES_QOL_CASTLE=0 leaves them out)
    write(Word::CastleItems, env_flag("FATES_QOL_CASTLE").unwrap_or(true) as u32);
    // FATES_QOL_SUPPORTS=1 also skips support conversations a player opens
    write(Word::SkipSupports, env_flag("FATES_QOL_SUPPORTS").unwrap_or(false) as u32);
    write(Word::ChoiceMode, std::env::var("FATES_QOL_CHOICE_MODE").ok().and_then(|v| v.parse().ok()).unwrap_or(1));
    // transblur: the battle transitions' zoom blur (FATES_TRANSITION_BLUR=1/0 for tests)
    write(Word::TransitionBlur, transition_blur(pc) as u32);
    // drawdist: Full Draw Distance (FATES_DRAWDIST=1/0 for tests)
    write(Word::FullDrawDistance, full_draw_distance(pc) as u32);
    // castlegifts: Online Rewards (FATES_ONLINE_REWARDS=1/0 for tests)
    write(Word::OnlineRewards, online_rewards(pc) as u32);
    // bossmusic: Boss Music Stays (FATES_BOSS_MUSIC=1/0 for tests)
    write(Word::BossMusic, boss_music(pc) as u32);
    crate::qol4::apply(); // qol4: Options in the Main Menu, the map menu's items, no extra data message
    log::info!("qol: skip story {story}, SpotPass questions {}", if spotpass { "answered No" } else { "asked" });
}

/// the counts overrides/qol.c kept, for the end of a headless run.
pub fn summary() -> Option<String> {
    HOOKS.get()?;
    let r = |word| read(word).unwrap_or(0);
    Some(format!(
        "qol: skips started {}, movies skipped {}, cutscenes skipped {}, choices shown during skips {}, SpotPass questions answered {}, plain title logo in place of a route's {}",
        r(Word::SkipsStarted),
        r(Word::MoviesSkipped),
        r(Word::CutscenesSkipped),
        r(Word::ChoicesShown),
        r(Word::SpotpassAnswered),
        r(Word::TitlePlainShown)
    ) + &crate::qol4::summary() // qol4
        // transblur
        + &format!(", transition frames without the zoom blur {}", r(Word::TransitionUnblurred))
        // castlegifts
        + &format!(", online rewards: bonuses shown claimable {}, presents given {}", r(Word::RewardsUnlocked), r(Word::PresentsGiven))
        // bossmusic
        + &format!(
            "\nbossmusic: on {}, bosses engaged {}, themes kept after a battle {}, played in the map BGM's place {}, battles held {}, back to the map BGM {}",
            r(Word::BossMusic),
            r(Word::BossEngaged),
            r(Word::BossKept),
            r(Word::BossPlayed),
            r(Word::BossHeld),
            r(Word::BossBack)
        ))
}

// ---------------------------------------------------------------------------
// game speed
// ---------------------------------------------------------------------------

/// the speeds the options offer; 0 is Max.
pub const SPEEDS: [u32; 7] = [1, 2, 3, 4, 5, 8, 0];
/// how many refreshes a console refresh Max runs: the most a fast PC keeps
/// up with in the menus; where it cannot, the schedule falls behind and
/// starts over (CATCH_UP_LIMIT), so the game simply runs as fast as it can.
pub const MAX_SPEED: u32 = 16;
/// the fastest speed whose sound is played, time-stretched; faster ones
/// are muted (the stretch's pieces would be too far apart to follow).
pub const MOST_HEARD: u32 = 4;

/// the event function types that are story (event::Function::Type, the
/// byte at ProcEvent +0x5C): Opening 0x0C, Complete 0x0D (the scene after
/// a won map), Ending 0x0F. Everything else an event can be on a map is
/// the battle's own (Turn 0x10, TurnAfter 0x12, Talk 0x17, villages, boss
/// and battle talk...) and is never skipped. FATES_QOL_STORY_TYPES (hex
/// mask) replaces the list, for tests.
pub fn story_types() -> u32 {
    std::env::var("FATES_QOL_STORY_TYPES")
        .ok()
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .unwrap_or((1 << 0x0C) | (1 << 0x0D) | (1 << 0x0F))
}

/// qol3: the event types that, on a map still at turn 1 in the player's
/// phase (map::Situation +0x0C the turn, +0x08 the phase's force), are the
/// map's opening talk, the conversation before the battle: MapLoad 7 and
/// Turn 0x10. Skip Story skips them as story. Turn events of later turns
/// and of other phases, TurnAfter, Reinforce, Area, Talk (boss quotes,
/// villages, talk between units) are the battle's and stay.
/// FATES_QOL_OPENING_TYPES (hex mask) replaces the list; 0 keeps every
/// opening talk (round 2's rule).
pub fn opening_types() -> u32 {
    std::env::var("FATES_QOL_OPENING_TYPES")
        .ok()
        .and_then(|v| u32::from_str_radix(v.trim_start_matches("0x"), 16).ok())
        .unwrap_or((1 << 0x07) | (1 << 0x10))
}

/// qol3: map::Situation's turn (u16, from 1) and the force whose phase it
/// is (0 the player's).
pub const SITUATION_TURN: u32 = 0x0C;
pub const SITUATION_PHASE: u32 = 0x08;

/// the running event (the event manager at 0x006D42F0, +8), its type
/// byte, and map::Situation (null off a map).
pub const EVENT_RUNNING: u32 = 0x006D_42F8;
pub const SITUATION: u32 = 0x006D_7ED0;

/// a story event left running this long without a skip of Skip Story's
/// (an event's message or question that waits for the player, which no
/// skip passes) is shown: the cover lifts.
const STORY_WAIT: u64 = 300;
/// how long the cover takes to lift, in seconds.
pub const COVER_FADE: f32 = 0.3;

/// the speed now in effect, in thousandths, for the frontend's loops.
static SPEED_MILLI: AtomicU32 = AtomicU32::new(1000);

pub fn speed_name(speed: u32) -> String {
    match speed {
        0 => "Max".to_owned(),
        n => format!("{n}\u{d7}"),
    }
}

/// the refreshes run a console refresh at a setting (0 is Max).
pub fn multiplier(speed: u32) -> u32 {
    match speed {
        0 => MAX_SPEED,
        n => n.clamp(1, MAX_SPEED),
    }
}

/// the speed in effect.
pub fn speed() -> f64 {
    SPEED_MILLI.load(Relaxed) as f64 / 1000.0
}

/// one of the game's refreshes at the speed in effect: the console's, a
/// speed-th of it.
pub fn frame_period() -> Duration {
    let milli = SPEED_MILLI.load(Relaxed).max(1000);
    if milli == 1000 {
        return super::FRAME_TIME;
    }
    Duration::from_nanos((super::FRAME_TIME.as_nanos() as u64 * 1000) / milli as u64)
}

/// the Game Speed key: 1x to the fast speed and back. Returns the new speed.
pub fn toggle_speed(pc: &mut PcSettings) -> u32 {
    let fast = if pc.game_speed_fast == 1 { 2 } else { pc.game_speed_fast };
    pc.game_speed = if pc.game_speed == 1 { fast } else { 1 };
    pc.game_speed
}

/// what the frontend keeps for the options above.
#[derive(Default)]
pub struct State {
    /// the speed last put in effect (multiplier), and whether its sound plays.
    applied: u32,
    /// Skip Story is passing a story event: the screen is covered and the
    /// game runs at Max. See Cover.
    pub skipping: bool,
    pub cover: Cover,
    /// the speed the game actually reaches (a slow PC may not reach the one
    /// asked for), measured over the last quarter seconds, for the sound's
    /// time-stretch: the frames and the time it was last measured at.
    measured: f64,
    sampled: Option<(Instant, u64)>,
    /// the boon/bane panel, read from the game each frame.
    pub chara: Option<CharaPanel>,
    /// where the top screen was last shown, in points.
    pub top_screen: Option<egui::Rect>,
    /// CharaMenuItem::Draw's count the frame before, to tell it still draws.
    chara_draws: u32,
    chara_seen: Option<Instant>,
}

// ---------------------------------------------------------------------------
// the boon/bane panel
// ---------------------------------------------------------------------------

/// the Personality table: *0x006D3B98 its records, *0x006D3B9C how many
/// (GameData::Initialize -> Personality::Initialize). A record is 0x40
/// bytes; unit::Edit::GetCapability / GetGrow / GetLimit read, for the
/// avatar's boon b and bane w, record[b] +0x0C / +0x14 / +0x1C and
/// record[w] +0x24 / +0x2C / +0x34: eight signed bytes each, HP, Str, Mag,
/// Skl, Spd, Lck, Def, Res.
pub const PERSONALITY: u32 = 0x006D_3B98;
/// the GameSkip singleton (state byte at +0, flags at +4).
pub const GAME_SKIP: u32 = 0x006D_8808;

/// what a boon and a bane change: base stats, growth rates (percent) and
/// stat caps, each for the eight stats.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub base: [i8; 8],
    pub growth: [i8; 8],
    pub cap: [i8; 8],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CharaPanel {
    /// 1 to 8, the Personality record (and stat + 1) of the boon and bane.
    pub boon: u8,
    pub bane: u8,
    /// the boon's, the bane's, and the two together.
    pub from_boon: Modifiers,
    pub from_bane: Modifiers,
    pub total: Modifiers,
    /// the row with the cursor: 3 Boon, 4 Bane, other rows other numbers.
    pub selected: u32,
    /// their names as the menu shows them, in the game's language.
    pub boon_name: String,
    pub bane_name: String,
}

/// the names in the game's English text (GameData.bin, MID_個性_* and
/// MID_短所_*), for a menu row whose text could not be read.
const BOON_NAMES: [&str; 8] = ["Robust", "Strong", "Clever", "Deft", "Quick", "Lucky", "Sturdy", "Calm"];
const BANE_NAMES: [&str; 8] = ["Sickly", "Weak", "Dull", "Clumsy", "Slow", "Unlucky", "Fragile", "Excitable"];

/// a wchar_t string of the game's, up to max characters.
fn read_text(memory: &mut zakuro_core::memory::Memory, address: u32, max: usize) -> Option<String> {
    if address == 0 {
        return None;
    }
    let mut raw = vec![0u8; max * 2];
    memory.read_bytes(address, &mut raw);
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).take_while(|&u| u != 0).collect();
    let text = String::from_utf16_lossy(&units);
    (!text.trim().is_empty()).then_some(text)
}

pub const STAT_NAMES: [&str; 8] = ["HP", "Str", "Mag", "Skl", "Spd", "Lck", "Def", "Res"];

fn read_u32(memory: &mut zakuro_core::memory::Memory, address: u32) -> u32 {
    let mut raw = [0u8; 4];
    memory.read_bytes(address, &mut raw);
    u32::from_le_bytes(raw)
}

/// a Personality record's boon or bane modifiers.
pub fn modifiers(memory: &mut zakuro_core::memory::Memory, index: u8, bane: bool) -> Option<Modifiers> {
    let table = read_u32(memory, zakuro_core::region::us(PERSONALITY));
    let count = read_u32(memory, zakuro_core::region::us(PERSONALITY) + 4);
    if table == 0 || index as u32 >= count.min(64) {
        return None;
    }
    let mut record = [0u8; 0x40];
    memory.read_bytes(table + index as u32 * 0x40, &mut record);
    let at = if bane { 0x24 } else { 0x0C };
    let take = |offset: usize| -> [i8; 8] { std::array::from_fn(|i| record[offset + i] as i8) };
    Some(Modifiers { base: take(at), growth: take(at + 8), cap: take(at + 16) })
}

impl Modifiers {
    fn plus(&self, other: &Modifiers) -> Modifiers {
        let add = |a: &[i8; 8], b: &[i8; 8]| -> [i8; 8] { std::array::from_fn(|i| a[i].saturating_add(b[i])) };
        Modifiers { base: add(&self.base, &other.base), growth: add(&self.growth, &other.growth), cap: add(&self.cap, &other.cap) }
    }
}

impl super::App {
    /// qol: after each emulated frame and before each picture: the Game
    /// Speed key, the speed in effect (Max while a skip of Skip Story's
    /// runs), its sound, and the boon/bane panel.
    pub(crate) fn qol_poll(&mut self) {
        if crate::input_api::take_action(crate::controls::PcAction::FastForward) && self.game.is_some() {
            let speed = toggle_speed(&mut self.settings.pc);
            let text = format!("Game Speed {}", speed_name(speed));
            self.menus.toast = Some((text, Instant::now()));
            self.apply_settings();
        }
        let story = skips_story(&self.settings.pc);
        let mut skipping = false;
        let mut panel = None;
        // Exit to Desktop, confirmed in My Castle's menu
        if read(Word::QuitRequest).unwrap_or(0) != 0 {
            write(Word::QuitRequest, 0);
            log::info!("qol: Exit to Desktop was chosen in the game's menu (My Castle's or the map's)"); // qol4: or the map's
            self.stop = true;
        }
        self.qol4_poll(); // qol4: the Main Menu's Options, the game's soft reset
        // qol3: Exit to Desktop chosen in the title's Main Menu: the options'
        // own question asks
        if read(Word::TitleExitRequest).unwrap_or(0) != 0 {
            write(Word::TitleExitRequest, 0);
            log::info!("qol3: Exit to Desktop was chosen in the Main Menu");
            self.fates.open_exit = true;
        }
        if let Some(game) = &mut self.game {
            if story {
                let seen = Seen::read(&mut game.system.memory);
                skipping = self.qol.cover.step(seen, game.frame);
            } else {
                self.qol.cover = Cover::default();
            }
            if self.settings.pc.boon_bane_stats {
                panel = self.qol.read_chara(&mut game.system.memory);
            }
        }
        self.qol.skipping = skipping;
        self.qol.chara = panel;
        let wanted = if skipping { MAX_SPEED } else { multiplier(self.settings.pc.game_speed) };
        // the speed reached, for the sound: game refreshes against the wall
        if let Some(game) = &self.game {
            let now = Instant::now();
            match self.qol.sampled {
                Some((at, frame)) if now - at >= Duration::from_millis(250) => {
                    let reached = game.frame.saturating_sub(frame) as f64 * super::FRAME_TIME.as_secs_f64() / (now - at).as_secs_f64();
                    let reached = reached.clamp(1.0, wanted.max(1) as f64);
                    self.qol.measured = if self.qol.measured == 0.0 { reached } else { self.qol.measured * 0.6 + reached * 0.4 };
                    self.qol.sampled = Some((now, game.frame));
                    if let (Some(audio), true) = (&self.audio, wanted > 1) {
                        audio.set_speed(self.qol.measured);
                    }
                }
                Some(_) => {}
                None => self.qol.sampled = Some((now, game.frame)),
            }
        }
        if wanted != self.qol.applied {
            let before = self.qol.applied;
            self.qol.applied = wanted;
            SPEED_MILLI.store(wanted * 1000, Relaxed);
            zakuro_core::pcos::rt::set_speed(wanted as f64);
            zakuro_core::native_gfx::set_latest_only(wanted > 1);
            if let Some(audio) = &self.audio {
                audio.set_speed(wanted as f64);
                // what was queued at the old speed goes, so the sound
                // follows the picture at once
                if before != 0 {
                    audio.clear();
                }
            }
            self.hfr.reset();
            if before != 0 {
                log::info!("qol: the game runs at {}{}", speed_name(if wanted == MAX_SPEED { 0 } else { wanted }), if skipping { " while Skip Story skips" } else { "" });
            }
        }
    }

}

/// the boon/bane panel as the game's memory has it now, with no wait for
/// the menu to be seen drawing (for pictures of it, FATES_UI_SNAPSHOT).
pub fn panel_now(memory: &mut zakuro_core::memory::Memory) -> Option<CharaPanel> {
    State::default().read_chara(memory)
}

/// what Skip Story's cover looks at in the game each frame.
#[derive(Clone, Copy, Debug, Default)]
pub struct Seen {
    /// the running event is story (a story type, or any event off a map).
    pub story: bool,
    /// an event's question is up (overrides/qol.c's dialog hooks).
    pub dialog: bool,
    /// GameSkip's state byte: a skip runs.
    pub skip: bool,
    /// the skip running is one Skip Story started.
    pub ours: bool,
}

impl Seen {
    pub fn read(memory: &mut zakuro_core::memory::Memory) -> Seen {
        let event = read_u32(memory, zakuro_core::region::us(EVENT_RUNNING));
        let story = event != 0 && {
            let mut kind = [0u8; 1];
            memory.read_bytes(event + 0x5C, &mut kind);
            let kind = kind[0] as u32;
            let situation = read_u32(memory, zakuro_core::region::us(SITUATION));
            (kind < 32 && (story_types() >> kind) & 1 != 0) || situation == 0 || {
                // qol3: the map's opening talk, as overrides/qol.c has it
                let mut place = [0u8; 6];
                memory.read_bytes(situation + SITUATION_PHASE, &mut place);
                let (phase, turn) = (place[0], u16::from_le_bytes([place[4], place[5]]));
                kind < 32 && (opening_types() >> kind) & 1 != 0 && turn == 1 && phase == 0
            }
        };
        let skip_object = read_u32(memory, zakuro_core::region::us(GAME_SKIP));
        let skip = skip_object != 0 && {
            let mut state = [0u8; 1];
            memory.read_bytes(skip_object, &mut state);
            state[0] != 0
        };
        if event == 0 && read(Word::DialogOpen).unwrap_or(0) != 0 {
            // no event, no event question: one closed some other way
            write(Word::DialogOpen, 0);
        }
        let ours = read(Word::SkipOurs).unwrap_or(0) != 0;
        if ours && !skip {
            // the skip it started is over
            write(Word::SkipOurs, 0);
        }
        Seen { story, dialog: read(Word::DialogOpen).unwrap_or(0) != 0, skip, ours: ours && skip }
    }
}

/// Skip Story's cover: from the moment a story event starts (or Skip Story
/// starts a skip) to its end, the screen is black and the game runs at
/// Max, so nothing of the story is seen, not even the game's black
/// "Skipping..." or the moment between two of an event's skippable parts.
/// It lifts for a question the event asks (the route, any choice), and if
/// the event waits STORY_WAIT refreshes with no skip (a message that waits
/// for the player). Never on a map's battle: those events are not story
/// (see story_types), and the player's own Start skips (the enemy phase)
/// are not ours.
#[derive(Default, Clone, Debug)]
pub struct Cover {
    pub covered: bool,
    /// the refresh the event last had a skip running, or started.
    last_skip: u64,
    was_story: bool,
    /// when the cover last lifted, for the fade.
    pub lifted: Option<Instant>,
    /// refreshes covered, for the log.
    pub covered_frames: u64,
}

impl Cover {
    /// one refresh: whether the screen is covered.
    pub fn step(&mut self, seen: Seen, frame: u64) -> bool {
        if seen.story && !self.was_story {
            self.last_skip = frame;
        }
        self.was_story = seen.story;
        if seen.skip {
            self.last_skip = frame;
        }
        let story = seen.story && !seen.dialog && frame.saturating_sub(self.last_skip) < STORY_WAIT;
        let covered = story || seen.ours;
        if self.covered && !covered {
            self.lifted = Some(Instant::now());
        }
        self.covered = covered;
        self.covered_frames += covered as u64;
        covered
    }

    /// how black the screen is: 1 covered, then fading over COVER_FADE.
    pub fn alpha(&self) -> f32 {
        if self.covered {
            return 1.0;
        }
        match self.lifted {
            Some(at) => (1.0 - at.elapsed().as_secs_f32() / COVER_FADE).max(0.0),
            None => 0.0,
        }
    }
}

/// headless: the cover as a window would have it, frame by frame, for
/// pictures of what the player sees (FATES_QOL_SEEN=1 writes the covered
/// frames' dumps black) and the log.
static HEADLESS_COVER: std::sync::Mutex<Option<(Cover, Option<u64>, Vec<(u64, u64)>)>> = std::sync::Mutex::new(None);

pub fn headless_frame(memory: &mut zakuro_core::memory::Memory, frame: u64, pc: &PcSettings) {
    log_events(memory, frame);
    if !skips_story(pc) || HOOKS.get().is_none() {
        return;
    }
    let seen = Seen::read(memory);
    let mut guard = HEADLESS_COVER.lock().unwrap();
    let (cover, start, spans) = guard.get_or_insert_with(|| (Cover::default(), None, Vec::new()));
    let covered = cover.step(seen, frame);
    match (covered, *start) {
        (true, None) => *start = Some(frame),
        (false, Some(from)) => {
            spans.push((from, frame));
            println!("qol: frames {from}-{frame} covered (Skip Story)");
            *start = None;
        }
        _ => {}
    }
}

/// qol3, headless: FATES_QOL_EVENTS=1 prints each event as it starts and
/// ends, with its type and the map's turn and phase, to show which talk is
/// the map's opening and which the battle's.
fn log_events(memory: &mut zakuro_core::memory::Memory, frame: u64) {
    static ON: OnceLock<bool> = OnceLock::new();
    static LAST: std::sync::Mutex<(u32, u32)> = std::sync::Mutex::new((0, 0));
    if !*ON.get_or_init(|| env_flag("FATES_QOL_EVENTS").unwrap_or(false)) {
        return;
    }
    let event = read_u32(memory, zakuro_core::region::us(EVENT_RUNNING));
    let mut kind = [0u8; 1];
    if event != 0 {
        memory.read_bytes(event + 0x5C, &mut kind);
    }
    let now = (event, kind[0] as u32);
    let mut last = LAST.lock().unwrap();
    if *last == now {
        return;
    }
    *last = now;
    let situation = read_u32(memory, zakuro_core::region::us(SITUATION));
    let (turn, phase) = if situation != 0 {
        let mut place = [0u8; 6];
        memory.read_bytes(situation + SITUATION_PHASE, &mut place);
        (u16::from_le_bytes([place[4], place[5]]) as i32, place[0] as i32)
    } else {
        (-1, -1)
    };
    let opening = situation != 0 && now.1 < 32 && (opening_types() >> now.1) & 1 != 0 && turn == 1 && phase == 0;
    match event {
        0 => println!("qol3: frame {frame} event ended (turn {turn}, phase {phase})"),
        _ => println!(
            "qol3: frame {frame} event type 0x{:02X} (turn {turn}, phase {phase}{}){}",
            now.1,
            if situation == 0 { ", no map" } else { "" },
            if opening { ": the map's opening talk" } else { "" }
        ),
    }
}

/// headless: whether the frame just run is covered.
pub fn headless_covered() -> bool {
    HEADLESS_COVER.lock().unwrap().as_ref().is_some_and(|(cover, _, _)| cover.covered)
}

impl State {
    /// whether the sound of the frame just run is played: up to MOST_HEARD
    /// it is time-stretched, above it muted.
    pub fn heard(&self) -> bool {
        self.applied <= MOST_HEARD
    }

    /// the boon/bane panel, while the avatar's menu draws (its rows' Draw
    /// counted on in the last half second).
    fn read_chara(&mut self, memory: &mut zakuro_core::memory::Memory) -> Option<CharaPanel> {
        let draws = read(Word::CharaDraws)?;
        if draws != self.chara_draws {
            self.chara_draws = draws;
            self.chara_seen = Some(Instant::now());
        }
        if self.chara_seen.is_none_or(|seen| seen.elapsed() > Duration::from_millis(150)) {
            return None;
        }
        let boon = read(Word::CharaBoon)? as u8;
        let bane = read(Word::CharaBane)? as u8;
        let recent = draws.wrapping_sub(read(Word::CharaSelectedAt).unwrap_or(0)) < 16;
        let selected = if recent { read(Word::CharaSelected).unwrap_or(0) } else { 0 };
        let from_boon = modifiers(memory, boon, false)?;
        let from_bane = modifiers(memory, bane, true)?;
        let total = from_boon.plus(&from_bane);
        let mut name = |word, names: &[&str; 8], index: u8| {
            read(word)
                .and_then(|address| read_text(memory, address, 15))
                .unwrap_or_else(|| names.get((index as usize).wrapping_sub(1)).copied().unwrap_or("").to_owned())
        };
        let boon_name = name(Word::CharaBoonText, &BOON_NAMES, boon);
        let bane_name = name(Word::CharaBaneText, &BANE_NAMES, bane);
        Some(CharaPanel { boon, bane, from_boon, from_bane, total, selected, boon_name, bane_name })
    }
}
