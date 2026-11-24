//! the PC options menu, laid out like the game's own New Game screen: the
//! ornate bar with the title across the top, the teal wallpaper, coloured
//! pennant tags for the categories, the notched parchment panel holding the
//! rows, the hand cursor on the selected one, and the help line in the bar
//! along the bottom.

use egui::Color32;

use super::kit::{Art, Canvas, TEXT_CREAM, TEXT_DARK, TEXT_GOLD, TEXT_MUTED};
use super::{Nav, Prompt, Prompts};
use crate::controls::{Family, Keys, PadButtons};
use crate::settings::{AspectMode, BottomAnchor, BottomScreen, ButtonLayout, Filter, Glyphs, Language, PcSettings, Settings, Side, WideSides, WindowMode};

/// the menu's size in units, the 3DS's pixels.
pub const WIDTH: f32 = 440.0;
pub const HEIGHT: f32 = 282.0;
const HEADER: f32 = 34.0;
const TABS_Y: f32 = 39.0;
const TAB_H: f32 = 21.0;
// mods: the panel's measures, which the Mods page lays its rows out on
pub(super) const PANEL_X: f32 = 14.0;
pub(super) const PANEL_Y: f32 = 63.0;
pub(super) const PANEL_W: f32 = WIDTH - 28.0;
pub(super) const PANEL_H: f32 = 182.0;
pub(super) const ROW: f32 = 19.0;
pub(super) const ROWS_TOP: f32 = PANEL_Y + 15.0;
const VISIBLE: usize = 8;
const FOOTER_Y: f32 = HEIGHT - HEADER;
pub(super) const PILL_W: f32 = 168.0;

/// what the menu knows about the computer, for the lists it offers.
#[derive(Default, Clone)]
pub struct Env {
    pub monitors: Vec<String>,
    /// the chosen monitor's display modes: width, height, refresh in Hz.
    pub modes: Vec<(u32, u32, u32)>,
    pub audio_devices: Vec<String>,
    /// mods: the mods the Mods page lists.
    pub mods: super::mods::ModsView,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Display,
    Screen,
    Audio,
    Controls,
    Interface,
    /// mods: the mods, on and off and in order
    Mods,
    /// qol: game speed, Skip Story, the SpotPass questions, boon/bane stats
    Gameplay,
    /// cheats: the Cheats tab (fates_ui/cheats.rs)
    Cheats,
}

pub const TABS: [Tab; 8] = [Tab::Display, Tab::Screen, Tab::Audio, Tab::Controls, Tab::Gameplay, Tab::Interface, Tab::Mods, Tab::Cheats]; // cheats: Cheats last

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::Display => "Display",
            Tab::Screen => "Bottom Screen",
            Tab::Audio => "Audio",
            Tab::Controls => "Controls",
            Tab::Interface => "Interface",
            Tab::Mods => "Mods",
            Tab::Gameplay => "Gameplay", // qol
            Tab::Cheats => "Cheats", // cheats
        }
    }

    /// the pennant colour: red, blue, purple, orange, green.
    fn colour(self) -> u8 {
        match self {
            Tab::Display => 0,
            Tab::Screen => 1,
            Tab::Audio => 2,
            Tab::Controls => 3,
            Tab::Interface => 4,
            Tab::Mods => 5,
            Tab::Gameplay => 1, // qol: blue, between orange and green
            Tab::Cheats => 2, // cheats: purple
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Main,
    Keyboard,
    Controller,
    /// cheats: a group of the Cheats tab, and a list to choose from
    Cheats(u8),
    Pick,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Item {
    WindowMode,
    Resolution,
    Monitor,
    RenderScale,
    /// gfx-hfr: pc.native_graphics
    Renderer,
    Aspect,
    /// wide-ui: pc.wide_sides
    WideSides,
    Vsync,
    FrameRate,
    FpsCap,
    /// vrrcap: pc.vrr
    Vrr,
    /// hfr60: pc.smooth_30fps
    Smooth30,
    Filter,
    Msaa,
    /// transblur: pc.transition_blur
    TransitionBlur,
    /// drawdist: pc.full_draw_distance
    FullDrawDistance,
    ShowFps,
    BottomMode,
    BottomSide,
    BottomSize,
    BottomOpacity,
    // bottom2: the floating window's place, size and Auto's delay
    BottomPosition,
    BottomScale,
    BottomOffsetX,
    BottomOffsetY,
    BottomLinger,
    // Haiku: what brings it out in Auto, and the wait for a unit's card
    BottomShowUnits,
    BottomShowForecast,
    BottomShowCombat,
    BottomShowGuides,
    BottomShowDelay,
    Volume,
    Mute,
    AudioDevice,
    MuteUnfocused,
    /// bossmusic: pc.boss_music
    BossMusic,
    ButtonLayout,
    Glyphs,
    Deadzone,
    Rumble,
    TouchSpeed,
    KeyboardPage,
    ControllerPage,
    RestoreControls,
    UiScale,
    Language,
    Key(usize),
    Pad(usize),
    RestoreKeys,
    RestorePad,
    // mods: the Mods page's rows: a mod by its place in the load order,
    // the line shown when there are none, and the two actions
    Mod(usize),
    ModsNone,
    ModsFolder,
    /// zigdl: which compiler code mods use
    ModsCompiler,
    ModsApply,
    // qol: the Gameplay tab's rows
    GameSpeed,
    FastSpeed,
    SkipStory,
    BoonBane,
    // cameras: the battle free camera and walking in My Castle's view
    FreeCamera,
    CastleWalk,
    FreeCameraStyle,
    // info-ui: the enemy targeting lines and the growth-rates button
    TargetingLines,
    GrowthButton,
    // pulse: Divine Pulse and its charges
    DivinePulse,
    PulseCharges,
    PulseFixed,
    // supports
    CorrinSAny,
    KanaGender,
    /// qol: closes Haiku, after the game's Yes/No
    ExitDesktop,
    /// cheats: a row of the Cheats tab (fates_ui/cheats.rs's ids)
    Cheat(u16),
    /// qol4: the game's soft reset, after the same Yes/No
    ReturnTitle,
    /// jpvoice: Voices, English or Japanese (only with the voice layer
    /// installed), and the restart that applies a change
    Voices,
    VoicesRestart,
    /// oneexe: Haiku Setup (Rebuild, Change dump or DLC, voices)
    HaikuSetup,
    /// versions: the version of the game that plays (with two or more
    /// installed), and the restart that applies a change
    GameVersion,
    GameVersionRestart,
    /// tones: Corrin's speaking style (only with a Japanese-script
    /// translation on)
    SpeakingStyle,
    /// updater: Check for Updates
    CheckUpdates,
    /// castlegifts: the gifts only online play and StreetPass gave
    OnlineRewards,
}

/// what the app has to do after a change.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    /// save and apply the settings.
    Apply,
    /// as Apply, and the window's mode, size or monitor.
    Display,
    /// as Apply, and reopen the sound on another device.
    Audio,
    /// wait for a key for the binding, an index into Keys::all_mut.
    RebindKey(usize),
    /// wait for a controller button, an index into PadButtons::all_mut.
    RebindPad(usize),
    /// the menu closed.
    Closed,
    /// mods: a change on the Mods page.
    Mods(super::mods::ModsEffect),
    /// qol: Exit to Desktop, confirmed.
    Quit,
    /// qol4: Return to Title, confirmed.
    ReturnTitle,
}

// pulse: Divine Pulse's mode (0 off, 1 story, 2 fixed, 3 unlimited), which
// picks the Gameplay tab's rows; set each frame from the settings
thread_local! {
    pub(super) static PULSE_ROWS: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

fn items(tab: Tab, page: Page, mode: BottomScreen, mods: usize) -> Vec<Item> {
    let pulse_mode = PULSE_ROWS.with(|rows| rows.get()); // pulse
    use Item::*;
    match page {
        Page::Keyboard => (0..Keys::default().all_mut().len()).map(Key).chain([RestoreKeys]).collect(),
        Page::Controller => (0..PadButtons::default().all_mut().len()).map(Pad).chain([RestorePad]).collect(),
        Page::Cheats(group) => super::cheats::items(group), // cheats
        Page::Pick => super::cheats::pick_items(), // cheats
        Page::Main => match tab {
            // native-final: the pure build has one renderer, the native one,
            // so there is no Renderer row to choose with
            #[cfg(feature = "emulation")]
            Tab::Display => vec![WindowMode, Resolution, Monitor, RenderScale, Renderer, Aspect, WideSides, Vsync, FrameRate, FpsCap, Vrr, Smooth30, Filter, Msaa, FullDrawDistance, TransitionBlur, ShowFps],
            #[cfg(not(feature = "emulation"))]
            Tab::Display => vec![WindowMode, Resolution, Monitor, RenderScale, Aspect, WideSides, Vsync, FrameRate, FpsCap, Vrr, Smooth30, Filter, Msaa, FullDrawDistance, TransitionBlur, ShowFps],
            // bottom2: the rows that mean something for the mode chosen
            Tab::Screen => match mode {
                BottomScreen::Auto => vec![
                    BottomMode, BottomShowUnits, BottomShowForecast, BottomShowCombat, BottomShowGuides, BottomShowDelay, BottomLinger,
                    BottomPosition, BottomScale, BottomOffsetX, BottomOffsetY, BottomOpacity,
                ],
                BottomScreen::Overlay | BottomScreen::OnDemand => vec![BottomMode, BottomPosition, BottomScale, BottomOffsetX, BottomOffsetY, BottomOpacity],
                BottomScreen::Panel => vec![BottomMode, BottomSide, BottomSize],
                BottomScreen::Classic => vec![BottomMode],
            },
            Tab::Audio => {
                let mut list = vec![Volume, Mute, AudioDevice, MuteUnfocused, BossMusic]; // bossmusic
                crate::haiku_voices::audio_rows(&mut list, Voices, VoicesRestart); // jpvoice
                list
            }
            Tab::Controls => vec![ButtonLayout, Glyphs, Deadzone, Rumble, TouchSpeed, KeyboardPage, ControllerPage, RestoreControls],
            // nolang: the USA game has English text only (m/@E), so its Language
            // row changed nothing; it is shown on the European game (five
            // languages) and the Japanese one (Haiku's menu language)
            Tab::Interface if zakuro_core::region::is_eur() || zakuro_core::region::is_jpn() => {
                let mut list = vec![UiScale, Language];
                crate::haiku_versions::interface_rows(&mut list, GameVersion, GameVersionRestart); // versions
                list.extend(crate::haiku_setup_entry::available().then_some(HaikuSetup)); // oneexe
                list.push(CheckUpdates); // updater
                list
            }
            Tab::Interface => {
                let mut list = vec![UiScale];
                crate::haiku_versions::interface_rows(&mut list, GameVersion, GameVersionRestart); // versions
                list.extend(crate::haiku_setup_entry::available().then_some(HaikuSetup)); // oneexe
                list.push(CheckUpdates); // updater
                list
            }
            // qol3: no SpotPass row (its questions are always answered No)
            Tab::Gameplay => {
                let mut list = vec![GameSpeed, FastSpeed, SkipStory, BoonBane, FreeCamera, FreeCameraStyle, CastleWalk, TargetingLines, GrowthButton, CorrinSAny, KanaGender, DivinePulse, OnlineRewards, ReturnTitle, ExitDesktop]; // castlegifts: OnlineRewards; qol; cameras: FreeCamera, FreeCameraStyle, CastleWalk; info-ui: TargetingLines, GrowthButton; supports: CorrinSAny, KanaGender; qol4: ReturnTitle; pulse: DivinePulse
                super::pulse::gameplay_rows(&mut list, pulse_mode); // pulse: its charges while it is on
                super::tones::gameplay_rows(&mut list, SpeakingStyle, KanaGender); // tones: after Kana's Gender
                list
            }
            Tab::Mods => super::mods::items(mods), // mods
            Tab::Cheats => super::cheats::groups(), // cheats
        },
    }
}

const FRAME_RATES: [u32; 7] = [30, 60, 120, 144, 165, 240, 0];
const FPS_CAPS: [u32; 7] = [0, 30, 60, 120, 144, 165, 240];
const MSAA: [u32; 4] = [1, 2, 4, 8];
/// window sizes offered besides the display's own modes, ultrawide among them.
const SIZES: [(u32, u32); 12] = [
    (1280, 720),
    (1600, 900),
    (1920, 1080),
    (2560, 1080),
    (2560, 1440),
    (3440, 1440),
    (3840, 1600),
    (3840, 2160),
    (5120, 1440),
    (5120, 2160),
    (1280, 800),
    (1920, 1200),
];

fn on_off(value: bool) -> String {
    if value { "On" } else { "Off" }.to_owned()
}

fn percent(value: f32) -> String {
    format!("{:.0}%", value * 100.0)
}

/// eur: the languages the EUR Special Edition has text for, in the order
/// its menus list them.
const EUR_LANGUAGES: [Language; 5] = [Language::English, Language::French, Language::German, Language::Spanish, Language::Italian];

/// eur: the language the EUR game shows for a system language: its own
/// five, anything else English (what its Owner::Initialize does).
fn eur_language(language: Language) -> Language {
    if EUR_LANGUAGES.contains(&language) {
        language
    } else {
        Language::English
    }
}

fn step<T: Clone + PartialEq>(list: &[T], value: T, dir: i32) -> T {
    let at = list.iter().position(|v| *v == value).unwrap_or(0) as i32;
    list[(at + dir).rem_euclid(list.len() as i32) as usize].clone()
}

fn nudge(value: f32, dir: i32, by: f32, low: f32, high: f32) -> f32 {
    (((value + by * dir as f32) / by).round() * by).clamp(low, high)
}

/// the sizes the resolution row offers: windowed, the display's modes and
/// the common sizes that fit it; exclusive, its modes alone. 0 x 0 is the
/// default, the desktop's own in exclusive full screen.
pub fn sizes(pc: &PcSettings, env: &Env) -> Vec<(u32, u32)> {
    let mut list: Vec<(u32, u32)> = env.modes.iter().map(|(w, h, _)| (*w, *h)).collect();
    let largest = list.iter().map(|(w, h)| w * h).max().unwrap_or(u32::MAX);
    let (most_w, most_h) = list.iter().fold((0, 0), |(a, b), (w, h)| (a.max(*w), b.max(*h)));
    if pc.window_mode == WindowMode::Windowed {
        list.extend(SIZES.iter().filter(|(w, h)| env.modes.is_empty() || (*w <= most_w && *h <= most_h && w * h <= largest)));
    }
    list.retain(|(w, h)| *w >= 640 && *h >= 480);
    list.sort_by_key(|(w, h)| (*w as u64 * *h as u64, *w));
    list.dedup();
    list.insert(0, (0, 0));
    list
}

fn size_name(pc: &PcSettings, (w, h): (u32, u32)) -> String {
    match (w, h, pc.window_mode) {
        (0, _, WindowMode::Exclusive) => "Desktop".to_owned(),
        (0, _, _) => "Default".to_owned(),
        _ => format!("{w} \u{d7} {h}"),
    }
}

fn mode_name(mode: WindowMode) -> &'static str {
    match mode {
        WindowMode::Windowed => "Windowed",
        WindowMode::Borderless => "Borderless",
        WindowMode::Exclusive => "Fullscreen",
    }
}

fn aspect_name(aspect: AspectMode) -> &'static str {
    match aspect {
        AspectMode::Native => "Native (5:3)",
        AspectMode::Auto => "Auto",
        AspectMode::Wide16x9 => "16:9",
        AspectMode::Wide21x9 => "21:9",
        AspectMode::Wide32x9 => "32:9",
    }
}

/// wide-ui
fn sides_name(sides: WideSides) -> &'static str {
    match sides {
        WideSides::Artwork => "Artwork",
        WideSides::Extend => "Extend",
        WideSides::Black => "Black",
    }
}

fn bottom_name(mode: BottomScreen) -> &'static str {
    crate::bottom_screen::mode_name(mode) // bottom2
}

/// bottom2: the modes in the order the menu and F9 go through them.
const BOTTOM_MODES: [BottomScreen; 5] = [BottomScreen::Auto, BottomScreen::Overlay, BottomScreen::Panel, BottomScreen::OnDemand, BottomScreen::Classic];

/// bottom2: an offset from the window's edge, in hundredths of its height.
fn offset(value: f32) -> String {
    format!("{:.0}", value * 100.0)
}

fn layout_name(layout: ButtonLayout) -> &'static str {
    match layout {
        ButtonLayout::Auto => "Auto",
        ButtonLayout::Positional => "Nintendo (A right)",
        ButtonLayout::Labels => "Xbox (A bottom)",
    }
}

fn glyphs_name(glyphs: Glyphs) -> &'static str {
    match glyphs {
        Glyphs::Auto => "Auto",
        Glyphs::Nintendo => "Nintendo",
        Glyphs::Xbox => "Xbox",
        Glyphs::PlayStation => "PlayStation",
        Glyphs::Keyboard => "Keyboard",
    }
}

/// a row: its label, its value (none for one that is pressed rather than
/// changed), and its help line.
pub fn row(item: Item, settings: &Settings, env: &Env, armed: Option<Item>, rebinding: Option<Item>) -> (String, Option<String>, String) {
    let pc = &settings.pc;
    let s = |text: &str| text.to_owned();
    match item {
        Item::WindowMode => (s("Window Mode"), Some(s(mode_name(pc.window_mode))), s("Play in a window, a borderless window covering the display, or exclusive fullscreen.")),
        Item::Resolution => {
            let value = match pc.window_mode {
                WindowMode::Borderless => "Desktop".to_owned(),
                WindowMode::Exclusive => size_name(pc, (pc.fullscreen_width, pc.fullscreen_height)),
                WindowMode::Windowed => size_name(pc, (pc.window_width, pc.window_height)),
            };
            let value = if pc.window_mode == WindowMode::Exclusive && pc.refresh_rate > 0 && pc.fullscreen_width > 0 {
                format!("{value} @ {}", pc.refresh_rate)
            } else {
                value
            };
            (s("Resolution"), Some(value), s("The window's size, or the display mode in fullscreen."))
        }
        Item::Monitor => {
            let name = env.monitors.get(pc.monitor as usize).cloned().unwrap_or_else(|| format!("Display {}", pc.monitor + 1));
            (s("Monitor"), Some(name), s("Which display to play on in fullscreen."))
        }
        Item::RenderScale => (
            s("Render Scale"),
            Some(format!("{}\u{d7} ({}\u{d7}{})", settings.resolution, 400 * settings.resolution, 240 * settings.resolution)),
            s("How many times the 3DS's resolution the game is drawn at. Applies when the game next starts."),
        ),
        // gfx-hfr
        Item::Renderer => (
            s("Renderer"),
            Some(s(if pc.native_graphics { "Native" } else { "Compatibility" })),
            s("Native draws with the PC's GPU, Compatibility as the 3DS did. Applies when the game next starts."),
        ),
        Item::Aspect => (s("Aspect Ratio"), Some(s(aspect_name(pc.aspect))), s("The shape of the top screen. Wider shows more of the battlefield, and the menus keep their shape. Auto fills the window.")),
        // wide-ui
        Item::WideSides => (
            s("Ultrawide Sides"),
            Some(s(sides_name(pc.wide_sides))),
            s("Beside menus, talks and other 5:3 screens: the game's own frame art, the picture carried on, or black."),
        ),
        Item::Vsync => (s("VSync"), Some(on_off(pc.vsync)), s("Wait for the display between frames, so the picture never tears.")),
        Item::FrameRate => {
            let value = if pc.frame_rate == 0 { s("Display") } else { format!("{} FPS", pc.frame_rate) };
            // smooth2: Display with VSync off and no cap is uncapped
            (s("Frame Rate"), Some(value), s("How many frames a second are shown. Display matches its refresh rate, or with VSync off and no cap, as many as the PC can draw."))
        }
        Item::FpsCap => {
            let value = if pc.fps_cap == 0 { s("Off") } else { format!("{} FPS", pc.fps_cap) };
            (s("FPS Cap"), Some(value), s("The most frames a second shown, whatever else is set."))
        }
        // vrrcap
        Item::Vrr => (
            s("Variable Refresh"),
            Some(on_off(pc.vrr)),
            s("For G-SYNC and FreeSync displays: the frame rate stays just under the refresh rate (162 FPS at 170 Hz), so motion stays even. Off for fixed refresh."),
        ),
        // hfr60
        Item::Smooth30 => (
            s("Smooth 30 FPS Scenes"),
            Some(on_off(pc.smooth_30fps)),
            s("At 60 FPS, a frame is drawn between each two of the battles', 3D cutscenes' and title screen's, which the game runs at 30. Above 60 FPS every scene is smoothed."),
        ),
        Item::Filter => (s("Filtering"), Some(s(settings.filter.name())), s("How the picture is scaled to the window: smooth, sharp, or pixels.")),
        Item::Msaa => {
            let value = if pc.msaa <= 1 { s("Off") } else { format!("{}\u{d7}", pc.msaa) };
            // perf3: what it costs
            (s("Anti-Aliasing"), Some(value), s("Multisampling, which smooths the edges of the 3D. Each step up costs GPU time, most at a high render scale, where 4\u{d7} looks almost the same as 8\u{d7}. If the GPU falls behind, pictures in between take fewer samples first."))
        }
        // drawdist
        Item::FullDrawDistance => (
            s("Full Draw Distance"),
            Some(on_off(pc.full_draw_distance)),
            s("Grass and scenery are drawn however far away they are, at full detail, so nothing pops in as the camera nears it. Off, the 3DS's own distances."),
        ),
        // transblur
        Item::TransitionBlur => (
            s("Battle Transition Blur"),
            Some(on_off(pc.transition_blur)),
            s("The zoom blur as the map gives way to a fight and back. Off, the zoom is drawn at the display's frame rate."),
        ),
        Item::ShowFps => (s("Show FPS"), Some(on_off(settings.show_fps)), s("Show the frame rate in the corner of the screen.")),
        Item::BottomMode => {
            let help = match pc.bottom_screen {
                BottomScreen::Auto => "The touch screen slides in when the game shows something on it, and away after. M or L3 shows or hides it.",
                BottomScreen::Overlay => "The touch screen always shows over the game. Drag its frame to move it, or a corner to size it.",
                BottomScreen::Panel => "The touch screen sits beside the game, in the room a wide display leaves.",
                BottomScreen::OnDemand => "The touch screen slides in only while M or L3 is held, or after a tap of it.",
                BottomScreen::Classic => "Both screens as on the 3DS. F9 changes their arrangement.",
            };
            (s("Display Mode"), Some(s(bottom_name(pc.bottom_screen))), s(help))
        }
        Item::BottomSide => (s("Side"), Some(s(if pc.bottom_side == Side::Left { "Left" } else { "Right" })), s("Which side of the game the touch screen goes on.")),
        Item::BottomSize => (s("Size"), Some(percent(pc.bottom_size)), s("The touch screen's height next to the game's.")),
        Item::BottomOpacity => (s("Opacity"), Some(percent(pc.bottom_opacity)), s("How solid the touch screen is when drawn over the game.")),
        Item::BottomPosition => (s("Position"), Some(s(pc.bottom_anchor.name())), s("The corner or edge of the window the touch screen keeps to.")),
        Item::BottomScale => (s("Size"), Some(percent(pc.bottom_height)), s("The touch screen's height, as a share of the window's.")),
        Item::BottomOffsetX => (s("Across"), Some(offset(pc.bottom_x)), s("How far the touch screen sits in from the side it keeps to.")),
        Item::BottomOffsetY => (s("Up and Down"), Some(offset(pc.bottom_y)), s("How far the touch screen sits in from the top or bottom it keeps to.")),
        Item::BottomLinger => (s("Hide After"), Some(format!("{:.1} s", pc.bottom_linger)), s("How long the touch screen stays once the game stops showing anything on it.")),
        Item::BottomShowUnits => (s("Unit Info"), Some(on_off(pc.bottom_show_units)), s("Show a unit's details when the cursor rests on it, and in the unit list.")),
        Item::BottomShowForecast => (s("Attack Forecast"), Some(on_off(pc.bottom_show_forecast)), s("Show the forecast while choosing whom to attack, heal or trade with.")),
        Item::BottomShowCombat => (s("Combat"), Some(on_off(pc.bottom_show_combat)), s("Show the touch screen through a battle, up to the EXP bar and level-up.")),
        Item::BottomShowGuides => (s("Tutorials and Setup"), Some(on_off(pc.bottom_show_guides)), s("Show tutorial pages and choices made on the touch screen, like your avatar's build.")),
        Item::BottomShowDelay => (s("Show After"), Some(format!("{:.1} s", pc.bottom_show_delay)), s("How long the cursor rests on a unit before its details come out.")),
        Item::Volume => (s("Master Volume"), Some(format!("{:.0}", settings.volume * 100.0)), s("The volume of everything the game plays.")),
        Item::Mute => (s("Mute"), Some(on_off(settings.mute)), s("Silence the game.")),
        Item::AudioDevice => {
            let value = if pc.audio_device.is_empty() { s("System Default") } else { pc.audio_device.clone() };
            (s("Output Device"), Some(value), s("The speakers or headphones the sound plays on."))
        }
        Item::MuteUnfocused => (s("Mute in Background"), Some(on_off(pc.mute_unfocused)), s("Silence the game while another window is in front.")),
        // bossmusic
        Item::BossMusic => (
            s("Boss Music Stays"),
            Some(on_off(pc.boss_music)),
            s("Once you fight a boss, its battle theme keeps playing on the map until the boss is defeated, as in later Fire Emblem games."),
        ),
        Item::ButtonLayout => (
            s("Button Layout"),
            Some(s(layout_name(pc.button_layout))),
            s("Which face button confirms. Auto follows the controller connected."),
        ),
        Item::Glyphs => (s("Button Icons"), Some(s(glyphs_name(pc.glyphs))), s("Which button pictures the menus show.")),
        Item::Deadzone => (s("Stick Dead Zone"), Some(percent(pc.stick_deadzone)), s("How far a stick has to move before it counts.")),
        Item::Rumble => (s("Rumble"), Some(on_off(pc.rumble)), s("Let the controller vibrate.")),
        Item::TouchSpeed => (s("Touch Cursor Speed"), Some(format!("{:.0}", pc.touch_cursor_speed)), s("How fast the right stick moves the touch cursor.")),
        // cameras
        Item::FreeCamera => (
            s("Battle Free Camera"),
            Some(on_off(pc.battle_free_camera)),
            format!(
                "Move the camera freely in battles and My Castle. {} switches it, {} gives the game's camera back.",
                camera_key(settings, "Free camera"),
                camera_key(settings, "Camera reset")
            ),
        ),
        Item::FreeCameraStyle => (
            s("Free Camera Style"),
            Some(s(if pc.free_camera_orbit { "Orbit" } else { "Fly" })),
            s(if pc.free_camera_orbit {
                "Turn around the fight with the right stick or a drag; the wheel zooms."
            } else {
                "Left stick or WASD moves, L/R or Q/E rise and sink, right stick or a drag looks, the wheel goes ahead."
            }),
        ),
        Item::CastleWalk => (
            s("Walk in Castle View"),
            Some(on_off(pc.castle_walk)),
            s("In My Castle's close-up view (L), walk with the left stick, turn the camera with the right stick or a drag, zoom with the wheel."),
        ),
        Item::KeyboardPage => (s("Keyboard Controls"), None, s("Choose the keys for each of the 3DS's buttons.")),
        Item::ControllerPage => (s("Controller Controls"), None, s("Choose the controller buttons for each of the 3DS's buttons.")),
        Item::RestoreControls | Item::RestoreKeys | Item::RestorePad => {
            let value = (armed == Some(item)).then(|| s("Press again"));
            let help = match item {
                Item::RestoreKeys => "Put every key back as it was.",
                Item::RestorePad => "Put every controller button back as it was.",
                _ => "Put the keys, buttons and controller options back as they were.",
            };
            (s("Restore Defaults"), value, s(help))
        }
        Item::UiScale => (s("Menu Size"), Some(percent(pc.ui_scale)), s("The size of these menus.")),
        // eur: the EUR Special Edition shows its text in the language the
        // system reports: English, French, German, Spanish or Italian
        // jpprep: the Japanese game is Japanese only; the row says which
        // language Haiku's own menus are in
        Item::Language if zakuro_core::region::is_jpn() => (
            s("Menu Language"),
            Some(s(if pc.japanese_english_menus { "English" } else { "日本語" })),
            s("The language of Haiku's own menus. The game's text is Japanese (or a translation mod's)."),
        ),
        Item::Language if zakuro_core::region::is_eur() => (
            s("Game Language"),
            Some(s(eur_language(settings.language).name())),
            s("The language of the game's text: English, French, German, Spanish or Italian. It changes the next time the game starts."),
        ),
        Item::Language => (
            s("Language"),
            Some(s(settings.language.name())),
            s("The game's language. It changes the next time the game starts."),
        ),
        Item::Key(index) => {
            let mut keys = settings.keys.clone();
            let (name, key) = {
                let all = keys.all_mut();
                (all[index].0.to_owned(), all[index].1.label())
            };
            let value = if rebinding == Some(item) { s("Press a key") } else { key };
            // qol4: the options menu's own key
            if name == crate::controls::OPTIONS_ROW {
                return (name, Some(value), s("The key that opens and closes these options. Press, then press the key; it leaves any other binding."));
            }
            (name, Some(value), s("Press, then press the key to use. Esc cancels."))
        }
        Item::Pad(index) => {
            let mut pad = settings.pad.clone();
            let (name, button) = {
                let all = pad.all_mut();
                // named as on the controller connected, Xbox's without one
                let family = crate::input_api::connected_pads().first().map_or(Family::Xbox, |pad| pad.family);
                (all[index].0.to_owned(), all[index].1.label_on(family, family.positional(settings.pc.button_layout)))
            };
            let value = if rebinding == Some(item) { s("Press a button") } else { button };
            // qol4: the options menu's own button
            if name == crate::controls::OPTIONS_ROW {
                return (name, Some(value), s("The button that opens and closes these options (Start and Select together do too). It leaves any other binding. Esc cancels."));
            }
            (name, Some(value), s("Press, then press the controller button to use. Esc cancels."))
        }
        Item::Mod(_) | Item::ModsNone | Item::ModsFolder | Item::ModsCompiler | Item::ModsApply => super::mods::row(item, env), // mods
        Item::Cheat(id) => super::cheats::row(id, settings), // cheats
        // qol
        Item::GameSpeed => (
            s("Game Speed"),
            Some(crate::qol::speed_name(pc.game_speed)),
            game_speed_key(settings) + " switches between 1\u{d7} and Fast Speed. Above 4\u{d7} the sound is muted.",
        ),
        Item::FastSpeed => (
            s("Fast Speed"),
            Some(crate::qol::speed_name(pc.game_speed_fast)),
            game_speed_key(settings) + " switches to this speed from 1\u{d7}.",
        ),
        Item::SkipStory => (
            s("Skip Story"),
            Some(on_off(pc.skip_story)),
            // qol3: the talk before each battle too; the battle's own stays
            s("Skip cutscenes, movies and the talk before battles. Talk during battles, boss quotes and choices still show."),
        ),
        // info-ui
        // qol4: on or off; on is every enemy's lines, following a moved unit
        Item::TargetingLines => (
            s("Enemy Targeting Lines"),
            Some(on_off(pc.targeting_lines == crate::settings::TargetingLines::On)),
            s("On the battle map, show a line from each enemy to the unit it will attack next enemy phase, as the enemy's own AI decides. The lines follow a unit while you move it."),
        ),
        Item::DivinePulse | Item::PulseCharges | Item::PulseFixed => super::pulse::row(item, settings), // pulse
        Item::GrowthButton => (
            s("Growth Rates Button"),
            Some(s(if pc.growth_button { "Show" } else { "Hide" })),
            s("A button on the lower screen's unit status that shows the unit's growth rates. G or the bound controller button does the same."),
        ),
        // supports
        Item::CorrinSAny => (
            s("Corrin's S Supports"),
            Some(s(if pc.corrin_s_any { "Anyone" } else { "Game's Own" })),
            s(if pc.corrin_s_any {
                "Corrin can reach S rank and marry anyone Corrin has a support with, of either gender. Takes effect the next time the game starts."
            } else {
                "Corrin's S ranks are the game's own. Takes effect the next time the game starts."
            }),
        ),
        Item::KanaGender => (
            s("Kana's Gender"),
            Some(s(match pc.kana_gender {
                crate::settings::KanaGender::Ask => "Ask",
                crate::settings::KanaGender::Vanilla => "Game's Own",
                crate::settings::KanaGender::Male => "Male",
                crate::settings::KanaGender::Female => "Female",
                crate::settings::KanaGender::Random => "Random",
            })),
            s(match pc.kana_gender {
                crate::settings::KanaGender::Ask => "When Kana's paralogue starts, choose Kana's gender: male, female or random.",
                crate::settings::KanaGender::Vanilla => "As in the game: a male Corrin has a daughter, a female Corrin a son.",
                crate::settings::KanaGender::Random => "Kana is a son or a daughter at random, decided when Kana's paralogue starts.",
                _ => "Kana's gender, whatever Corrin's.",
            }),
        ),
        // jpvoice
        Item::Voices => (
            s("Voices"),
            Some(s(if pc.voices == crate::settings::Voices::Japanese { "Japanese" } else { "English" })),
            s(crate::haiku_voices::help(pc.voices)),
        ),
        Item::VoicesRestart => (s("Restart with These Voices"), None, s("Restart the game so the voices change. Anything you haven't saved will be lost.")),
        Item::SpeakingStyle => super::tones::row(), // tones
        // castlegifts
        Item::OnlineRewards => (
            s("Online Rewards"),
            Some(s(if pc.online_rewards { "Unlocked" } else { "Game's Own" })),
            s(if pc.online_rewards {
                "Claim each Battle and Visit Bonus once in My Castle's Inbox > Rewards, and get the StreetPass presents from Extras > Presents on the title screen."
            } else {
                "The Battle and Visit Bonuses and the StreetPass presents need online play and StreetPass, so they stay locked."
            }),
        ),
        Item::ExitDesktop => (s("Exit to Desktop"), None, s("Close the game and return to the desktop. Anything you haven't saved will be lost.")),
        // oneexe
        Item::HaikuSetup => (s("Haiku Setup"), None, s("Add or remove a version of the game, change your dump or DLC, add the Japanese voices, or build the game's code again. The game closes first; Play in Haiku Setup starts it again.")),
        // versions
        Item::GameVersion => {
            let tag = crate::haiku_versions::shown(&pc.game_version);
            (s("Game Version"), Some(s(crate::haiku_versions::label(&tag))), crate::haiku_versions::help(&tag))
        }
        Item::GameVersionRestart => (s("Restart with This Version"), None, s("Restart the game with the version as set. Anything you haven't saved will be lost.")),
        // updater
        Item::CheckUpdates => (
            s("Check for Updates"),
            Some(s(if pc.check_updates { "On" } else { "Off" })),
            s(if pc.check_updates {
                "Once a day, ask GitHub (api.github.com) for a newer Haiku and say so on the Main Menu. Only Haiku's version is sent."
            } else {
                "Haiku never checks for updates or connects to GitHub. New versions are at github.com/tyle-beep/Haiku."
            }),
        ),
        // qol4
        Item::ReturnTitle => (s("Return to Title"), None, s("Restart the game from its opening, as the game's own soft reset does. Anything you haven't saved will be lost.")),
        Item::BoonBane => (
            s("Boon/Bane Stats"),
            Some(s(if pc.boon_bane_stats { "Show" } else { "Hide" })),
            s("When choosing your avatar's boon and bane, show what they do to each stat."),
        ),
    }
}

/// changes item by dir (-1 or 1, 0 to press it).
pub fn adjust(item: Item, dir: i32, settings: &mut Settings, env: &Env) -> Option<Effect> {
    let pc = &mut settings.pc;
    let d = if dir == 0 { 1 } else { dir };
    let effect = match item {
        Item::WindowMode => {
            pc.window_mode = step(&[WindowMode::Windowed, WindowMode::Borderless, WindowMode::Exclusive], pc.window_mode, d);
            Effect::Display
        }
        Item::Resolution => {
            let list = sizes(pc, env);
            match pc.window_mode {
                WindowMode::Borderless => return None,
                WindowMode::Exclusive => {
                    let (w, h) = step(&list, (pc.fullscreen_width, pc.fullscreen_height), d);
                    pc.fullscreen_width = w;
                    pc.fullscreen_height = h;
                    pc.refresh_rate = env.modes.iter().filter(|m| (m.0, m.1) == (w, h)).map(|m| m.2).max().unwrap_or(0);
                }
                WindowMode::Windowed => {
                    let (w, h) = step(&list, (pc.window_width, pc.window_height), d);
                    pc.window_width = w;
                    pc.window_height = h;
                }
            }
            Effect::Display
        }
        Item::Monitor => {
            let count = env.monitors.len().max(1) as i32;
            pc.monitor = (pc.monitor as i32 + d).rem_euclid(count) as u32;
            Effect::Display
        }
        Item::RenderScale => {
            settings.resolution = (settings.resolution as i32 + d).clamp(1, 8) as u32;
            Effect::Apply
        }
        // gfx-hfr: saved, read when the game next starts
        Item::Renderer => {
            pc.native_graphics = !pc.native_graphics;
            Effect::Apply
        }
        Item::Aspect => {
            pc.aspect = step(&[AspectMode::Native, AspectMode::Auto, AspectMode::Wide16x9, AspectMode::Wide21x9, AspectMode::Wide32x9], pc.aspect, d);
            Effect::Apply
        }
        // wide-ui: applied from the next frame
        Item::WideSides => {
            pc.wide_sides = step(&[WideSides::Artwork, WideSides::Extend, WideSides::Black], pc.wide_sides, d);
            Effect::Apply
        }
        Item::Vsync => {
            pc.vsync = !pc.vsync;
            Effect::Apply
        }
        Item::FrameRate => {
            pc.frame_rate = step(&FRAME_RATES, pc.frame_rate, d);
            Effect::Apply
        }
        Item::FpsCap => {
            pc.fps_cap = step(&FPS_CAPS, pc.fps_cap, d);
            Effect::Apply
        }
        // vrrcap: from the next picture
        Item::Vrr => {
            pc.vrr = !pc.vrr;
            Effect::Apply
        }
        // hfr60: from the next refresh
        Item::Smooth30 => {
            pc.smooth_30fps = !pc.smooth_30fps;
            Effect::Apply
        }
        Item::Filter => {
            settings.filter = step(&Filter::ALL, settings.filter, d);
            Effect::Apply
        }
        Item::Msaa => {
            pc.msaa = step(&MSAA, pc.msaa.max(1), d);
            Effect::Apply
        }
        // drawdist: from the next frame
        Item::FullDrawDistance => {
            pc.full_draw_distance = !pc.full_draw_distance;
            Effect::Apply
        }
        // transblur: from the next transition
        Item::TransitionBlur => {
            pc.transition_blur = !pc.transition_blur;
            Effect::Apply
        }
        Item::ShowFps => {
            settings.show_fps = !settings.show_fps;
            Effect::Apply
        }
        Item::BottomMode => {
            pc.bottom_screen = step(&BOTTOM_MODES, pc.bottom_screen, d);
            Effect::Apply
        }
        // bottom2: a new corner or edge puts it neatly there
        Item::BottomPosition => {
            pc.bottom_anchor = step(&BottomAnchor::ALL, pc.bottom_anchor, d);
            let centred_x = matches!(pc.bottom_anchor, BottomAnchor::Top | BottomAnchor::Bottom);
            let centred_y = matches!(pc.bottom_anchor, BottomAnchor::Left | BottomAnchor::Right);
            pc.bottom_x = if centred_x { 0.0 } else { 0.02 };
            pc.bottom_y = if centred_y { 0.0 } else { 0.02 };
            Effect::Apply
        }
        Item::BottomScale => {
            pc.bottom_height = nudge(pc.bottom_height, d, 0.01, 0.15, 0.9);
            Effect::Apply
        }
        Item::BottomOffsetX => {
            pc.bottom_x = nudge(pc.bottom_x, d, 0.01, -1.0, 1.5);
            Effect::Apply
        }
        Item::BottomOffsetY => {
            pc.bottom_y = nudge(pc.bottom_y, d, 0.01, -1.0, 1.0);
            Effect::Apply
        }
        Item::BottomShowUnits => {
            pc.bottom_show_units = !pc.bottom_show_units;
            Effect::Apply
        }
        Item::BottomShowForecast => {
            pc.bottom_show_forecast = !pc.bottom_show_forecast;
            Effect::Apply
        }
        Item::BottomShowCombat => {
            pc.bottom_show_combat = !pc.bottom_show_combat;
            Effect::Apply
        }
        Item::BottomShowGuides => {
            pc.bottom_show_guides = !pc.bottom_show_guides;
            Effect::Apply
        }
        Item::BottomShowDelay => {
            pc.bottom_show_delay = nudge(pc.bottom_show_delay, d, 0.1, 0.0, 2.0);
            Effect::Apply
        }
        Item::BottomLinger => {
            pc.bottom_linger = nudge(pc.bottom_linger, d, 0.25, 0.0, 5.0);
            Effect::Apply
        }
        Item::BottomSide => {
            pc.bottom_side = if pc.bottom_side == Side::Left { Side::Right } else { Side::Left };
            Effect::Apply
        }
        Item::BottomSize => {
            pc.bottom_size = nudge(pc.bottom_size, d, 0.05, 0.3, 1.0);
            Effect::Apply
        }
        Item::BottomOpacity => {
            pc.bottom_opacity = nudge(pc.bottom_opacity, d, 0.05, 0.0, 1.0);
            Effect::Apply
        }
        Item::Volume => {
            settings.volume = nudge(settings.volume, d, 0.05, 0.0, 1.0);
            Effect::Apply
        }
        Item::Mute => {
            settings.mute = !settings.mute;
            Effect::Apply
        }
        Item::AudioDevice => {
            let mut list = vec![String::new()];
            list.extend(env.audio_devices.iter().cloned());
            pc.audio_device = step(&list, pc.audio_device.clone(), d);
            if !list.contains(&pc.audio_device) {
                pc.audio_device.clear();
            }
            Effect::Audio
        }
        Item::MuteUnfocused => {
            pc.mute_unfocused = !pc.mute_unfocused;
            Effect::Apply
        }
        // bossmusic: from the next tick on a map
        Item::BossMusic => {
            pc.boss_music = !pc.boss_music;
            Effect::Apply
        }
        Item::ButtonLayout => {
            pc.button_layout = step(&[ButtonLayout::Auto, ButtonLayout::Positional, ButtonLayout::Labels], pc.button_layout, d);
            Effect::Apply
        }
        Item::Glyphs => {
            pc.glyphs = step(&[Glyphs::Auto, Glyphs::Nintendo, Glyphs::Xbox, Glyphs::PlayStation, Glyphs::Keyboard], pc.glyphs, d);
            Effect::Apply
        }
        Item::Deadzone => {
            pc.stick_deadzone = nudge(pc.stick_deadzone, d, 0.01, 0.0, 0.5);
            Effect::Apply
        }
        Item::Rumble => {
            pc.rumble = !pc.rumble;
            Effect::Apply
        }
        Item::TouchSpeed => {
            pc.touch_cursor_speed = nudge(pc.touch_cursor_speed, d, 50.0, 100.0, 1600.0);
            Effect::Apply
        }
        // cameras
        Item::FreeCamera => {
            pc.battle_free_camera = !pc.battle_free_camera;
            Effect::Apply
        }
        Item::CastleWalk => {
            pc.castle_walk = !pc.castle_walk;
            Effect::Apply
        }
        Item::FreeCameraStyle => {
            pc.free_camera_orbit = !pc.free_camera_orbit;
            Effect::Apply
        }
        Item::UiScale => {
            pc.ui_scale = nudge(pc.ui_scale, d, 0.05, 0.75, 2.0);
            Effect::Apply
        }
        Item::Language if zakuro_core::region::is_jpn() => {
            pc.japanese_english_menus = !pc.japanese_english_menus; // jpprep
            Effect::Apply
        }
        Item::Language if zakuro_core::region::is_eur() => {
            settings.language = step(&EUR_LANGUAGES, eur_language(settings.language), d); // eur
            Effect::Apply
        }
        Item::Language => {
            settings.language = step(&Language::ALL, settings.language, d);
            Effect::Apply
        }
        Item::RestoreControls => {
            let defaults = PcSettings::default();
            settings.keys = Keys::default();
            settings.pad = PadButtons::default();
            let pc = &mut settings.pc;
            pc.button_layout = defaults.button_layout;
            pc.glyphs = defaults.glyphs;
            pc.stick_deadzone = defaults.stick_deadzone;
            pc.rumble = defaults.rumble;
            pc.touch_cursor_speed = defaults.touch_cursor_speed;
            Effect::Apply
        }
        Item::RestoreKeys => {
            settings.keys = Keys::default();
            Effect::Apply
        }
        Item::RestorePad => {
            settings.pad = PadButtons::default();
            Effect::Apply
        }
        Item::Key(index) => return (dir == 0).then_some(Effect::RebindKey(index)),
        Item::Pad(index) => return (dir == 0).then_some(Effect::RebindPad(index)),
        Item::KeyboardPage | Item::ControllerPage => return None,
        // mods: the Mods page's
        Item::Mod(_) | Item::ModsNone | Item::ModsFolder | Item::ModsCompiler | Item::ModsApply => return super::mods::adjust(item, dir, env),
        Item::Cheat(id) => return super::cheats::adjust(id, dir, settings), // cheats
        // qol: a speed chosen other than 1x is also what the key switches to
        Item::GameSpeed => {
            pc.game_speed = step(&crate::qol::SPEEDS, pc.game_speed, d);
            if pc.game_speed != 1 {
                pc.game_speed_fast = pc.game_speed;
            }
            Effect::Apply
        }
        Item::FastSpeed => {
            pc.game_speed_fast = step(&crate::qol::SPEEDS[1..], pc.game_speed_fast, d);
            if pc.game_speed != 1 {
                pc.game_speed = pc.game_speed_fast;
            }
            Effect::Apply
        }
        Item::SkipStory => {
            pc.skip_story = !pc.skip_story;
            Effect::Apply
        }
        Item::BoonBane => {
            pc.boon_bane_stats = !pc.boon_bane_stats;
            Effect::Apply
        }
        // updater: on starts a check (none ran yet), off hides the notice
        Item::CheckUpdates => {
            pc.check_updates = !pc.check_updates;
            crate::haiku_update::setting_changed(pc.check_updates);
            Effect::Apply
        }
        // info-ui
        Item::TargetingLines => {
            use crate::settings::TargetingLines as T;
            pc.targeting_lines = step(&[T::Off, T::On], pc.targeting_lines, d); // qol4: on or off
            Effect::Apply
        }
        Item::GrowthButton => {
            pc.growth_button = !pc.growth_button;
            Effect::Apply
        }
        // pulse
        Item::DivinePulse | Item::PulseCharges | Item::PulseFixed => return super::pulse::adjust(item, d, pc),
        // supports
        Item::CorrinSAny => {
            pc.corrin_s_any = !pc.corrin_s_any;
            Effect::Apply
        }
        Item::KanaGender => {
            use crate::settings::KanaGender as K;
            pc.kana_gender = step(&[K::Ask, K::Vanilla, K::Male, K::Female, K::Random], pc.kana_gender, d);
            Effect::Apply
        }
        // jpvoice: either way, the other language
        Item::Voices => {
            use crate::settings::Voices as V;
            pc.voices = if pc.voices == V::Japanese { V::English } else { V::Japanese };
            crate::haiku_voices::chosen(pc.voices);
            Effect::Apply
        }
        Item::SpeakingStyle => return super::tones::adjust(d), // tones
        Item::OnlineRewards => {
            pc.online_rewards = !pc.online_rewards; // castlegifts
            Effect::Apply
        }
        // versions: the next version installed, either way
        Item::GameVersion => {
            pc.game_version = crate::haiku_versions::step(&pc.game_version, d);
            crate::haiku_versions::chosen(&pc.game_version);
            Effect::Apply
        }
        Item::ExitDesktop | Item::ReturnTitle | Item::VoicesRestart | Item::HaikuSetup | Item::GameVersionRestart => return None, // qol4: ReturnTitle; jpvoice: VoicesRestart; oneexe: HaikuSetup; versions
    };
    Some(effect)
}

/// cameras: the key and button a camera action is on, for the help line.
fn camera_key(settings: &Settings, name: &str) -> String {
    let mut keys = settings.keys.clone();
    let mut pad = settings.pad.clone();
    let key = keys.all_mut().into_iter().find(|(n, _)| *n == name).map(|(_, key)| key.label()).unwrap_or_default();
    let family = crate::input_api::connected_pads().first().map(|pad| pad.family);
    let button = match family {
        Some(family) => pad.all_mut().into_iter().find(|(n, _)| *n == name).map(|(_, b)| b.label_on(family, family.positional(settings.pc.button_layout))),
        None => None,
    };
    match (key.trim(), button.as_deref().map(str::trim)) {
        ("" | "-", None | Some("" | "-")) => format!("The {name} binding"),
        (key, None | Some("" | "-")) => key.to_owned(),
        ("" | "-", Some(button)) => button.to_owned(),
        (key, Some(button)) => format!("{key} / {button}"),
    }
}

/// qol5: the toast when the Free Camera key is pressed in My Castle: the
/// castle's free camera came out (Some(true)) or went back (Some(false)),
/// or it can't come out here (None: a menu, a scene, the close-up view).
pub fn castle_free_camera_toast(settings: &Settings, change: Option<bool>) -> String {
    match change {
        Some(true) => format!("Free Camera: {} to go back", camera_key(settings, "Free camera")),
        Some(false) => "Free Camera Off".to_owned(),
        None => "The free camera works while you walk around the castle".to_owned(),
    }
}

/// qol: the key bound to Game Speed, for the help line.
fn game_speed_key(settings: &Settings) -> String {
    let mut keys = settings.keys.clone();
    let key = keys.all_mut().into_iter().find(|(name, _)| *name == "Game Speed").map(|(_, key)| key.label()).unwrap_or_default();
    let key = key.replace("Backquote", "`");
    match key.trim() {
        "" | "None" | "-" | "\u{2014}" => "The Game Speed key".to_owned(),
        key => format!("The {key} key"),
    }
}

/// whether a row is a toggle or list, changed with left and right, rather
/// than pressed.
fn changes(item: Item) -> bool {
    if matches!(item, Item::DivinePulse | Item::PulseCharges | Item::PulseFixed) {
        return true; // pulse
    }
    if let Item::Cheat(id) = item {
        return super::cheats::changes(id); // cheats
    }
    !matches!(
        item,
        Item::KeyboardPage | Item::ControllerPage | Item::RestoreControls | Item::RestoreKeys | Item::RestorePad | Item::Key(_) | Item::Pad(_)
            | Item::ExitDesktop // qol
            | Item::ReturnTitle // qol4
            | Item::VoicesRestart // jpvoice
            | Item::HaikuSetup // oneexe
            | Item::GameVersionRestart // versions
            | Item::ModsNone | Item::ModsFolder | Item::ModsApply // mods (zigdl: ModsCompiler changes with left and right)
    )
}

pub struct OptionsMenu {
    pub open: bool,
    tab: usize,
    page: Page,
    selected: usize,
    /// the first row shown.
    top: usize,
    /// when it opened, for the fade.
    opened: f64,
    /// a restore row pressed once, waiting for a second press.
    armed: Option<Item>,
    /// a binding row waiting for its key or button.
    pub rebinding: Option<Item>,
    /// where the pointer was last frame, so a still pointer does not take
    /// the selection from the keys.
    last_pointer: Option<egui::Pos2>,
    /// the pointer is over the menu, for the app to keep clicks from the game.
    pub hovered: bool,
    /// bottom2: the bottom screen's mode, which picks the tab's rows.
    bottom_mode: BottomScreen,
    /// mods: how many mods the Mods page lists.
    mod_count: usize,
    /// mods: opened from the game's Main Menu: the Mods page alone.
    pub mods_only: bool,
    /// mods: the dialog asking to restart, while it is up.
    confirm: Option<super::mods::Confirm>,
    /// qol3: opened from the title's Main Menu: the Exit to Desktop
    /// question alone, over the game; No closes it.
    pub exit_only: bool,
}

impl Default for OptionsMenu {
    fn default() -> Self {
        OptionsMenu {
            open: false,
            tab: 0,
            page: Page::Main,
            selected: 0,
            top: 0,
            opened: 0.0,
            armed: None,
            rebinding: None,
            last_pointer: None,
            hovered: false,
            bottom_mode: BottomScreen::default(),
            mod_count: 0,
            mods_only: false,
            confirm: None,
            exit_only: false, // qol3
        }
    }
}

/// where things are, in units.
struct Hit {
    tabs: Vec<(f32, f32)>,
    /// qol: the tags' text size, smaller when seven have to fit
    scale: f32,
}

impl OptionsMenu {
    pub fn show(&mut self, now: f64) {
        if !self.open {
            self.open = true;
            self.opened = now;
            self.page = Page::Main;
            self.armed = None;
            self.rebinding = None;
            self.last_pointer = None;
            self.mods_only = false;
            self.confirm = None;
            self.exit_only = false; // qol3
        }
    }

    /// qol3: the Exit to Desktop question alone, as the title's Main Menu
    /// asks: the options' own question, No chosen to start with.
    pub fn show_exit(&mut self, now: f64) {
        self.show(now);
        self.exit_only = true;
        self.confirm = Some(super::mods::Confirm { yes: false, download: None, exit: true, title: false });
    }

    /// mods: opens on the Mods page alone, as the game's Main Menu asks.
    pub fn show_mods(&mut self, now: f64) {
        self.show(now);
        self.mods_only = true;
        self.tab = TABS.iter().position(|tab| *tab == Tab::Mods).unwrap_or(0);
        self.page = Page::Main;
        self.selected = 0;
        self.top = 0;
    }

    /// mods: puts up the restart dialog, for the screenshots.
    pub fn ask_restart(&mut self) {
        self.confirm = Some(super::mods::Confirm { yes: true, download: None, exit: false, title: false });
    }

    /// codemods: puts up the compiler download question, for the screenshots.
    pub fn ask_download(&mut self, ask: super::codemods::Ask) {
        self.confirm = Some(super::mods::Confirm { yes: true, download: Some(ask), exit: false, title: false });

    }

    /// qol4: the Return to Title question, for pictures of it.
    #[allow(dead_code)]
    pub fn ask_title(&mut self) {
        self.confirm = Some(super::mods::Confirm { yes: false, download: None, exit: false, title: true });
    }

    /// qol: the Exit to Desktop question, for pictures of it.
    #[allow(dead_code)]
    pub fn ask_exit(&mut self) {
        self.confirm = Some(super::mods::Confirm { yes: false, download: None, exit: true, title: false });
    }

    /// oneexe: Haiku Setup's Yes/No, for the interface's pictures
    pub fn ask_setup(&mut self) {
        crate::haiku_setup_entry::set_asking(true);
        self.confirm = Some(super::mods::Confirm { yes: false, download: None, exit: false, title: false });
    }

    /// mods: the rows the page shows at once.
    fn visible(&self) -> usize {
        if self.page == Page::Cheats(super::states::GROUP) {
            super::states::VISIBLE // pulse: the slot's card under the rows
        } else if self.tab() == Tab::Mods && self.page == Page::Main {
            super::mods::VISIBLE
        } else {
            VISIBLE
        }
    }

    /// mods: moves the selected mod up or down the load order, the
    /// selection with it.
    fn move_mod(&mut self, index: usize, by: i32, effects: &mut Vec<Effect>) {
        let to = index as i32 + by;
        if to < 0 || to as usize >= self.mod_count {
            return;
        }
        effects.push(Effect::Mods(super::mods::ModsEffect::Move(index, by)));
        self.selected = to as usize;
        self.keep_visible();
    }

    pub fn hide(&mut self) {
        self.open = false;
        self.confirm = None; // mods
        self.rebinding = None;
        self.armed = None;
        self.hovered = false;
    }

    /// for the screenshots: which tab, page and row.
    /// cheats2: the Cheats tab is the one showing (its unit view is asked for only then)
    pub fn on_cheats(&self) -> bool {
        self.open && TABS[self.tab] == Tab::Cheats
    }

    pub fn select(&mut self, tab: usize, page: Page, row: usize) {
        self.tab = tab;
        self.page = page;
        self.selected = row;
        self.top = row.saturating_sub(self.visible() - 1);
    }

    pub fn fade(&self, now: f64) -> f32 {
        ((now - self.opened) / 0.12).clamp(0.0, 1.0) as f32
    }

    fn tab(&self) -> Tab {
        TABS[self.tab]
    }

    fn items(&self) -> Vec<Item> {
        items(self.tab(), self.page, self.bottom_mode, self.mod_count)
    }

    /// bottom2: the Bottom Screen tab is open, on a row that places or
    /// shows the floating window, for the app to show it while it changes.
    pub fn previewing_bottom(&self) -> bool {
        self.open && self.page == Page::Main && self.tab() == Tab::Screen && self.items().get(self.selected).is_some_and(|item| *item != Item::BottomLinger)
    }

    fn title(&self) -> &'static str {
        if self.mods_only {
            return "Mods"; // mods
        }
        match self.page {
            Page::Main => "Options",
            Page::Keyboard => "Keyboard Controls",
            Page::Controller => "Controller Controls",
            Page::Cheats(_) | Page::Pick => super::cheats::title(self.page), // cheats
        }
    }

    fn keep_visible(&mut self) {
        let count = self.items().len();
        self.selected = self.selected.min(count.saturating_sub(1));
        if self.selected < self.top {
            self.top = self.selected;
        }
        let visible = self.visible();
        if self.selected >= self.top + visible {
            self.top = self.selected + 1 - visible;
        }
        self.top = self.top.min(count.saturating_sub(visible));
    }

    fn switch_tab(&mut self, by: i32) {
        if self.mods_only {
            return; // mods: the Mods page alone
        }
        self.tab = (self.tab as i32 + by).rem_euclid(TABS.len() as i32) as usize;
        self.page = Page::Main;
        self.selected = 0;
        self.top = 0;
        self.armed = None;
    }

    /// what a press of a row does inside the menu, before the settings.
    fn press(&mut self, item: Item, dir: i32, settings: &mut Settings, env: &Env, effects: &mut Vec<Effect>) {
        // pulse: a save state's slot opens its dialog
        if let (Item::Cheat(id), 0) = (item, dir) {
            if super::states::is_ours(id) && !super::states::changes(id) {
                super::states::press(id, settings);
                return;
            }
        }
        // cheats: A on a Cheats row opens a page or a list, or acts
        if let (Item::Cheat(id), 0) = (item, dir) {
            match super::cheats::press(id, self.page, self.selected) {
                super::cheats::Press::Open(page) => {
                    self.page = page;
                    self.selected = 0;
                    self.top = 0;
                }
                super::cheats::Press::Back(page, row) => {
                    self.page = page;
                    self.selected = row;
                    self.top = 0;
                    self.keep_visible();
                }
                super::cheats::Press::Nothing => {}
            }
            return;
        }
        match item {
            Item::KeyboardPage | Item::ControllerPage if dir == 0 => {
                self.page = if item == Item::KeyboardPage { Page::Keyboard } else { Page::Controller };
                self.selected = 0;
                self.top = 0;
                return;
            }
            // mods: a restart is asked for first
            Item::ModsApply if dir == 0 => {
                self.confirm = Some(super::mods::Confirm { yes: true, download: None, exit: false, title: false });
                return;
            }
            // oneexe: Haiku Setup, after the game's Yes/No (No first)
            Item::HaikuSetup if dir == 0 => {
                crate::haiku_setup_entry::set_asking(true);
                self.confirm = Some(super::mods::Confirm { yes: false, download: None, exit: false, title: false });
                return;
            }
            // qol: the game's Yes/No first, No chosen to start with
            Item::ExitDesktop if dir == 0 => {
                self.confirm = Some(super::mods::Confirm { yes: false, download: None, exit: true, title: false });
                return;
            }
            // versions: the restart that plays another version, after the Yes/No
            Item::GameVersionRestart if dir == 0 => {
                crate::haiku_versions::set_asking(true);
                self.confirm = Some(super::mods::Confirm { yes: true, download: None, exit: false, title: false });
                return;
            }
            // jpvoice: the restart that applies Voices, after the Yes/No
            Item::VoicesRestart if dir == 0 => {
                crate::haiku_voices::set_asking(true);
                self.confirm = Some(super::mods::Confirm { yes: true, download: None, exit: false, title: false });
                return;
            }
            // qol4: the same for Return to Title
            Item::ReturnTitle if dir == 0 => {
                self.confirm = Some(super::mods::Confirm { yes: false, download: None, exit: false, title: true });
                return;
            }
            Item::RestoreControls | Item::RestoreKeys | Item::RestorePad if dir == 0 => {
                if self.armed != Some(item) {
                    self.armed = Some(item);
                    return;
                }
                self.armed = None;
            }
            _ => {}
        }
        if dir != 0 && !changes(item) {
            return;
        }
        if let Some(effect) = adjust(item, dir, settings, env) {
            self.bottom_mode = settings.pc.bottom_screen; // bottom2
            if matches!(effect, Effect::RebindKey(_) | Effect::RebindPad(_)) {
                self.rebinding = Some(item);
            }
            effects.push(effect);
        }
    }

    fn back(&mut self, effects: &mut Vec<Effect>) {
        // cheats: a list goes back to its page, a page to the tab
        if let Page::Cheats(group) = self.page {
            self.page = Page::Main;
            self.selected = group as usize;
            self.top = 0;
            self.keep_visible();
            return;
        }
        if self.page == Page::Pick {
            let (group, row) = super::cheats::with_ui(|ui| ui.from);
            self.page = Page::Cheats(group);
            self.selected = row;
            self.top = 0;
            self.keep_visible();
            return;
        }
        if self.page != Page::Main {
            let came_from = if self.page == Page::Keyboard { Item::KeyboardPage } else { Item::ControllerPage };
            self.page = Page::Main;
            self.selected = self.items().iter().position(|i| *i == came_from).unwrap_or(0);
            self.top = 0;
            self.keep_visible();
        } else {
            self.hide();
            effects.push(Effect::Closed);
        }
    }

    /// moves through the menu with the keys and buttons of a frame and the
    /// pointer, then draws it. returns what the app has to do.
    pub fn frame(&mut self, canvas: &Canvas, input: &egui::InputState, nav: &Nav, prompts: &Prompts, settings: &mut Settings, env: &Env) -> Vec<Effect> {
        let mut effects = Vec::new();
        let now = input.time;
        self.bottom_mode = settings.pc.bottom_screen; // bottom2
        PULSE_ROWS.with(|rows| rows.set(super::pulse::mode(&settings.pc))); // pulse
        self.mod_count = env.mods.rows.len(); // mods
        // hdpacks: a download being unpacked shows over the Mods page; B
        // closes the menu, the unpacking goes on
        let unpacking = (self.open && self.tab() == Tab::Mods).then(crate::haiku_import::running).flatten();
        // codemods: the compiler download question, when the app asks it
        if let Some(ask) = env.mods.ask_download {
            if self.confirm.as_ref().is_none_or(|confirm| confirm.download.is_none()) {
                self.confirm = Some(super::mods::Confirm { yes: true, download: Some(ask), exit: false, title: false });
            }
        }
        super::mods::set_compiler_row(env.mods.compiler_row.then_some(settings.pc.code_mods_compiler)); // zigdl
        // codemods: the compile's dialog takes the input while it is up
        if unpacking.is_some() && env.mods.job.is_none() {
            if nav.back || nav.close || input.pointer.secondary_clicked() {
                self.hide();
                effects.push(Effect::Closed);
            }
        } else if env.mods.job.is_some() {
            if nav.back || nav.close {
                effects.push(Effect::Mods(super::mods::ModsEffect::CancelJob));
            }
        } else if let Some(confirm) = &mut self.confirm {
            // mods: the restart dialog takes the input while it is up
            let download = confirm.download.is_some();
            let choice = confirm.download.is_some_and(|ask| ask.setup); // zigdl
            match super::mods::confirm_nav(confirm, canvas, input, nav, env.mods.pending) {
                Some(super::mods::Answer::Yes) if confirm.exit => {
                    self.confirm = None;
                    effects.push(Effect::Quit); // qol
                }
                // oneexe: Haiku Setup starts, and the game closes
                Some(super::mods::Answer::Yes) if crate::haiku_setup_entry::asking() => {
                    self.confirm = None;
                    if crate::haiku_setup_entry::open() {
                        effects.push(Effect::Quit);
                    }
                }
                // qol4: Return to Title: the menu closes, the game resets
                Some(super::mods::Answer::Yes) if confirm.title => {
                    self.confirm = None;
                    self.hide();
                    effects.push(Effect::ReturnTitle);
                    effects.push(Effect::Closed);
                }
                Some(super::mods::Answer::Yes) => {
                    self.confirm = None;
                    effects.push(Effect::Mods(if download { super::mods::ModsEffect::Download(true) } else { super::mods::ModsEffect::Restart }));
                }
                // zigdl: on the compiler choice, No is the other compiler
                Some(super::mods::Answer::No) if choice => {
                    self.confirm = None;
                    effects.push(Effect::Mods(super::mods::ModsEffect::UseSetupCompiler));
                }
                Some(super::mods::Answer::No) | Some(super::mods::Answer::Cancel) => {
                    self.confirm = None;
                    if download {
                        effects.push(Effect::Mods(super::mods::ModsEffect::Download(false)));
                    }
                }
                None => {}
            }
            if self.confirm.is_none() {
                crate::haiku_voices::set_asking(false); // jpvoice
                crate::haiku_setup_entry::set_asking(false); // oneexe
                crate::haiku_versions::set_asking(false); // versions
            }
            // qol3: the title's question alone: answered, the menu closes
            if self.exit_only && self.confirm.is_none() {
                self.hide();
                effects.push(Effect::Closed);
            }
        } else if super::cheats::confirming() {
            super::cheats::confirm_nav(canvas, input, nav); // cheats: a cheat's Yes/No
        } else if super::states::confirming() {
            super::states::confirm_nav(canvas, input, nav); // pulse: a save state's dialog
        } else if self.rebinding.is_none() {
            self.navigate(canvas, input, nav, settings, env, &mut effects);
        }
        if self.open {
            // qol3: the title's Exit to Desktop question is shown alone
            if !self.exit_only {
                self.draw(canvas, now, prompts, settings, env);
                super::cheats::draw_confirm(canvas, now); // cheats
                super::states::draw_confirm(canvas, now); // pulse
            }
            if let Some(job) = &env.mods.job {
                super::codemods::draw_job(canvas, job, now); // codemods
            } else if let Some(confirm) = &self.confirm {
                super::mods::draw_confirm(confirm, canvas, now, env.mods.pending);
            }
            if let Some(progress) = &unpacking {
                super::mods::draw_unpacking(progress, canvas, now); // hdpacks
            }
        }
        effects
    }

    fn navigate(&mut self, canvas: &Canvas, input: &egui::InputState, nav: &Nav, settings: &mut Settings, env: &Env, effects: &mut Vec<Effect>) {
        let items = self.items();
        let count = items.len();
        if nav.close {
            self.hide();
            effects.push(Effect::Closed);
            return;
        }
        // mods: X and Y on a controller, Shift with Up and Down on the
        // keyboard, move the selected mod through the load order
        if let Some(Item::Mod(index)) = items.get(self.selected).copied() {
            let shift = input.modifiers.shift;
            let by = if nav.extra || (shift && nav.up) {
                -1
            } else if nav.extra2 || (shift && nav.down) {
                1
            } else {
                0
            };
            if by != 0 {
                self.move_mod(index, by, effects);
                return;
            }
        }
        let visible = self.visible();
        if nav.up {
            self.selected = if self.selected == 0 { count - 1 } else { self.selected - 1 };
            self.armed = None;
        }
        if nav.down {
            self.selected = (self.selected + 1) % count;
            self.armed = None;
        }
        if nav.page_up {
            self.selected = self.selected.saturating_sub(visible);
        }
        if nav.page_down {
            self.selected = (self.selected + visible).min(count - 1);
        }
        if nav.prev_tab {
            self.switch_tab(-1);
        }
        if nav.next_tab {
            self.switch_tab(1);
        }
        self.keep_visible();
        let item = self.items()[self.selected];
        if nav.left {
            self.press(item, -1, settings, env, effects);
        }
        if nav.right {
            self.press(item, 1, settings, env, effects);
        }
        if nav.confirm {
            if changes(item) {
                self.press(item, 1, settings, env, effects);
            } else {
                self.press(item, 0, settings, env, effects);
            }
        }
        if nav.back {
            self.back(effects);
            return;
        }

        // the pointer
        let hit = self.hit(canvas);
        let pointer = input.pointer.hover_pos();
        let moved = pointer.is_some() && pointer != self.last_pointer;
        self.last_pointer = pointer;
        self.hovered = false;
        let Some(pos) = pointer else { return };
        let (x, y) = canvas.unit_of(pos);
        let inside = (0.0..WIDTH).contains(&x) && (0.0..HEIGHT).contains(&y);
        self.hovered = inside;
        let clicked = input.pointer.primary_clicked();
        let wheel: f32 = input
            .events
            .iter()
            .filter_map(|event| match event {
                egui::Event::MouseWheel { delta, unit, .. } => Some(match unit {
                    egui::MouseWheelUnit::Point => delta.y / 40.0,
                    egui::MouseWheelUnit::Line => delta.y,
                    egui::MouseWheelUnit::Page => delta.y * visible as f32,
                }),
                _ => None,
            })
            .sum();
        if input.pointer.secondary_clicked() {
            self.back(effects);
            return;
        }
        if inside && wheel.abs() >= 0.5 {
            let rows = wheel.round() as i32;
            let most = count.saturating_sub(visible) as i32;
            self.top = (self.top as i32 - rows).clamp(0, most) as usize;
            self.selected = self.selected.clamp(self.top, self.top + visible - 1).min(count - 1);
        }
        // the tags
        if (TABS_Y..TABS_Y + TAB_H).contains(&y) {
            for (index, (tx, tw)) in hit.tabs.iter().enumerate() {
                if (*tx..tx + tw).contains(&x) && clicked && index != self.tab && !self.mods_only {
                    self.tab = index;
                    self.page = Page::Main;
                    self.selected = 0;
                    self.top = 0;
                }
            }
        }
        // the header's prompt closes or goes back
        if clicked && y < HEADER && x > WIDTH - 120.0 {
            self.back(effects);
            return;
        }
        // the scroll arrows
        let arrow_x = PANEL_X + PANEL_W - 30.0;
        // mods: the scroll arrows sit above the details box on the Mods page
        let arrows_bottom = if visible == VISIBLE { PANEL_Y + PANEL_H } else { ROWS_TOP + visible as f32 * ROW + 12.0 };
        if clicked && (arrow_x..arrow_x + 24.0).contains(&x) {
            if (PANEL_Y - 6.0..PANEL_Y + 12.0).contains(&y) && self.top > 0 {
                self.top -= 1;
                self.selected = self.selected.min(self.top + visible - 1);
                return;
            }
            if (arrows_bottom - 12.0..arrows_bottom + 6.0).contains(&y) && self.top + visible < count {
                self.top += 1;
                self.selected = self.selected.max(self.top);
                return;
            }
        }
        // the rows
        if (PANEL_X..PANEL_X + PANEL_W).contains(&x) && y >= ROWS_TOP {
            let index = self.top + ((y - ROWS_TOP) / ROW) as usize;
            if index < count && index < self.top + visible {
                // mods: the selected mod's move arrows
                if let (true, Item::Mod(at)) = (clicked && index == self.selected, items[index]) {
                    let row_y = ROWS_TOP + (index - self.top) as f32 * ROW;
                    for (ax, _, by) in super::mods::move_arrows(row_y) {
                        if (ax - 1.0..ax + 13.0).contains(&x) {
                            self.move_mod(at, by, effects);
                            return;
                        }
                    }
                }
                if moved && index != self.selected {
                    self.selected = index;
                    self.armed = None;
                }
                if clicked {
                    self.selected = index;
                    let item = self.items()[index];
                    let pill_x = PANEL_X + PANEL_W - 24.0 - PILL_W;
                    let dir = if changes(item) && (pill_x - 4.0..pill_x + 18.0).contains(&x) {
                        -1
                    } else if changes(item) {
                        1
                    } else {
                        0
                    };
                    self.press(item, dir, settings, env, effects);
                }
            }
        }
    }

    /// the tags' places across the top.
    fn hit(&self, canvas: &Canvas) -> Hit {
        // qol: as much room around each name as lets them all fit between
        // the L and R prompts (18 units for six, less for seven), and a
        // smaller text when even 8 units each would not fit
        let mut scale = 0.82;
        let shown = self.shown_tabs();
        let gap = if shown.len() > 6 { 3.0 } else { 4.0 };
        let fixed = 2.0 * 26.0 + gap * shown.len().saturating_sub(1) as f32;
        let at_full: f32 = shown.iter().map(|(_, tab)| canvas.kit.measure(tab.name(), scale)).sum();
        let most = WIDTH - fixed - 8.0 * shown.len() as f32;
        if at_full > most {
            scale *= most / at_full;
        }
        let names: Vec<f32> = shown.iter().map(|(_, tab)| canvas.kit.measure(tab.name(), scale)).collect();
        let room = WIDTH - fixed - names.iter().sum::<f32>();
        let pad = (room / names.len().max(1) as f32).clamp(8.0, 18.0);
        let widths: Vec<f32> = names.iter().map(|w| w + pad).collect();
        let total: f32 = widths.iter().sum::<f32>() + gap * (widths.len() - 1) as f32;
        let mut x = (WIDTH - total) / 2.0;
        let tabs = widths
            .iter()
            .map(|w| {
                let at = x;
                x += w + gap;
                (at, *w)
            })
            .collect();
        Hit { tabs, scale }
    }

    /// mods: the tags shown, with their index in TABS: all of them, or the
    /// Mods page's alone when the Main Menu opened it.
    fn shown_tabs(&self) -> Vec<(usize, Tab)> {
        TABS.iter().copied().enumerate().filter(|(_, tab)| !self.mods_only || *tab == Tab::Mods).collect()
    }

    /// the tags' places in units, for tests that click them.
    #[allow(dead_code)]
    pub fn tab_places(&self, canvas: &Canvas) -> Vec<(f32, f32)> {
        self.hit(canvas).tabs
    }

    fn draw(&mut self, canvas: &Canvas, now: f64, prompts: &Prompts, settings: &Settings, env: &Env) {
        let white = Color32::WHITE;
        // the wallpaper behind it all, and the bars
        canvas.image(Art::Wallpaper, 0.0, HEADER - 2.0, WIDTH, FOOTER_Y - HEADER + 4.0, Color32::from_rgb(210, 220, 215));
        canvas.image(Art::Header, 0.0, 0.0, WIDTH, HEADER, Color32::from_rgb(200, 120, 100));
        canvas.image(Art::Header, 0.0, FOOTER_Y, WIDTH, HEADER, white);
        canvas.text(10.0, 9.0, self.title(), 1.0, TEXT_GOLD);

        // the prompts in the header's corner, in the game's dark pill
        let back = if self.page == Page::Main { "Close" } else { "Back" };
        let items = self.items();
        let selected = items[self.selected.min(items.len() - 1)];
        let entries = match self.tab() {
            Tab::Mods => super::mods::prompts(selected, self.mods_only, prompts.set), // mods
            _ => vec![(Prompt::Confirm, "Select"), (Prompt::Back, back)],
        };
        let width = super::prompts_width(canvas, prompts, &entries);
        let px = WIDTH - 8.0 - width - 10.0;
        canvas.nine(Art::PillPrompt, px, 8.0, width + 10.0, 18.0, [7.0, 7.0, 7.0, 7.0], 1.0, white);
        super::draw_prompts(canvas, prompts, px + 5.0, 9.0, &entries);

        // the category tags, with L and R either side
        let hit = self.hit(canvas);
        for ((index, tab), (x, w)) in self.shown_tabs().iter().zip(&hit.tabs) {
            let chosen = *index == self.tab;
            let tint = if chosen { white } else { Color32::from_rgb(120, 112, 104) };
            canvas.nine(Art::Tag(tab.colour()), *x, TABS_Y, *w, TAB_H, [7.0, 6.0, 12.0, 7.0], 0.85, tint);
            let colour = if chosen { TEXT_CREAM } else { Color32::from_rgb(200, 186, 160) };
            canvas.text_centered(x + w / 2.0 - 2.0, TABS_Y + 3.0 + (0.82 - hit.scale) * 7.0, tab.name(), hit.scale, colour); // qol: hit.scale
        }
        if let (Some(first), Some(last), false) = (hit.tabs.first(), hit.tabs.last(), self.mods_only) {
            super::draw_prompt(canvas, prompts, first.0 - 22.0, TABS_Y + 3.0, Prompt::PrevTab);
            super::draw_prompt(canvas, prompts, last.0 + last.1 + 5.0, TABS_Y + 3.0, Prompt::NextTab);
        }

        // the panel and its rows
        canvas.nine(Art::Panel, PANEL_X, PANEL_Y, PANEL_W, PANEL_H, [60.0, 40.0, 60.0, 40.0], 1.0, white);
        let bob = ((now * 6.0).sin() * 1.5) as f32;
        let pill_x = PANEL_X + PANEL_W - 24.0 - PILL_W;
        let visible = self.visible(); // mods
        for (slot, index) in (self.top..items.len().min(self.top + visible)).enumerate() {
            let item = items[index];
            let y = ROWS_TOP + slot as f32 * ROW;
            let chosen = index == self.selected;
            let (label, value, _) = row(item, settings, env, self.armed, self.rebinding);
            if chosen {
                canvas.three(Art::Highlight, PANEL_X + 22.0, y - 1.0, PANEL_W - 44.0, ROW + 1.0, [10.0, 10.0], white);
            } else if slot + 1 < visible && index + 1 < items.len() {
                // the faint rule between rows, as under the game's entries
                canvas.fill(PANEL_X + 26.0, y + ROW - 0.5, PANEL_W - 52.0, 0.6, Color32::from_rgba_unmultiplied(70, 110, 110, 70));
            }
            let label_colour = if chosen { TEXT_CREAM } else { TEXT_DARK };
            let label = super::mods::fit_label(canvas, item, label, pill_x - if chosen { 36.0 } else { 8.0 } - (PANEL_X + 32.0)); // mods
            let label_width = canvas.text(PANEL_X + 32.0, y + 2.0, &label, 0.95, label_colour);
            super::mods::draw_row_extras(canvas, item, env, PANEL_X + 32.0, y, label_width, chosen, now); // mods
            super::states::draw_row_extras(canvas, item, PANEL_X + 32.0, y, label_width, chosen); // statecheck: a slot made with another layer
            match value {
                Some(value) => {
                    let pill = if chosen { Art::PillRed } else { Art::PillCream };
                    canvas.nine(pill, pill_x, y + 2.0, PILL_W, ROW - 4.0, [9.0, 9.0, 9.0, 9.0], 0.62, white);
                    let colour = if chosen { TEXT_CREAM } else { TEXT_DARK };
                    let blink = self.rebinding == Some(item) && (now * 2.0).fract() > 0.5;
                    if !blink {
                        let room = PILL_W - 30.0;
                        let mut scale = 0.9;
                        while canvas.kit.measure(&value, scale) > room && scale > 0.7 {
                            scale -= 0.05;
                        }
                        let mut value = value;
                        if canvas.kit.measure(&value, scale) > room {
                            while canvas.kit.measure(&format!("{value}..."), scale) > room && value.pop().is_some() {}
                            value = format!("{}...", value.trim_end());
                        }
                        canvas.text_centered(pill_x + PILL_W / 2.0, y + 2.5 + (0.9 - scale) * 8.0, &value, scale, colour);
                    }
                    if changes(item) {
                        canvas.image(Art::ArrowLeft, pill_x + 4.0, y + 4.0, 6.3, 10.0, white);
                        canvas.image(Art::ArrowRight, pill_x + PILL_W - 10.3, y + 4.0, 6.3, 10.0, white);
                    }
                }
                None if matches!(item, Item::KeyboardPage | Item::ControllerPage) => {
                    canvas.image(Art::ArrowRight, PANEL_X + PANEL_W - 38.0, y + 3.0, 6.3, 10.0, white);
                }
                None => {}
            }
            if chosen {
                canvas.sprite(Art::Hand, PANEL_X + 4.0 + bob, y + 2.5, white);
            }
        }
        // the scroll arrows, bobbing as the game's do
        let float = ((now * 4.0).sin() * 1.5) as f32;
        let arrow_x = PANEL_X + PANEL_W - 30.0;
        // mods: on the Mods page they sit above the details box
        let arrows_bottom = if visible == VISIBLE { PANEL_Y + PANEL_H } else { ROWS_TOP + visible as f32 * ROW + 12.0 };
        if self.top > 0 {
            canvas.image(Art::ScrollUp, arrow_x, PANEL_Y + 1.0 - float, 22.0, 13.0, white);
        }
        if self.top + visible < items.len() {
            canvas.image(Art::ScrollDown, arrow_x, arrows_bottom - 15.0 + float, 22.0, 13.0, white);
        }
        // pulse: the chosen save state's card
        if self.page == Page::Cheats(super::states::GROUP) {
            super::states::draw_card(canvas, selected);
        }
        // mods: the selected mod's details, and the keyboard's move keys
        if self.tab() == Tab::Mods && self.page == Page::Main {
            super::mods::draw_details(canvas, selected, env);
            if prompts.set == crate::controls::GlyphSet::Keyboard && matches!(selected, Item::Mod(_)) {
                super::mods::draw_keyboard_hint(canvas, PANEL_Y + 2.0);
            }
        }

        // the help line
        let item = items[self.selected.min(items.len() - 1)];
        let (_, _, help) = row(item, settings, env, self.armed, self.rebinding);
        let mut help_scale = 0.9;
        let mut lines = canvas.kit.wrap_even(&help, WIDTH - 40.0, help_scale);
        if lines.len() > 1 {
            help_scale = 0.8;
            lines = canvas.kit.wrap_even(&help, WIDTH - 40.0, help_scale);
        }
        let line_h = 13.0;
        let top = FOOTER_Y + (HEADER - lines.len() as f32 * line_h) / 2.0 - 1.0;
        for (i, line) in lines.iter().enumerate() {
            canvas.text_centered(WIDTH / 2.0, top + i as f32 * line_h, line, help_scale, TEXT_CREAM);
        }
        let _ = TEXT_MUTED;
    }
}
