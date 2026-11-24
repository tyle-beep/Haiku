//! what the user chose, kept between runs in the system's place for
//! settings.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use zakuro_gpu::{ScreenLayout, ScreenOrientation};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Renderer {
    #[default]
    Vulkan,
    OpenGl,
}

/// how the screens are arranged in the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Screens {
    #[default]
    Stacked,
    SideBySide,
    TopOnly,
    BottomOnly,
    LargeTop,
}

impl Screens {
    pub const ALL: [Screens; 5] = [Screens::Stacked, Screens::SideBySide, Screens::LargeTop, Screens::TopOnly, Screens::BottomOnly];

    pub fn name(self) -> &'static str {
        match self {
            Screens::Stacked => "Top over bottom",
            Screens::SideBySide => "Screens side by side",
            Screens::TopOnly => "Top screen only",
            Screens::BottomOnly => "Bottom screen only",
            Screens::LargeTop => "Big top screen, small bottom one",
        }
    }

    pub fn screens(self) -> ScreenLayout {
        match self {
            Screens::Stacked => ScreenLayout::Stacked,
            Screens::SideBySide => ScreenLayout::SideBySide,
            Screens::TopOnly => ScreenLayout::TopOnly,
            Screens::BottomOnly => ScreenLayout::BottomOnly,
            Screens::LargeTop => ScreenLayout::LargeTop,
        }
    }

    /// the one after it, going round.
    pub fn next(self) -> Screens {
        let at = Screens::ALL.iter().position(|&layout| layout == self).unwrap_or(0);
        Screens::ALL[(at + 1) % Screens::ALL.len()]
    }

    /// with one screen showing, the other one alone, else the same.
    pub fn other_screen(self) -> Screens {
        match self {
            Screens::TopOnly => Screens::BottomOnly,
            Screens::BottomOnly => Screens::TopOnly,
            both => both,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Orientation {
    #[default]
    North,
    East,
    South,
    West,
}

impl Orientation {
    pub const ALL: [Orientation; 4] = [Orientation::North, Orientation::East, Orientation::South, Orientation::West];

    pub fn name(self) -> &'static str {
        match self {
            Orientation::North => "0 degree",
            Orientation::East => "90 degree",
            Orientation::South => "180 degree",
            Orientation::West => "270 degree",
        }
    }

    pub fn screen_orientation(self) -> ScreenOrientation {
        match self {
            Orientation::North => ScreenOrientation::North,
            Orientation::East => ScreenOrientation::East,
            Orientation::South => ScreenOrientation::South,
            Orientation::West => ScreenOrientation::West,
        }
    }
}

/// how the screens' pixels look scaled up to the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Filter {
    #[default]
    Smooth,
    Sharp,
    Pixels,
}

impl Filter {
    pub const ALL: [Filter; 3] = [Filter::Smooth, Filter::Sharp, Filter::Pixels];

    pub fn name(self) -> &'static str {
        match self {
            Filter::Smooth => "Smooth",
            Filter::Sharp => "Sharp",
            Filter::Pixels => "Pixels",
        }
    }

    pub fn screens(self) -> zakuro_gpu::ScreenFilter {
        match self {
            Filter::Smooth => zakuro_gpu::ScreenFilter::Smooth,
            Filter::Sharp => zakuro_gpu::ScreenFilter::Sharp,
            Filter::Pixels => zakuro_gpu::ScreenFilter::Pixels,
        }
    }
}

/// the top screen's shape. a game draws into 400x240 whatever this is, so
/// anything but the console's own only looks right with a widescreen code
/// on, which makes the game draw a wider scene into those pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TopShape {
    #[default]
    Console,
    Wide16x9,
    Wide21x9,
}

impl TopShape {
    pub const ALL: [TopShape; 3] = [TopShape::Console, TopShape::Wide16x9, TopShape::Wide21x9];

    pub fn name(self) -> &'static str {
        match self {
            TopShape::Console => "The console's, 5:3",
            TopShape::Wide16x9 => "16:9",
            TopShape::Wide21x9 => "21:9",
        }
    }

    /// the top screen's width at its height of 240.
    pub fn width(self) -> f32 {
        match self {
            TopShape::Console => 400.0,
            TopShape::Wide16x9 => 240.0 * 16.0 / 9.0,
            TopShape::Wide21x9 => 560.0,
        }
    }
}

/// how the top screen's 3D shows, for games that draw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stereo3d {
    /// games draw flat, as with the console's 3D slider down.
    #[default]
    Off,
    Anaglyph,
    SideBySide,
    HalfSideBySide,
    Interlaced,
}

impl Stereo3d {
    pub const ALL: [Stereo3d; 5] = [Stereo3d::Off, Stereo3d::Anaglyph, Stereo3d::SideBySide, Stereo3d::HalfSideBySide, Stereo3d::Interlaced];

    pub fn name(self) -> &'static str {
        match self {
            Stereo3d::Off => "Off",
            Stereo3d::Anaglyph => "Red and cyan glasses",
            Stereo3d::SideBySide => "Side by side",
            Stereo3d::HalfSideBySide => "Side by side, half width",
            Stereo3d::Interlaced => "Alternate rows",
        }
    }

    pub fn mode(self) -> zakuro_gpu::StereoMode {
        match self {
            Stereo3d::Off => zakuro_gpu::StereoMode::Off,
            Stereo3d::Anaglyph => zakuro_gpu::StereoMode::Anaglyph,
            Stereo3d::SideBySide => zakuro_gpu::StereoMode::SideBySide,
            Stereo3d::HalfSideBySide => zakuro_gpu::StereoMode::HalfSideBySide,
            Stereo3d::Interlaced => zakuro_gpu::StereoMode::Interlaced,
        }
    }
}

/// the console's system language, which a title reads from cfg:u and picks
/// its localised text and files by, the codes the console numbers them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    Japanese,
    #[default]
    English,
    French,
    German,
    Italian,
    Spanish,
    ChineseSimplified,
    Korean,
    Dutch,
    Portuguese,
    Russian,
    ChineseTraditional,
}

impl Language {
    pub const ALL: [Language; 12] = [
        Language::Japanese,
        Language::English,
        Language::French,
        Language::German,
        Language::Italian,
        Language::Spanish,
        Language::ChineseSimplified,
        Language::Korean,
        Language::Dutch,
        Language::Portuguese,
        Language::Russian,
        Language::ChineseTraditional,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Language::Japanese => "Japanese",
            Language::English => "English",
            Language::French => "French",
            Language::German => "German",
            Language::Italian => "Italian",
            Language::Spanish => "Spanish",
            Language::ChineseSimplified => "Chinese (Simplified)",
            Language::Korean => "Korean",
            Language::Dutch => "Dutch",
            Language::Portuguese => "Portuguese",
            Language::Russian => "Russian",
            Language::ChineseTraditional => "Chinese (Traditional)",
        }
    }

    /// the number the console's blocks give the language.
    pub fn code(self) -> u8 {
        match self {
            Language::Japanese => 0,
            Language::English => 1,
            Language::French => 2,
            Language::German => 3,
            Language::Italian => 4,
            Language::Spanish => 5,
            Language::ChineseSimplified => 6,
            Language::Korean => 7,
            Language::Dutch => 8,
            Language::Portuguese => 9,
            Language::Russian => 10,
            Language::ChineseTraditional => 11,
        }
    }
}

/// what the console's cameras see.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CameraSource {
    /// the computer's camera, on only while a game takes pictures.
    #[default]
    Webcam,
    /// a picture from a file, a QR code's say.
    Picture,
    /// nothing, black.
    Nothing,
}

impl CameraSource {
    pub const ALL: [CameraSource; 3] = [CameraSource::Webcam, CameraSource::Picture, CameraSource::Nothing];

    pub fn name(self) -> &'static str {
        match self {
            CameraSource::Webcam => "The computer's camera",
            CameraSource::Picture => "A picture",
            CameraSource::Nothing => "Nothing",
        }
    }
}

// input: the key and controller bindings live in controls.rs (lists of keys
// and buttons per console button, the PC actions, the default scheme).
pub use crate::controls::{Keys, PadButtons};

/// what a game has of its own, over the general settings. each one left
/// out follows them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GameSettings {
    pub hardware_rasterizer: Option<bool>,
    pub resolution: Option<u32>,
    pub texture_packs: Option<bool>,
    pub frame_interpolation: Option<bool>,
    pub scale: Option<u32>,
    #[serde(deserialize_with = "or_default")]
    pub layout: Option<Screens>,
    #[serde(deserialize_with = "or_default")]
    pub orientation: Option<Orientation>,
    #[serde(deserialize_with = "or_default")]
    pub filter: Option<Filter>,
    pub integer_scale: Option<bool>,
    #[serde(deserialize_with = "or_default")]
    pub top_shape: Option<TopShape>,
    #[serde(deserialize_with = "or_default")]
    pub stereo: Option<Stereo3d>,
    pub depth_3d: Option<u32>,
    #[serde(deserialize_with = "or_default")]
    pub language: Option<Language>,
    pub recompiled: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// the folder the library lists games from.
    pub games: Option<PathBuf>,
    /// the console's system language, which titles pick their localised
    /// text and files by.
    #[serde(deserialize_with = "or_default")]
    pub language: Language,
    #[serde(deserialize_with = "or_default")]
    pub renderer: Renderer,
    /// draw the 3D on the GPU rather than in software.
    pub hardware_rasterizer: bool,
    /// how many times the console's resolution the GPU draws the 3D at.
    pub resolution: u32,
    /// draw the texture packs put in games' mods folders.
    pub texture_packs: bool,
    /// draw a picture in between each two of a game at 30 frames a second.
    pub frame_interpolation: bool,
    /// run a game's recompiled code when it has some, rather than
    /// interpreting everything.
    pub recompiled: bool,
    /// window size, times the console's.
    pub scale: u32,
    #[serde(deserialize_with = "or_default")]
    pub layout: Screens,
    #[serde(deserialize_with = "or_default")]
    pub orientation: Orientation,
    /// how the screens' pixels look scaled up.
    #[serde(deserialize_with = "or_default")]
    pub filter: Filter,
    /// scale the screens by whole numbers only, leaving a border.
    pub integer_scale: bool,
    /// how wide the top screen shows.
    #[serde(deserialize_with = "or_default")]
    pub top_shape: TopShape,
    /// how the 3D shows.
    #[serde(deserialize_with = "or_default")]
    pub stereo: Stereo3d,
    /// how deep games draw the 3D, the console's slider, 0 to 100.
    pub depth_3d: u32,
    /// each eye gets the other's picture.
    pub swap_eyes: bool,
    /// 0 to 1.
    pub volume: f32,
    pub mute: bool,
    pub show_fps: bool,
    /// a picture behind the library.
    pub background: Option<PathBuf>,
    /// how much of it shows, 0 to 1.
    pub background_opacity: f32,
    pub keys: Keys,
    pub pad: PadButtons,
    /// the enhancements turned on, by name, for each game by its title id.
    pub enhancements: std::collections::BTreeMap<String, Vec<String>>,
    /// what each game has of its own, by its title id.
    pub per_game: std::collections::BTreeMap<String, GameSettings>,
    /// what the console's cameras see, and the picture when it is one.
    #[serde(deserialize_with = "or_default")]
    pub camera: CameraSource,
    pub camera_picture: Option<PathBuf>,
    /// the name the player goes by in rooms, the console's own too.
    pub nickname: String,
    /// the number that tells this console apart over local wireless, made
    /// once.
    pub console_id: u64,
    /// the room last joined, and the one opened, its name and port.
    pub room_address: String,
    pub room_name: String,
    pub room_port: u16,
    /// the PC port's own options, shown in its Fates-styled options menu.
    pub pc: PcSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            games: None,
            language: Language::English,
            renderer: Renderer::Vulkan,
            hardware_rasterizer: true,
            resolution: 3,
            texture_packs: true,
            frame_interpolation: false,
            recompiled: true,
            scale: 2,
            layout: Screens::Stacked,
            orientation: Orientation::North,
            filter: Filter::Smooth,
            integer_scale: false,
            top_shape: TopShape::Console,
            stereo: Stereo3d::Off,
            depth_3d: 50,
            swap_eyes: false,
            volume: 1.0,
            mute: false,
            show_fps: false,
            background: None,
            background_opacity: 0.35,
            keys: Keys::default(),
            pad: PadButtons::default(),
            enhancements: std::collections::BTreeMap::new(),
            per_game: std::collections::BTreeMap::new(),
            camera: CameraSource::Webcam,
            camera_picture: None,
            nickname: String::new(),
            console_id: 0,
            room_address: String::new(),
            room_name: String::new(),
            room_port: zakuro_core::room::DEFAULT_PORT,
            pc: PcSettings::default(),
        }
    }
}

/// a setting's value, or its default when the file holds one this build
/// does not know, a newer Zakuro's say, so that one value does not lose all
/// the others.
fn or_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    let value = toml::Value::deserialize(deserializer)?;
    Ok(T::deserialize(value).unwrap_or_default())
}

impl GameSettings {
    /// the settings with these over them.
    fn over(&self, settings: &mut Settings) {
        settings.hardware_rasterizer = self.hardware_rasterizer.unwrap_or(settings.hardware_rasterizer);
        settings.resolution = self.resolution.unwrap_or(settings.resolution);
        settings.texture_packs = self.texture_packs.unwrap_or(settings.texture_packs);
        settings.frame_interpolation = self.frame_interpolation.unwrap_or(settings.frame_interpolation);
        settings.scale = self.scale.unwrap_or(settings.scale);
        settings.layout = self.layout.unwrap_or(settings.layout);
        settings.orientation = self.orientation.unwrap_or(settings.orientation);
        settings.filter = self.filter.unwrap_or(settings.filter);
        settings.integer_scale = self.integer_scale.unwrap_or(settings.integer_scale);
        settings.top_shape = self.top_shape.unwrap_or(settings.top_shape);
        settings.stereo = self.stereo.unwrap_or(settings.stereo);
        settings.depth_3d = self.depth_3d.unwrap_or(settings.depth_3d);
        settings.language = self.language.unwrap_or(settings.language);
        settings.recompiled = self.recompiled.unwrap_or(settings.recompiled);
    }
}

/// what a game goes by in the settings file, its title id.
pub fn game_key(program_id: u64) -> String {
    format!("{program_id:016X}")
}

impl Settings {
    /// the settings a game plays with, or the general ones without a game.
    pub fn for_game(&self, program_id: Option<u64>) -> Settings {
        let mut settings = self.clone();
        if let Some(own) = program_id.and_then(|id| self.per_game.get(&game_key(id))) {
            own.over(&mut settings);
        }
        settings
    }

    /// whether a game has anything of its own.
    pub fn has_own(&self, program_id: u64) -> bool {
        self.per_game.contains_key(&game_key(program_id))
    }

    /// where the console's 3D slider is, down while the 3D is off, so games
    /// do not draw a picture nobody sees.
    pub fn slider_3d(&self) -> f32 {
        if self.stereo == Stereo3d::Off {
            0.0
        } else {
            self.depth_3d.min(100) as f32 / 100.0
        }
    }

    /// the 3D the presenter shows.
    pub fn stereo(&self) -> zakuro_gpu::Stereo {
        zakuro_gpu::Stereo { mode: self.stereo.mode(), swap: self.swap_eyes }
    }

    /// the saved settings, or the defaults when there are none.
    pub fn load() -> Settings {
        let Some(path) = path() else { return Settings::default() };
        match std::fs::read_to_string(&path) {
            Ok(text) => {
                let mut settings: Settings = toml::from_str(&text).unwrap_or_else(|error| {
                    // kept aside, the next save would write over it
                    let aside = path.with_extension("toml.bad");
                    let _ = std::fs::copy(&path, &aside);
                    log::warn!("could not read {}, {error}, using the defaults, it is kept as {}", path.display(), aside.display());
                    Settings::default()
                });
                crate::haiku_versions::settings_loaded(&mut settings); // versions2: the playing version's own
                // bottom2: settings from older versions, brought up once
                if settings.pc.migrate() {
                    settings.save();
                }
                settings
            }
            Err(_) => {
                let mut settings = Settings::default();
                crate::haiku_versions::settings_loaded(&mut settings); // versions2
                settings
            }
        }
    }

    pub fn save(&self) {
        let Some(path) = path() else { return };
        let file = crate::haiku_versions::settings_saved(self); // versions2: into the playing version's own
        let written = toml::to_string_pretty(&*file)
            .map_err(|error| error.to_string())
            .and_then(|text| write(&path, &text).map_err(|error| error.to_string()));
        if let Err(error) = written {
            log::warn!("could not save the settings to {}, {error}", path.display());
        }
    }
}

fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, text)
}

/// the system's place for a program's settings.
fn path() -> Option<PathBuf> {
    let var = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
    let dir = if cfg!(windows) {
        var("APPDATA")
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|home| home.join("Library/Application Support"))
    } else {
        var("XDG_CONFIG_HOME").or_else(|| var("HOME").map(|home| home.join(".config")))
    };
    dir.map(|dir| dir.join("haiku").join("settings.toml"))
}

#[cfg(test)]
mod tests {
    /// qol3: a file from before the SpotPass setting went still reads, its
    /// other settings kept, and the line is not written back.
    #[test]
    fn an_old_spotpass_line_is_ignored() {
        let old: super::Settings = toml::from_str("[pc]\nask_spotpass = true\nskip_story = true\ngame_speed = 3\n").expect("parses");
        assert!(old.pc.skip_story);
        assert_eq!(old.pc.game_speed, 3);
        let text = toml::to_string(&old).expect("writes");
        assert!(!text.contains("ask_spotpass"));
    }

    /// qol4: the targeting lines are on or off. Older files read: "all" and
    /// "cursor" are On; "off" from before version 2 (the old default,
    /// written back) becomes On once, and a later Off is kept.
    #[test]
    fn targeting_lines_are_on_or_off() {
        use super::TargetingLines;
        assert_eq!(super::Settings::default().pc.targeting_lines, TargetingLines::On);
        for (name, lines) in [("all", TargetingLines::On), ("cursor", TargetingLines::On), ("on", TargetingLines::On), ("off", TargetingLines::Off)] {
            let file: super::Settings = toml::from_str(&format!("[pc]
settings_version = 2
targeting_lines = \"{name}\"
")).expect("parses");
            assert_eq!(file.pc.targeting_lines, lines, "{name}");
        }
        let mut old: super::Settings = toml::from_str("[pc]
settings_version = 1
targeting_lines = \"off\"
skip_story = true
").expect("parses");
        assert!(old.pc.migrate());
        assert_eq!(old.pc.targeting_lines, TargetingLines::On);
        assert!(old.pc.skip_story);
        old.pc.targeting_lines = TargetingLines::Off;
        let text = toml::to_string(&old).expect("writes");
        assert!(text.contains("targeting_lines = \"off\""));
        let mut back: super::Settings = toml::from_str(&text).expect("reads");
        assert!(!back.pc.migrate());
        assert_eq!(back.pc.targeting_lines, TargetingLines::Off);
        let on = toml::to_string(&super::Settings::default()).expect("writes");
        assert!(on.contains("targeting_lines = \"on\""));
    }

    /// bottom2: a settings file from before versions with Panel (the old
    /// default, written back) is moved to Auto once; other modes are kept;
    /// one without the field gets Auto; the floating fields default; after
    /// the migration a chosen Panel stays.
    #[test]
    fn older_settings_keep_their_bottom_screen() {
        let mut old: super::Settings = toml::from_str("[pc]\nbottom_screen = \"panel\"\nbottom_opacity = 0.85\n").expect("parses");
        assert_eq!(old.pc.bottom_screen, super::BottomScreen::Panel);
        assert_eq!(old.pc.settings_version, 0);
        assert!(old.pc.migrate());
        assert_eq!(old.pc.bottom_screen, super::BottomScreen::Auto);
        assert_eq!(old.pc.settings_version, super::SETTINGS_VERSION);
        assert!(!old.pc.migrate(), "once only");
        // chosen again after the migration, Panel stays
        old.pc.bottom_screen = super::BottomScreen::Panel;
        let text = toml::to_string(&old).expect("writes");
        let mut back: super::Settings = toml::from_str(&text).expect("reads");
        assert!(!back.pc.migrate());
        assert_eq!(back.pc.bottom_screen, super::BottomScreen::Panel);
        for (name, mode) in [("overlay", super::BottomScreen::Overlay), ("on_demand", super::BottomScreen::OnDemand), ("classic", super::BottomScreen::Classic)] {
            let mut file: super::Settings = toml::from_str(&format!("[pc]\nbottom_screen = \"{name}\"\n")).expect("parses");
            assert!(file.pc.migrate());
            assert_eq!(file.pc.bottom_screen, mode, "an older file's other modes are kept");
        }
        assert_eq!(super::Settings::default().pc.settings_version, super::SETTINGS_VERSION);
        let old: super::Settings = toml::from_str("[pc]\nbottom_screen = \"panel\"\nbottom_opacity = 0.85\n").expect("parses");
        assert_eq!(old.pc.bottom_opacity, 0.85);
        assert_eq!(old.pc.bottom_anchor, super::BottomAnchor::Right);
        assert!((old.pc.bottom_height - 0.33).abs() < 1e-6);
        for (name, mode) in [("overlay", super::BottomScreen::Overlay), ("on_demand", super::BottomScreen::OnDemand), ("classic", super::BottomScreen::Classic), ("auto", super::BottomScreen::Auto)] {
            let file: super::Settings = toml::from_str(&format!("[pc]\nbottom_screen = \"{name}\"\n")).expect("parses");
            assert_eq!(file.pc.bottom_screen, mode);
        }
        let fresh: super::Settings = toml::from_str("").expect("parses");
        assert_eq!(fresh.pc.bottom_screen, super::BottomScreen::Auto);
        let missing: super::Settings = toml::from_str("[pc]\nvsync = true\n").expect("parses");
        assert_eq!(missing.pc.bottom_screen, super::BottomScreen::Auto);
        // and they go round a save
        let mut placed = super::Settings::default();
        placed.pc.bottom_anchor = super::BottomAnchor::TopLeft;
        placed.pc.bottom_x = 0.1;
        let text = toml::to_string(&placed).expect("writes");
        let back: super::Settings = toml::from_str(&text).expect("reads");
        assert_eq!(back.pc.bottom_anchor, super::BottomAnchor::TopLeft);
        assert_eq!(back.pc.bottom_x, 0.1);
    }

    use super::*;
    use winit::keyboard::KeyCode;

    #[test]
    fn settings_come_back_as_they_were_saved() {
        let mut settings =
            Settings { games: Some("/games".into()), show_fps: true, layout: Screens::TopOnly, language: Language::German, ..Settings::default() };
        settings.keys.a.set(KeyCode::KeyK);
        settings.pad.a.set(crate::controls::PadButton::South);
        let text = toml::to_string_pretty(&settings).unwrap();
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
    }

    #[test]
    fn a_value_this_build_does_not_know_keeps_the_others() {
        let settings: Settings = toml::from_str("show_fps = true\nlayout = \"three_screens\"\nfilter = \"crt\"\n").unwrap();
        assert!(settings.show_fps);
        assert_eq!(settings.layout, Screens::default());
        assert_eq!(settings.filter, Filter::default());
    }

    #[test]
    fn missing_fields_take_their_defaults() {
        let settings: Settings = toml::from_str("show_fps = true\n[keys]\na = \"KeyK\"\n").unwrap();
        assert!(settings.show_fps);
        assert_eq!(settings.keys.a, crate::controls::Binds::of(&[KeyCode::KeyK]));
        assert_eq!(settings.keys.b, Keys::default().b);
        assert_eq!(settings.scale, 2);
        assert_eq!(settings.layout, Screens::Stacked);
    }

    /// a game plays with what it has of its own, and with the general
    /// settings in everything else, and no other game is touched.
    #[test]
    fn a_game_has_its_own_settings_over_the_general_ones() {
        let mut settings = Settings { resolution: 2, layout: Screens::SideBySide, ..Settings::default() };
        settings.per_game.insert(
            game_key(0x0004_0000_0012_3400),
            GameSettings { orientation: Some(Orientation::East), resolution: Some(5), recompiled: Some(false), ..GameSettings::default() },
        );
        let game = settings.for_game(Some(0x0004_0000_0012_3400));
        assert_eq!(game.orientation, Orientation::East);
        assert_eq!(game.resolution, 5);
        assert!(!game.recompiled);
        assert_eq!(game.layout, Screens::SideBySide);
        for other in [Some(0x0004_0000_0012_3500), None] {
            let general = settings.for_game(other);
            assert_eq!(general, settings);
        }
        assert!(settings.has_own(0x0004_0000_0012_3400));
        assert!(!settings.has_own(0x0004_0000_0012_3500));
    }

    #[test]
    fn a_games_settings_come_back_as_they_were_saved() {
        let mut settings = Settings::default();
        settings.per_game.insert(
            game_key(0x0004_0000_0012_3400),
            GameSettings { orientation: Some(Orientation::West), language: Some(Language::Japanese), texture_packs: Some(false), ..GameSettings::default() },
        );
        let text = toml::to_string_pretty(&settings).unwrap();
        let own = text.split("[per_game.").nth(1).unwrap();
        assert!(!own.contains("resolution"), "only what it has of its own is written for a game");
        assert_eq!(toml::from_str::<Settings>(&text).unwrap(), settings);
    }

    /// a value of a newer Zakuro leaves the game following the general
    /// setting, and takes nothing else with it.
    #[test]
    fn a_value_this_build_does_not_know_follows_the_general_setting() {
        let text = "[per_game.0004000000123400]\nlayout = \"three_screens\"\nresolution = 4\n";
        let settings: Settings = toml::from_str(text).unwrap();
        let own = &settings.per_game[&game_key(0x0004_0000_0012_3400)];
        assert_eq!(own.layout, None);
        assert_eq!(own.resolution, Some(4));
    }

    #[test]
    fn layouts_go_round() {
        let mut layout = Screens::Stacked;
        for _ in 0..Screens::ALL.len() {
            layout = layout.next();
        }
        assert_eq!(layout, Screens::Stacked);
    }
}

/// how the window covers the display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    #[default]
    Windowed,
    /// a window without a border the size of the display.
    Borderless,
    /// the display switched to the chosen resolution and refresh rate.
    Exclusive,
}

/// the shape the top screen's picture is drawn at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AspectMode {
    /// the console's 5:3.
    Native,
    /// as wide as the window or display allows.
    #[default]
    Auto,
    Wide16x9,
    Wide21x9,
    Wide32x9,
}

/// wide-ui: what fills the wide sides of the top screen where its content
/// is 5:3 by nature (menus, the title, talk scenes over pictures, telops).
/// The game world (maps, battles, My Castle, 3D scenes) is always drawn
/// truly wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WideSides {
    /// side panels made from the game's own ornamental art, framing the 5:3.
    #[default]
    Artwork,
    /// the backdrop's own edges carried on, softened (the earlier look).
    Extend,
    /// plain black.
    Black,
}

impl WideSides {
    /// the value zakuro_gpu::wide::set_sides takes.
    pub fn code(self) -> u32 {
        match self {
            WideSides::Artwork => 0,
            WideSides::Extend => 1,
            WideSides::Black => 2,
        }
    }
}

/// where the bottom screen's information goes on a single PC display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BottomScreen {
    /// bottom2: the top screen fills the window and the bottom screen
    /// floats in when the game shows something worth seeing on it, then
    /// goes away again. The default for new settings; a settings file that
    /// names another mode keeps it.
    #[default]
    Auto,
    /// a framed panel docked beside the top screen, in the space ultrawide
    /// and widescreen leave free.
    Panel,
    /// floating over the top screen where it was put, always shown
    /// ("Always shown" in the options).
    Overlay,
    /// floating where it was put, shown only while its button is held, or
    /// toggled ("Manual" in the options).
    OnDemand,
    /// both screens at their console proportions, as the old layouts.
    Classic,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Left,
    #[default]
    Right,
}

/// which face button confirms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ButtonLayout {
    /// by the controller connected: Nintendo controllers by position like
    /// the 3DS, others by their labels.
    #[default]
    Auto,
    /// the 3DS's positions: the east button is A.
    Positional,
    /// the labels: the south button confirms, as on Xbox and PlayStation.
    Labels,
}

/// which button pictures prompts show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Glyphs {
    #[default]
    Auto,
    Nintendo,
    Xbox,
    PlayStation,
    Keyboard,
}

/// the PC port's options. Every field has a default so older settings files
/// still load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PcSettings {
    // display
    #[serde(deserialize_with = "or_default")]
    pub window_mode: WindowMode,
    /// the window's size while windowed, 0 for the default.
    pub window_width: u32,
    pub window_height: u32,
    /// the display mode in exclusive fullscreen, 0 for the desktop's.
    pub fullscreen_width: u32,
    pub fullscreen_height: u32,
    pub refresh_rate: u32,
    /// which display, by its index.
    pub monitor: u32,
    pub vsync: bool,
    /// how many frames a second are presented: 30, 60, 120, 144, 165, 240,
    /// or 0 to follow the display's refresh rate.
    pub frame_rate: u32,
    /// the most frames a second presented, 0 for no limit.
    pub fps_cap: u32,
    #[serde(deserialize_with = "or_default")]
    pub aspect: AspectMode,
    /// multisampling, 1, 2, 4 or 8.
    pub msaa: u32,
    // bottom screen
    #[serde(deserialize_with = "or_default")]
    pub bottom_screen: BottomScreen,
    #[serde(deserialize_with = "or_default")]
    pub bottom_side: Side,
    /// the panel's height relative to the top screen's, 0.3 to 1.
    pub bottom_size: f32,
    /// 0 to 1, for the overlay.
    pub bottom_opacity: f32,
    // controls
    #[serde(deserialize_with = "or_default")]
    pub button_layout: ButtonLayout,
    #[serde(deserialize_with = "or_default")]
    pub glyphs: Glyphs,
    /// 0 to 0.5.
    pub stick_deadzone: f32,
    pub rumble: bool,
    /// the right stick's touch pointer, bottom-screen pixels a second.
    pub touch_cursor_speed: f32,
    // audio
    /// the output device by name, empty for the system's default.
    pub audio_device: String,
    pub mute_unfocused: bool,
    // interface
    /// the options menu's and panels' size, 0.75 to 2.
    pub ui_scale: f32,
    // hfr: run the scenes Fates paces at 30 ticks a second (battles,
    // cutscenes, the title screen) at 60 ticks. Off by default: battles then
    // end about 17% sooner (some of their waits count ticks), see
    // dev/reports/hfr.md; frame interpolation smooths those scenes instead.
    pub game_60fps: bool,
    // gfx-hfr: draw the game with the native renderer (fates-gfx) rather
    // than the emulated GPU ("Renderer: Native / Compatibility"). Read at
    // start; FATES_NATIVE_GFX / FATES_GFX_NATIVE3D override it.
    pub native_graphics: bool,
    // bottom2: where the floating bottom screen (Auto, Manual, Always
    // shown) goes: the window edge or corner it keeps to, its distance from
    // that edge in window heights, and the bottom screen's height in window
    // heights. Dragging it with the mouse writes these.
    #[serde(deserialize_with = "or_default")]
    pub bottom_anchor: BottomAnchor,
    pub bottom_x: f32,
    pub bottom_y: f32,
    pub bottom_height: f32,
    /// bottom2: Auto puts it away this many seconds after it stops mattering.
    pub bottom_linger: f32,
    /// Haiku: which moments bring the bottom screen out in Auto: the card
    /// of the unit under the map cursor (and the unit list), the forecast
    /// while choosing a target, combat to its EXP bar, and tutorial pages
    /// and the avatar creator's touch grids.
    pub bottom_show_units: bool,
    pub bottom_show_forecast: bool,
    pub bottom_show_combat: bool,
    pub bottom_show_guides: bool,
    /// Haiku: how long the map cursor rests on a unit before Auto shows
    /// its card, in seconds.
    pub bottom_show_delay: f32,
    /// bottom2: the version of these settings, for one-time migrations.
    /// Missing in a file (0) means one from before versions; new settings
    /// start at SETTINGS_VERSION.
    #[serde(default)]
    pub settings_version: u32,
    /// wide-ui: what fills the wide sides of screens whose content is 5:3
    /// by nature (menus, the title, talk scenes, telops) on a wider top
    /// screen: the game's own art framing them, the old extension, or black.
    #[serde(deserialize_with = "or_default")]
    pub wide_sides: WideSides,
    // qol: how fast the game runs (1, 2, 3, 4, 5, 8; 0 is Max), the speed
    // the Game Speed key switches to from 1x, Skip Story, and the boon/bane
    // stat panel in the avatar's menu. qol3: `ask_spotpass` is gone (the
    // SpotPass questions are always answered No); an older file's line is
    // ignored when read and left out when saved
    pub game_speed: u32,
    pub game_speed_fast: u32,
    pub skip_story: bool,
    pub boon_bane_stats: bool,
    // cameras: the free camera in battle scenes (off: the game's camera
    // only), and walking in My Castle's third-person view (on: the left
    // stick walks and the right stick turns; off: as the 3DS, the circle
    // pad turns and Corrin stays put). See cameras.rs.
    pub battle_free_camera: bool,
    pub castle_walk: bool,
    // cameras: the free camera orbits the fight rather than flies
    pub free_camera_orbit: bool,
    // randomizer: the seed new randomized runs use ("1A2B-3C4D", any 1-8
    // hex digits), so a run can be shared; empty for a new random seed
    // each time. The New Game randomizer screen shows the seed it uses.
    #[serde(default)]
    pub randomizer_seed: String,
    // info-ui: the enemy targeting lines on the battle map (off, every
    // enemy's, or only the cursor's), and the growth-rates button on the
    // lower screen's unit status. See info_ui.rs.
    pub targeting_lines: TargetingLines,
    pub growth_button: bool,
    // cheats: the Cheats tab's switches ([pc.cheats], all off by default).
    // See cheats.rs.
    pub cheats: crate::cheats::CheatSettings,
    // supports: Corrin can S-support anyone with a Corrin support, of either
    // gender (read when the game starts), and Kana's gender (asked when
    // Kana's paralogue starts, the game's own, or chosen). See
    // zakuro-core/src/supports.rs.
    pub corrin_s_any: bool,
    #[serde(deserialize_with = "or_default")]
    pub kana_gender: KanaGender,
    // transblur: the radial (zoom) blur of the battle transitions, map to
    // fight and back. Off by default: without it the zoom is drawn with
    // pictures in between, at the display's rate. See overrides/transblur.c.
    pub transition_blur: bool,
    // drawdist: Full Draw Distance. On (the default), grass and the field's
    // objects are drawn however far away they are, at full detail, so nothing
    // pops in as the camera nears it; off, the game's own distances. See
    // overrides/drawdist.c.
    pub full_draw_distance: bool,
    // online: online battles' connection ([pc.online]): the host's port,
    // the STUN server, the router's port mapping. See online_ui.rs and
    // zakuro-core/src/netplay.
    pub online: OnlineSettings,
    // pulse: Divine Pulse, the modern games' rewind (off by default), and
    // its charges: Three Houses' story scaling, a fixed number a map
    // (divine_pulse_fixed, Engage's 10 on Hard and Maddening), or
    // unlimited (Engage's Normal). See pulse.rs.
    pub divine_pulse: bool,
    #[serde(deserialize_with = "or_default")]
    pub divine_pulse_charges: PulseCharges,
    pub divine_pulse_fixed: u32,
    // pulse: save states, under the Cheats tab (off by default), and the
    // slot the quick-save and quick-load keys use (0-7). See states.rs.
    pub save_states: bool,
    pub save_state_slot: u32,
    // jpvoice: the voices the game speaks, English (the game's own) or
    // Japanese, from the voice layer Haiku Setup extracts into
    // data/voices-ja (haiku_voices.rs). Takes effect when the game starts.
    #[serde(deserialize_with = "or_default")]
    pub voices: Voices,
    // jpprep: on the Japanese game (Fire Emblem if) Haiku's own menus are
    // Japanese; this shows them in English instead (for an English
    // translation mod). No effect on the USA and EUR games.
    pub japanese_english_menus: bool,
    // zigdl: which compiler code mods are compiled with when the computer
    // has none: asked the first time (when Haiku Setup's compiler is there
    // too), Zig downloaded once, or the compiler that came with Haiku.
    #[serde(deserialize_with = "or_default")]
    pub code_mods_compiler: CodeModsCompiler,
    // versions: the version of the game Haiku plays when more than one is
    // installed ("usa", "eur" or "jpn"; empty: the first installed). Haiku.exe
    // reads it at start (haiku-setup's layout.rs); Options, Interface, Game
    // Version sets it, and a restart applies it.
    pub game_version: String,
    // versions2: what each version of the game keeps of its own, by its
    // tag ("usa", "eur", "jpn"): the game language and the voices
    // ([pc.per_version.<tag>]). The version playing reads its entry at
    // start into `language` and `voices` above (haiku_versions.rs), which
    // the options change as before; a save writes them back into its
    // entry, the shared lines kept as they were (the default for a version
    // without an entry). Only in a Haiku folder with games\<version>\.
    pub per_version: std::collections::BTreeMap<String, VersionSettings>,
    // updater: Options > Interface > Check for Updates: once a day, ask
    // GitHub's API whether a newer Haiku is out (haiku_update.rs). Off: no
    // network at all.
    pub check_updates: bool,
    // hfr60: Options > Display > Smooth 30 FPS Scenes: at a frame rate of
    // 60 (Frame Rate 60, a 60 Hz display, a cap of 60), the scenes the game
    // itself runs at 30 (battles, 3D cutscenes, the title screen) get a
    // picture in between each two of theirs; 60 FPS scenes are untouched.
    // Above 60 every scene is smoothed whatever this says (zakuro hfr.rs).
    pub smooth_30fps: bool,
    // vrrcap: Options > Display > Variable Refresh: the display has a
    // variable refresh rate on (G-SYNC, FreeSync). Presents then keep to a
    // steady clock of their own a little under the display's highest rate
    // (zakuro hfr.rs vrr_limit), never to its refresh grid: at the top of
    // the range the display falls out of variable refresh.
    pub vrr: bool,
    // castlegifts: Options > Gameplay > Online Rewards: the gifts the 3DS
    // only gave through online play and StreetPass, given through the
    // game's own menus (overrides/castlegifts.c): My Castle's Inbox >
    // Rewards lets each Battle and Visit Bonus be claimed once, and the
    // title's Extras > Presents downloads its two StreetPass presents.
    // Off: those stay locked, as on a 3DS with no one to meet online.
    pub online_rewards: bool,
    // bossmusic: Options > Audio > Boss Music Stays (GitHub issue #20):
    // once a boss has been fought, its battle theme stays on as the map's
    // music until it is defeated or gone, as in Three Houses and Engage.
    // Off by default (the 3DS's own music). See overrides/bossmusic.c.
    pub boss_music: bool,
}

/// versions2: one version's own settings (PcSettings::per_version); each
/// left out follows the shared one.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VersionSettings {
    #[serde(deserialize_with = "or_default")]
    pub language: Option<Language>,
    #[serde(deserialize_with = "or_default")]
    pub voices: Option<Voices>,
}

/// pulse: how many charges of Divine Pulse a map gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PulseCharges {
    /// Three Houses: 3 at first, more as the story goes on (13 at most, 10 on Lunatic)
    #[default]
    Story,
    /// the same number every map
    Fixed,
    Unlimited,
}

/// online: how online battles connect, saved as `[pc.online]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OnlineSettings {
    /// the UDP port a host listens on; 0 for Haiku's own (24874, or the
    /// next free one).
    pub port: u16,
    /// the STUN server asked for the host's public address (host:port),
    /// empty for none.
    pub stun_server: String,
    /// ask the router to open the port (UPnP IGD).
    pub upnp: bool,
    /// ask the router to open the port (NAT-PMP / PCP).
    pub nat_pmp: bool,
    /// put the host's IPv6 address in the code when it has one.
    pub ipv6: bool,
    /// the joiner also looks for the host on its own network.
    pub lan_discovery: bool,
    /// seconds without a word from the other player before it counts as gone.
    pub timeout_secs: u32,
    /// online2: the host lets characters the Cheats tab changed into the
    /// battle (the Host panel's "Characters modified with cheats").
    pub allow_cheated_units: bool,
}

impl Default for OnlineSettings {
    fn default() -> Self {
        OnlineSettings {
            port: 0,
            stun_server: zakuro_core::netplay::DEFAULT_STUN.to_owned(),
            upnp: true,
            nat_pmp: true,
            ipv6: true,
            lan_discovery: true,
            timeout_secs: 15,
            allow_cheated_units: false,
        }
    }
}

/// zigdl: the compiler for code mods (PcSettings::code_mods_compiler).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CodeModsCompiler {
    #[default]
    Ask,
    Zig,
    Haiku,
}

/// jpvoice: the language of the voices (PcSettings::voices).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Voices {
    #[default]
    English,
    Japanese,
}

/// supports: Kana's gender. Ask shows the game's own three-choice dialog
/// (Male / Female / Random) when Kana's paralogue starts; Vanilla is the
/// game's own (a male Corrin has a daughter, a female Corrin a son).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KanaGender {
    #[default]
    Ask,
    Vanilla,
    Male,
    Female,
    Random,
}

impl KanaGender {
    pub fn core(self) -> zakuro_core::supports::KanaGender {
        use zakuro_core::supports::KanaGender as K;
        match self {
            KanaGender::Ask => K::Ask,
            KanaGender::Vanilla => K::Vanilla,
            KanaGender::Male => K::Male,
            KanaGender::Female => K::Female,
            KanaGender::Random => K::Random,
        }
    }
}

/// info-ui: whether the battle map shows the enemy targeting lines.
/// qol4: just on or off, on by default (the user's request). On is the
/// full behaviour: every enemy that would attack next enemy phase, and the
/// lines following a unit while it is moved. Files from before read as
/// before: "all" and "cursor" (the Cursor Only choice, gone) are On.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetingLines {
    Off,
    #[default]
    #[serde(alias = "all", alias = "cursor")]
    On,
}

/// bottom2: the settings' current version. 1: the bottom screen's Panel,
/// which older versions wrote back as their default, becomes Auto.
pub const SETTINGS_VERSION: u32 = 2; // qol4: 2, the targeting lines on

impl PcSettings {
    /// bottom2: brings settings from an older version up to date, once:
    /// `bottom_screen = "panel"` in a file from before version 1 was the
    /// old default written back on save, not a choice, so it becomes Auto.
    /// Afterwards whatever the player picks is kept. Whether it changed
    /// anything, so the file is saved.
    pub fn migrate(&mut self) -> bool {
        if self.settings_version >= SETTINGS_VERSION {
            return false;
        }
        if self.settings_version < 1 && self.bottom_screen == BottomScreen::Panel {
            log::info!("settings: the bottom screen goes from Panel, the old default, to Auto");
            self.bottom_screen = BottomScreen::Auto;
        }
        // qol4: the targeting lines are on by default now; a file from
        // before version 2 wrote Off back as the old default, not a choice
        if self.settings_version < 2 && self.targeting_lines == TargetingLines::Off {
            log::info!("settings: the enemy targeting lines go from Off, the old default, to On");
            self.targeting_lines = TargetingLines::On;
        }
        self.settings_version = SETTINGS_VERSION;
        true
    }
}

/// bottom2: the edge or corner of the window a floating bottom screen keeps
/// to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BottomAnchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    // bottom2: the right edge's middle, clear of the top screen's corners,
    // where Fates puts its forecast, terrain and unit windows
    #[default]
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl BottomAnchor {
    pub const ALL: [BottomAnchor; 8] = [
        BottomAnchor::TopLeft,
        BottomAnchor::Top,
        BottomAnchor::TopRight,
        BottomAnchor::Right,
        BottomAnchor::BottomRight,
        BottomAnchor::Bottom,
        BottomAnchor::BottomLeft,
        BottomAnchor::Left,
    ];

    pub fn name(self) -> &'static str {
        match self {
            BottomAnchor::TopLeft => "Top Left",
            BottomAnchor::Top => "Top",
            BottomAnchor::TopRight => "Top Right",
            BottomAnchor::Left => "Left",
            BottomAnchor::Right => "Right",
            BottomAnchor::BottomLeft => "Bottom Left",
            BottomAnchor::Bottom => "Bottom",
            BottomAnchor::BottomRight => "Bottom Right",
        }
    }
}

impl Default for PcSettings {
    fn default() -> Self {
        PcSettings {
            window_mode: WindowMode::Windowed,
            window_width: 0,
            window_height: 0,
            fullscreen_width: 0,
            fullscreen_height: 0,
            refresh_rate: 0,
            monitor: 0,
            vsync: true,
            frame_rate: 0,
            fps_cap: 0,
            aspect: AspectMode::Auto,
            msaa: 1,
            bottom_screen: BottomScreen::Auto, // bottom2
            bottom_side: Side::Right,
            // wide: 0.4 leaves room for a widescreen top screen beside the
            // panel on a 21:9 display, see bottom_panel::panel_top_aspect
            bottom_size: 0.4,
            // bottom2: solid, the floating screen carries text to read
            bottom_opacity: 1.0,
            button_layout: ButtonLayout::Auto,
            glyphs: Glyphs::Auto,
            stick_deadzone: 0.15,
            rumble: true,
            touch_cursor_speed: 400.0,
            audio_device: String::new(),
            mute_unfocused: false,
            ui_scale: 1.0,
            game_60fps: false, // hfr
            native_graphics: true, // gfx-hfr
            // bottom2: the right edge's middle, a third of the window high
            bottom_anchor: BottomAnchor::Right,
            bottom_x: 0.02,
            bottom_y: 0.0,
            bottom_height: 0.33,
            bottom_linger: 1.0,
            bottom_show_units: true,
            bottom_show_forecast: true,
            bottom_show_combat: true,
            bottom_show_guides: true,
            bottom_show_delay: 0.3,
            settings_version: SETTINGS_VERSION,
            wide_sides: WideSides::Artwork, // wide-ui
            // qol
            game_speed: 1,
            game_speed_fast: 2,
            skip_story: false,
            boon_bane_stats: true,
            // cameras
            battle_free_camera: false,
            castle_walk: true,
            free_camera_orbit: false,
            randomizer_seed: String::new(), // randomizer
            // info-ui
            targeting_lines: TargetingLines::On, // qol4: on by default
            growth_button: true,
            cheats: crate::cheats::CheatSettings::default(), // cheats
            // supports
            corrin_s_any: true,
            kana_gender: KanaGender::Ask,
            transition_blur: false, // transblur
            full_draw_distance: true, // drawdist
            online: OnlineSettings::default(), // online
            // pulse
            divine_pulse: false,
            divine_pulse_charges: PulseCharges::Story,
            divine_pulse_fixed: 10,
            save_states: false,
            save_state_slot: 0,
            voices: Voices::English, // jpvoice
            japanese_english_menus: false, // jpprep
            code_mods_compiler: CodeModsCompiler::Ask, // zigdl
            game_version: String::new(), // versions
            per_version: std::collections::BTreeMap::new(), // versions2
            check_updates: true, // updater
            smooth_30fps: true, // hfr60
            vrr: false, // vrrcap
            online_rewards: true, // castlegifts
            boss_music: false, // bossmusic
        }
    }
}
