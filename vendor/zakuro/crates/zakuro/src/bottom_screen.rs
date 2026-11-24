//! bottom: the 3DS's bottom screen on a single PC display.
//!
//! pc.bottom_screen picks how it shows (bottom2): Auto (the default) floats
//! it in over the top screen when the game shows something worth seeing on
//! it (bottom_scene) and away again after; Manual floats it in while its
//! button is held or after a tap of it; Always shown floats it where it was
//! put; Panel docks it beside the top screen; Classic is the console's own
//! arrangements. The floating window can be dragged by its frame, resized
//! by its corners (keeping 4:3) and snaps to the window's edges; it keeps
//! to an edge or corner (pc.bottom_anchor) so a resized window keeps it
//! sensible. The frame
//! and the window's background are the game's own art, from assets/ui:
//! SystemWindow_/Dialog.png's border for the frame, Wallpaper_/Wallpaper.png's
//! bottom-screen wallpaper behind everything, and SystemWindow2's pointing
//! hand for the controller's touch cursor.
//!
//! the layout math is zakuro_gpu::backend::bottom_panel, shared with the
//! presenter, so a touch lands where the screen is drawn.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use zakuro_core::services::hid::InputState;
use zakuro_gpu::backend::bottom_panel::{self as panel, Anchor, Composition, PanelMode, PanelParams, PanelPlaces, PanelSide, Placement};
use zakuro_gpu::{Overlay, OverlayMesh, OverlayTexture, OverlayVertex, StereoMode, Viewport};

use crate::bottom_scene::{AutoReveal, Change, Reading, Reason};
use crate::settings::{BottomAnchor, BottomScreen as Mode, Orientation, PcSettings, Screens, Settings, Side};

/// texture ids, well clear of the interface's.
const WALLPAPER: u64 = 0xB077_0000_0000_0001;
const DIALOG: u64 = 0xB077_0000_0000_0002;
const HAND: u64 = 0xB077_0000_0000_0003;

/// how long the on-demand button has to be held to count as a hold, which
/// hides the panel again when let go, rather than a tap, which toggles it.
/// l3toggle: only On Demand has the hold (its option says so: shown while
/// held); in Auto, Overlay and Panel letting go does nothing. It was 250 ms
/// in every mode, so a firm click of L3 was a hold and its release undid
/// the toggle (a player's report). A stick's click is slower than a key's.
const HOLD: Duration = Duration::from_millis(500);
/// l3toggle: and the polls it has to be seen held over, so a hitch (one long
/// frame between the press and the release) does not make a click a hold.
const HOLD_POLLS: u32 = 8;
/// how long sliding in or out takes.
const SLIDE: f32 = 0.22;
/// the touch cursor stays this long after it was last used, then fades.
const CURSOR_STAY: f32 = 1.5;
const CURSOR_FADE: f32 = 0.4;
/// the right stick's lean that moves the cursor.
const STICK_DEADZONE: f32 = 0.2;
/// how dark the wallpaper is behind the screens, so it does not draw the eye.
const WALLPAPER_SHADE: f32 = 0.5;

/// an RGBA picture, premultiplied, as the overlay textures want.
struct Picture {
    size: [u32; 2],
    pixels: Vec<u8>,
}

impl Picture {
    fn texture(&self, id: u64, linear: bool) -> OverlayTexture {
        OverlayTexture { id, offset: None, size: self.size, pixels: self.pixels.clone(), linear }
    }
}

struct Art {
    wallpaper: Picture,
    dialog: Picture,
    hand: Picture,
}

/// where the game's UI art is: the ui kit's assets folder (FATES_ROOT or the
/// program's folder, else the working directory), or FATES_ASSETS.
fn assets_dir() -> Option<PathBuf> {
    let dir = std::env::var_os("FATES_ASSETS").map(PathBuf::from).unwrap_or_else(crate::fates_ui::kit::assets_dir);
    dir.join("ui").is_dir().then_some(dir)
}

fn load(path: PathBuf) -> Option<image::RgbaImage> {
    image::open(&path).inspect_err(|error| log::warn!("bottom screen art {}: {error}", path.display())).ok().map(|image| image.to_rgba8())
}

fn premultiplied(image: &image::RgbaImage) -> Picture {
    let mut pixels = image.as_raw().clone();
    for pixel in pixels.chunks_exact_mut(4) {
        let alpha = pixel[3] as u32;
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u32 * alpha + 127) / 255) as u8;
        }
    }
    Picture { size: [image.width(), image.height()], pixels }
}

impl Art {
    fn load() -> Option<Art> {
        let ui = assets_dir()?.join("ui");
        // the bottom screen's wallpaper is stored turned a quarter, 240x320
        // at (16, 416) under the top screen's
        let sheet = load(ui.join("Wallpaper_").join("Wallpaper.png"))?;
        let wallpaper = image::imageops::crop_imm(&sheet, 16, 416, 240, 320).to_image();
        let wallpaper = image::imageops::rotate270(&wallpaper);
        let dialog = load(ui.join("SystemWindow_").join("Dialog.png"))?;
        let sheet = load(ui.join("SystemWindow2_").join("SystemWindow2.png"))?;
        let hand = image::imageops::crop_imm(&sheet, HAND_AT.0, HAND_AT.1, HAND_SIZE.0, HAND_SIZE.1).to_image();
        Some(Art { wallpaper: premultiplied(&wallpaper), dialog: premultiplied(&dialog), hand: premultiplied(&hand) })
    }
}

/// what the controllers do for the bottom screen this frame.
#[derive(Default)]
struct PadReading {
    /// the show/hide button is held.
    toggle: bool,
    /// the touch cursor's tap button is held.
    tap: bool,
    /// the right stick, -1..1, up positive.
    stick: (f32, f32),
}

/// reads the input API: the BottomScreen action (L3, [keys]/[pad]
/// bottom_screen, M on the keyboard), TouchClick (R3) and the right stick,
/// which the game no longer gets.
fn read_pad() -> PadReading {
    use crate::controls::PcAction;
    let [x, y] = crate::input_api::right_stick();
    // cameras: while the battle free camera or My Castle's view has the
    // right stick, it is not the touch pointer's, nor is R3
    if crate::cameras::owns_right_stick() {
        return PadReading { toggle: crate::input_api::action_held(PcAction::BottomScreen), tap: false, stick: (0.0, 0.0) };
    }
    PadReading {
        toggle: crate::input_api::action_held(PcAction::BottomScreen),
        tap: crate::input_api::action_held(PcAction::TouchClick),
        stick: (x, y),
    }
}

/// the pointing hand in SystemWindow2.png (the ui kit's Art::Hand), and its
/// fingertip within it. These layers are drawn by the presenter rather than
/// through the kit's egui painter: the wallpaper has to go under the
/// screens, and the frame and cursor show while the interface is not drawn.
const HAND_AT: (u32, u32) = (132, 62);
const HAND_SIZE: (u32, u32) = (24, 14);
const FINGERTIP: (f32, f32) = (23.5, 2.5);

/// textured quads for a layer.
struct Layer {
    meshes: Vec<OverlayMesh>,
    clip: [u32; 4],
}

impl Layer {
    fn new(window: (u32, u32)) -> Layer {
        Layer { meshes: Vec::new(), clip: [0, 0, window.0, window.1] }
    }

    /// a rectangle of the window, x0 y0 x1 y1, showing a part of a texture,
    /// u0 v0 u1 v1 in its pixels.
    fn quad(&mut self, texture: u64, size: [u32; 2], at: [f32; 4], from: [f32; 4], color: [u8; 4]) {
        if at[2] <= at[0] || at[3] <= at[1] {
            return;
        }
        if self.meshes.last().is_none_or(|mesh| mesh.texture != texture) {
            self.meshes.push(OverlayMesh { texture, clip: self.clip, vertices: Vec::new(), indices: Vec::new() });
        }
        let mesh = self.meshes.last_mut().expect("pushed above");
        let (w, h) = (size[0] as f32, size[1] as f32);
        let first = mesh.vertices.len() as u32;
        for (x, y, u, v) in [(at[0], at[1], from[0], from[1]), (at[2], at[1], from[2], from[1]), (at[2], at[3], from[2], from[3]), (at[0], at[3], from[0], from[3])] {
            mesh.vertices.push(OverlayVertex { position: [x, y], uv: [u / w, v / h], color });
        }
        mesh.indices.extend_from_slice(&[first, first + 1, first + 2, first, first + 2, first + 3]);
    }

    fn overlay(self) -> Overlay {
        Overlay { textures: Vec::new(), meshes: self.meshes, free: Vec::new() }
    }
}

fn shade(amount: f32, alpha: f32) -> [u8; 4] {
    let a = (alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    let c = (amount * alpha).clamp(0.0, 1.0) * 255.0;
    [c as u8, c as u8, c as u8, a]
}

/// the controller's touch cursor, in bottom-screen pixels.
struct Cursor {
    x: f32,
    y: f32,
    /// when the stick or the tap button last did something.
    used: Option<Instant>,
    tapping: bool,
    /// the stick leans past the deadzone this frame.
    moving: bool,
}

/// the bottom screen's presentation state, one per window.
pub struct BottomScreen {
    art: Option<Art>,
    art_tried: bool,
    /// the presenter has the textures, cleared when it is made anew.
    textures_sent: bool,
    /// shown, by its button. on-demand starts hidden, the others shown.
    shown: Option<bool>,
    /// 0 put away to 1 shown, eased toward shown.
    reveal: f32,
    /// the button is down, since when.
    held_since: Option<Instant>,
    /// l3toggle: the polls since it went down.
    held_polls: u32,
    /// the button's state when it went down, to undo a hold.
    shown_before_hold: bool,
    /// the BottomScreen action (keyboard or controller) was held.
    pad_toggle_down: bool,
    cursor: Cursor,
    /// a checking script holds the cursor's tap down.
    check_tap: bool,
    last_tick: Option<Instant>,
    /// where the bottom screen was last composed, none in classic mode.
    composed: Option<PanelPlaces>,
    window: (u32, u32),
    /// the mode the shown state belongs to; another starts from its default.
    mode: Option<Mode>,
    /// controllers are left out, while a checking script drives it (other
    /// programs' virtual controllers come and go on a shared machine).
    ignore_pads: bool,
    /// the mouse rests at the panel's edge of the window, or over the
    /// panel it brought out, which peeks a put-away panel.
    hover: bool,
    // bottom2: Auto, the mouse's dragging, the options' preview
    /// what the game shows on its lower screen, read each game frame.
    reading: Reading,
    /// the game frame of the last reading, for the log.
    game_frame: u64,
    auto: AutoReveal,
    /// the floating window's frame where it is shown, and which way it
    /// slides away, for the edge peek.
    shown_frame: Option<Viewport>,
    /// the frame grabbed by the mouse, and where it is going.
    drag: Option<Drag>,
    /// what the mouse is over: none, the frame (to move it) or a corner (to
    /// size it), for the frame's glow and the pointer's shape.
    pub over: Zone,
    /// the options menu is on the Bottom Screen tab: shown, to see changes.
    pub preview: bool,
    /// the button's manual state and Auto's dismissal before a hold.
    before_hold: (Option<bool>, Option<Reason>),
    /// the settings' placement at the last compose.
    settings_place: Option<Placement>,
    /// reads the game for Auto.
    probe: crate::bottom_scene::Probe,
    /// the pointer shape last asked of the window.
    icon: Option<winit::window::CursorIcon>,
}

/// bottom2: what the mouse is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Zone {
    #[default]
    None,
    /// the frame's border: drag to move.
    Frame,
    /// a corner, 0 top left, 1 top right, 2 bottom left, 3 bottom right:
    /// drag to resize.
    Corner(u8),
}

/// bottom2: a frame held by the mouse.
#[derive(Debug, Clone, Copy)]
struct Drag {
    zone: Zone,
    /// where the mouse went down and the frame then.
    from: (f32, f32),
    frame: Viewport,
    screen_h: f32,
    place: Placement,
}

/// bottom2: the floating window's placement from the settings.
pub fn placement(pc: &PcSettings) -> Placement {
    let anchor = match pc.bottom_anchor {
        BottomAnchor::TopLeft => Anchor::TopLeft,
        BottomAnchor::Top => Anchor::Top,
        BottomAnchor::TopRight => Anchor::TopRight,
        BottomAnchor::Left => Anchor::Left,
        BottomAnchor::Right => Anchor::Right,
        BottomAnchor::BottomLeft => Anchor::BottomLeft,
        BottomAnchor::Bottom => Anchor::Bottom,
        BottomAnchor::BottomRight => Anchor::BottomRight,
    };
    let finite = |v: f32, or: f32| if v.is_finite() { v } else { or };
    Placement { anchor, dx: finite(pc.bottom_x, 0.02), dy: finite(pc.bottom_y, 0.02), height: finite(pc.bottom_height, 0.33) }
}

/// bottom2: a placement into the settings.
pub fn store_placement(pc: &mut PcSettings, place: Placement) {
    pc.bottom_anchor = match place.anchor {
        Anchor::TopLeft => BottomAnchor::TopLeft,
        Anchor::Top => BottomAnchor::Top,
        Anchor::TopRight => BottomAnchor::TopRight,
        Anchor::Left => BottomAnchor::Left,
        Anchor::Right => BottomAnchor::Right,
        Anchor::BottomLeft => BottomAnchor::BottomLeft,
        Anchor::Bottom => BottomAnchor::Bottom,
        Anchor::BottomRight => BottomAnchor::BottomRight,
    };
    // a thousandth of the window is finer than a pixel and keeps the file tidy
    let tidy = |v: f32| (v * 1000.0).round() / 1000.0;
    pc.bottom_x = tidy(place.dx);
    pc.bottom_y = tidy(place.dy);
    pc.bottom_height = tidy(place.height);
}

impl Default for BottomScreen {
    fn default() -> Self {
        Self::new()
    }
}

impl BottomScreen {
    pub fn new() -> BottomScreen {
        BottomScreen {
            art: None,
            art_tried: false,
            textures_sent: false,
            shown: None,
            reveal: 1.0,
            held_since: None,
            held_polls: 0,
            shown_before_hold: false,
            pad_toggle_down: false,
            check_tap: false,
            cursor: Cursor { x: 160.0, y: 120.0, used: None, tapping: false, moving: false },
            last_tick: None,
            composed: None,
            window: (0, 0),
            mode: None,
            ignore_pads: std::env::var_os("FATES_BOTTOM_SCRIPT").is_some(),
            hover: false,
            reading: Reading::default(),
            game_frame: 0,
            auto: AutoReveal::default(),
            shown_frame: None,
            drag: None,
            over: Zone::None,
            preview: false,
            before_hold: (None, None),
            settings_place: None,
            probe: crate::bottom_scene::Probe::default(),
            icon: None,
        }
    }

    /// the presenter was made anew and has none of the textures.
    pub fn presenter_changed(&mut self) {
        self.textures_sent = false;
    }

    /// the panel mode in force, none for the console's arrangements, which
    /// is also what a presenter that cannot compose, a turned screen or 3D
    /// shown side by side get.
    pub fn mode(settings: &Settings, composes: bool) -> Option<PanelMode> {
        if !composes || settings.orientation != Orientation::North {
            return None;
        }
        if matches!(settings.stereo().mode, StereoMode::SideBySide | StereoMode::HalfSideBySide) {
            return None;
        }
        match settings.pc.bottom_screen {
            Mode::Panel => Some(PanelMode::Panel),
            Mode::Overlay => Some(PanelMode::Overlay),
            // bottom2: Auto slides in as Manual does, by the game too
            Mode::Auto | Mode::OnDemand => Some(PanelMode::OnDemand),
            Mode::Classic => None,
        }
    }

    /// shown by the button, or by the mode's default: put away for Auto and
    /// Manual, shown for the others.
    fn is_shown(&self, mode: PanelMode) -> bool {
        self.shown.unwrap_or(mode != PanelMode::OnDemand)
    }

    /// bottom2: shown by the button or by Auto.
    fn wanted(&self, mode: PanelMode) -> bool {
        self.is_shown(mode) || (self.mode == Some(Mode::Auto) && self.auto.shown.is_some())
    }

    /// a change of mode forgets the last one's shown or hidden.
    fn follow_mode(&mut self, settings: &Settings, composes: bool) {
        let mode = Self::mode(settings, composes).map(|_| settings.pc.bottom_screen);
        if mode != self.mode {
            self.mode = mode;
            self.shown = None;
            self.held_since = None;
            self.auto = AutoReveal::default();
            self.drag = None;
        }
    }

    /// bottom2: what the game shows on its lower screen, each game frame.
    pub fn game_state(&mut self, reading: Reading, frame: u64) {
        self.reading = reading;
        self.game_frame = frame;
    }

    /// bottom2: reads what Fates shows on its lower screen, after each game
    /// frame, for Auto.
    pub fn read_game(&mut self, memory: &mut zakuro_core::memory::Memory, frame: u64) {
        let reading = self.probe.read(memory);
        self.game_state(reading, frame);
    }

    /// bottom2: the pointer's shape over the frame, when it changed: the
    /// move cross over the border, the diagonal over a corner.
    pub fn icon_change(&mut self) -> Option<winit::window::CursorIcon> {
        use winit::window::CursorIcon;
        let zone = self.drag.map_or(self.over, |drag| drag.zone);
        let icon = match zone {
            Zone::None => CursorIcon::Default,
            Zone::Frame if self.drag.is_some() => CursorIcon::Grabbing,
            Zone::Frame => CursorIcon::Move,
            Zone::Corner(0 | 3) => CursorIcon::NwseResize,
            Zone::Corner(_) => CursorIcon::NeswResize,
        };
        (self.icon != Some(icon) && (self.icon.is_some() || icon != CursorIcon::Default)).then(|| {
            self.icon = Some(icon);
            icon
        })
    }

    /// bottom2: the floating window's frame while the options preview it.
    pub fn preview_frame(&self) -> Option<Viewport> {
        let floating = matches!(self.mode, Some(Mode::Auto | Mode::OnDemand | Mode::Overlay));
        self.composed.filter(|_| self.preview && floating).map(|places| places.frame)
    }

    /// bottom2: Auto has it out, and why, for the log and tests.
    #[allow(dead_code)]
    pub fn auto_reason(&self) -> Option<Reason> {
        self.auto.shown
    }

    /// the show/hide button, the keyboard's or the controller's: each press
    /// toggles, on the press; letting go does nothing, except in On Demand,
    /// where a hold shows it while held.
    pub fn button(&mut self, pressed: bool, settings: &Settings, composes: bool) {
        let Some(mode) = Self::mode(settings, composes) else { return };
        self.follow_mode(settings, composes);
        let shown = self.wanted(mode);
        log::debug!("bottom screen button {}, {mode:?} was {}", if pressed { "down" } else { "up" }, if shown { "shown" } else { "hidden" });
        match (pressed, self.held_since) {
            (true, None) => {
                self.held_since = Some(Instant::now());
                self.held_polls = 0;
                self.shown_before_hold = shown;
                self.before_hold = (self.shown, self.auto.dismissed);
                self.shown = Some(!shown);
                // bottom2: put away while Auto has it out, it stays away
                // until what brought it out has gone
                if shown && self.auto.shown.is_some() {
                    self.auto.dismiss();
                    log::info!("bottom2: auto put away by hand at game frame {}", self.game_frame);
                }
            }
            (false, Some(since)) => {
                self.held_since = None;
                // l3toggle: On Demand held, back to how it was; anything
                // else, it stays toggled
                let held = since.elapsed() >= HOLD && self.held_polls >= HOLD_POLLS;
                if self.mode == Some(Mode::OnDemand) && held {
                    self.shown = self.before_hold.0;
                    self.auto.dismissed = self.before_hold.1;
                }
            }
            _ => {}
        }
    }

    /// reads the controller each frame: the show/hide button, the touch
    /// cursor's stick and its tap button.
    pub fn poll(&mut self, settings: &Settings, composes: bool) {
        let pad = if self.ignore_pads { PadReading::default() } else { read_pad() };
        let toggle = pad.toggle;
        if toggle != self.pad_toggle_down {
            log::debug!("bottom screen: its button {}", if toggle { "down" } else { "up" });
            self.pad_toggle_down = toggle;
            self.button(toggle, settings, composes);
        }
        if self.held_since.is_some() {
            self.held_polls = self.held_polls.saturating_add(1); // l3toggle
        }
        let now = Instant::now();
        // qol4: the first poll puts the panel where it is wanted at once
        let first = self.last_tick.is_none();
        let dt = self.last_tick.map_or(0.0, |last| now.duration_since(last).as_secs_f32()).min(0.1);
        let dt = if crate::trailer_dump::lockstep() && !first { 1.0 / 60.0 } else { dt }; // trailer: one game frame per poll
        self.last_tick = Some(now);

        // the touch cursor: off with a speed of 0
        let speed = settings.pc.touch_cursor_speed;
        let (x, y) = pad.stick;
        let lean = x.hypot(y);
        self.cursor.moving = speed > 0.0 && lean > STICK_DEADZONE;
        if self.cursor.moving {
            // gentle near the middle for aiming, full speed at the edge
            let strength = ((lean - STICK_DEADZONE) / (1.0 - STICK_DEADZONE)).clamp(0.0, 1.0).powf(1.6);
            let step = speed * strength * dt / lean;
            self.cursor.x = (self.cursor.x + x * step).clamp(0.0, 319.0);
            // up on the stick is up the screen
            self.cursor.y = (self.cursor.y - y * step).clamp(0.0, 239.0);
            self.cursor.used = Some(now);
        }
        self.cursor.tapping = speed > 0.0 && (pad.tap || self.check_tap);
        if self.cursor.tapping {
            self.cursor.used = Some(now);
        }

        // the panel slides toward where it is wanted; the cursor in use
        // brings a put-away panel out while it is
        self.follow_mode(settings, composes);
        // bottom2: Auto follows the game's lower screen
        if self.mode == Some(Mode::Auto) {
            let linger = if settings.pc.bottom_linger.is_finite() { settings.pc.bottom_linger.clamp(0.0, 10.0) } else { 1.0 };
            // Haiku: only the moments the player chose ("Show For") bring it out
            let mut reading = self.reading;
            if reading.want.is_some_and(|reason| !reason.allowed(&settings.pc)) {
                reading.want = None;
            }
            let delay = settings.pc.bottom_show_delay;
            self.auto.unit_settle = Some(if delay.is_finite() { delay.clamp(0.0, 3.0) } else { 0.3 });
            match self.auto.step(reading, dt, linger) {
                Some(Change::Revealed(reason)) => log::info!("bottom2: auto reveal for {} at game frame {}", reason.name(), self.game_frame),
                Some(Change::Hidden { after, quiet }) => {
                    log::info!("bottom2: auto hide after {}{} at game frame {}", after.name(), if quiet { ", the lower screen now decorative" } else { "" }, self.game_frame)
                }
                None => {}
            }
        }
        let target = match Self::mode(settings, composes) {
            Some(mode) => (self.wanted(mode) || self.cursor_alpha() > 0.0 || self.hover || self.drag.is_some() || self.preview) as u8 as f32,
            None => 1.0,
        };
        let step = if SLIDE > 0.0 { dt / SLIDE } else { 1.0 };
        self.reveal = if target > self.reveal { (self.reveal + step).min(target) } else { (self.reveal - step).max(target) };
        // qol4: rather than sliding away from shown over the game's first
        // frames, a put-away panel (Auto, on demand) starts put away: at
        // launch it was seen, an empty frame, sliding off the screen
        if first {
            self.reveal = target;
        }
    }

    /// puts the touch cursor somewhere, for a checking script.
    pub fn check_cursor(&mut self, x: f32, y: f32, tap: bool) {
        self.cursor.x = x.clamp(0.0, 319.0);
        self.cursor.y = y.clamp(0.0, 239.0);
        self.cursor.used = Some(Instant::now());
        self.check_tap = tap;
    }

    /// how visible the touch cursor is, 0 to 1.
    fn cursor_alpha(&self) -> f32 {
        let Some(used) = self.cursor.used else { return 0.0 };
        let idle = used.elapsed().as_secs_f32();
        if self.cursor.tapping || idle <= CURSOR_STAY {
            1.0
        } else {
            (1.0 - (idle - CURSOR_STAY) / CURSOR_FADE).max(0.0)
        }
    }

    /// the controller's touch, and its stick kept from the C-stick while
    /// it moves the cursor. the mouse's touch comes first.
    pub fn apply_input(&self, state: &mut InputState) {
        if self.cursor.moving {
            state.c_stick_x = 0.0;
            state.c_stick_y = 0.0;
        }
        let visible = self.composed.is_none_or(|places| {
            let (x, y) = panel::window_point(places.bottom, self.cursor.x, self.cursor.y);
            panel::touch_point(places.bottom, self.window, x, y).is_some()
        });
        if state.touch.is_none() && self.cursor.tapping && visible && self.reveal > 0.99 {
            state.touch = Some((self.cursor.x as u16, self.cursor.y as u16));
        }
    }

    /// where the mouse is, in window pixels: resting at the window's edge on
    /// the panel's side brings a put-away panel out, and it stays while the
    /// mouse is over it.
    pub fn mouse_at(&mut self, x: f32, y: f32, settings: &Settings) {
        let Some(places) = self.composed else {
            self.hover = false;
            self.over = Zone::None;
            return;
        };
        let (w, h) = (self.window.0 as f32, self.window.1 as f32);
        let edge = (w.min(h * 16.0 / 9.0) * 0.012).max(6.0);
        // bottom2: the edge it slides out of, the side's for the docked
        // panel, the nearest one for a floating window
        let shown = self.shown_frame.unwrap_or(places.frame);
        let along_y = y >= shown.y && y < shown.y + shown.height;
        let along_x = x >= shown.x && x < shown.x + shown.width;
        let at_edge = match self.mode {
            Some(Mode::Panel) | None => {
                along_y
                    && match settings.pc.bottom_side {
                        Side::Right => x >= w - edge,
                        Side::Left => x <= edge,
                    }
            }
            Some(_) => match panel::slide_out((w, h), shown) {
                (dx, _) if dx > 0.0 => along_y && x >= w - edge,
                (dx, _) if dx < 0.0 => along_y && x <= edge,
                (_, dy) if dy > 0.0 => along_x && y >= h - edge,
                _ => along_x && y <= edge,
            },
        };
        let f = places.frame;
        let over = self.reveal > 0.0 && x >= f.x && x < f.x + f.width && y >= f.y && y < f.y + f.height;
        self.hover = at_edge || (self.hover && over);
        self.over = if self.drag.is_some() { self.over } else { self.zone_at(x, y) };
    }

    /// bottom2: the part of a floating window's frame at (x, y): its corners
    /// size it, the rest of its border moves it; the screen inside is the
    /// game's to touch.
    fn zone_at(&self, x: f32, y: f32) -> Zone {
        let floating = matches!(self.mode, Some(Mode::Auto | Mode::OnDemand | Mode::Overlay));
        let Some(places) = self.composed.filter(|_| floating && self.reveal > 0.99) else { return Zone::None };
        let (f, b) = (places.frame, places.bottom);
        let inside = |v: Viewport| x >= v.x && x < v.x + v.width && y >= v.y && y < v.y + v.height;
        if !inside(f) || inside(b) {
            return Zone::None;
        }
        // the corners' checker, two borders deep
        let reach = (panel::FRAME_INSET * places.art_scale * 2.0).min(f.width / 3.0);
        let right = x >= f.x + f.width - reach;
        let left = x < f.x + reach;
        let lower = y >= f.y + f.height - reach;
        let upper = y < f.y + reach;
        match (left, right, upper, lower) {
            (true, _, true, _) => Zone::Corner(0),
            (_, true, true, _) => Zone::Corner(1),
            (true, _, _, true) => Zone::Corner(2),
            (_, true, _, true) => Zone::Corner(3),
            _ => Zone::Frame,
        }
    }

    /// bottom2: the mouse's button went down at (x, y): on a floating
    /// window's frame it takes hold of it (and no touch goes to the game).
    pub fn press(&mut self, x: f32, y: f32) -> bool {
        let zone = self.zone_at(x, y);
        let Some(places) = self.composed.filter(|_| zone != Zone::None) else { return false };
        let Some(place) = self.placement_now() else { return false };
        self.drag = Some(Drag { zone, from: (x, y), frame: places.frame, screen_h: places.bottom.height, place });
        self.over = zone;
        log::debug!("bottom2: took the frame by {zone:?}");
        true
    }

    /// bottom2: the frame held follows the mouse.
    pub fn drag_to(&mut self, x: f32, y: f32) -> bool {
        let window = self.window;
        let Some(drag) = &mut self.drag else { return false };
        drag.place = match drag.zone {
            Zone::Corner(corner) => panel::resize(window, drag.frame, drag.screen_h, corner, x, y),
            _ => {
                let (nx, ny) = panel::snap(window, drag.frame, drag.frame.x + x - drag.from.0, drag.frame.y + y - drag.from.1);
                panel::placement_of(window, Viewport { x: nx, y: ny, ..drag.frame }, drag.screen_h)
            }
        };
        true
    }

    /// bottom2: let go of the frame: where it now is, for the settings.
    pub fn release(&mut self) -> Option<Placement> {
        let drag = self.drag.take()?;
        log::info!("bottom2: placed {:?} at {:.3}, {:.3}, {:.3} high", drag.place.anchor, drag.place.dx, drag.place.dy, drag.place.height);
        Some(drag.place)
    }

    /// bottom2: the placement in force: the one being dragged, else the
    /// settings' (kept from the last compose).
    fn placement_now(&self) -> Option<Placement> {
        self.drag.map(|drag| drag.place).or(self.settings_place)
    }

    /// the bottom screen's place when this composes the window, for the
    /// mouse's touches; none when the console arrangement is used.
    pub fn composed_bottom(&self) -> Option<Viewport> {
        self.composed.map(|places| places.bottom)
    }

    /// the window's composition for this frame, none to leave it to the
    /// presenter's arrangement, as without a game. classic is the bottom
    /// viewport of the console arrangement, for the cursor.
    pub fn compose(&mut self, window: (u32, u32), settings: &Settings, composes: bool, top_image: (u32, u32), classic: Option<Viewport>) -> Option<Composition> {
        self.window = window;
        if !composes {
            self.composed = None;
            return None;
        }
        if !self.art_tried {
            self.art_tried = true;
            self.art = Art::load();
            if self.art.is_none() {
                log::warn!("the bottom screen's frame art was not found under assets/ui, drawing it without");
            }
        }
        let mode = Self::mode(settings, composes);
        let mut under = Layer::new(window);
        let mut over = Layer::new(window);
        let mut composition = Composition { screens: None, bottom_opacity: 1.0, ..Composition::default() };

        if let Some(art) = &self.art {
            if !self.textures_sent {
                composition.under.textures = vec![art.wallpaper.texture(WALLPAPER, true), art.dialog.texture(DIALOG, false), art.hand.texture(HAND, false)];
                self.textures_sent = true;
            }
        }
        // wide-ui: the wallpaper only backs the layouts that arrange the
        // screens side by side (Panel, Classic). Where the top screen fills
        // the window (Auto, Always Shown, Manual), whatever it leaves at its
        // edges (a fixed aspect narrower than the window) stays black: thin
        // brown strips there read as a fault, not as a frame
        let backed = matches!(mode, None | Some(PanelMode::Panel));
        if let (Some(art), true) = (&self.art, backed) {
            // the wallpaper fills the window, cut to its shape
            let [tw, th] = art.wallpaper.size;
            let (w, h) = (window.0 as f32, window.1 as f32);
            let scale = (w / tw as f32).max(h / th as f32);
            let (cw, ch) = (w / scale, h / scale);
            let (u0, v0) = ((tw as f32 - cw) / 2.0, (th as f32 - ch) / 2.0);
            under.quad(WALLPAPER, art.wallpaper.size, [0.0, 0.0, w, h], [u0, v0, u0 + cw, v0 + ch], shade(WALLPAPER_SHADE, 1.0));
        }

        let bottom_place = match mode {
            Some(mode) => {
                let params = PanelParams {
                    mode,
                    side: match settings.pc.bottom_side {
                        Side::Left => PanelSide::Left,
                        Side::Right => PanelSide::Right,
                    },
                    size: if settings.pc.bottom_size.is_finite() { settings.pc.bottom_size } else { 0.4 },
                    top_aspect: top_aspect(top_image, settings.top_shape.width()),
                    integer: settings.integer_scale,
                    place: self.drag.map(|drag| drag.place).unwrap_or_else(|| placement(&settings.pc)),
                };
                self.settings_place = Some(placement(&settings.pc));
                let places = panel::revealed_layout(window, params, self.reveal);
                self.shown_frame = Some(panel::panel_layout(window, params).frame);
                // bottom2: the floating window at the opacity asked for
                let opacity = if mode == PanelMode::Panel { 1.0 } else { settings.pc.bottom_opacity.clamp(0.1, 1.0) };
                composition.bottom_opacity = opacity;
                let bottom_seen = panel::touch_point(places.bottom, window, places.bottom.x.max(0.0) + 1.0, places.bottom.y + 1.0).is_some()
                    || panel::touch_point(places.bottom, window, (places.bottom.x + places.bottom.width).min(window.0 as f32) - 1.0, places.bottom.y + 1.0).is_some();
                composition.screens = Some((Some(places.top), bottom_seen.then_some(places.bottom)));
                if let Some(art) = &self.art {
                    frame(&mut over, &art.dialog, places.frame, places.art_scale, opacity);
                    // bottom2: the frame warms under the mouse, and the
                    // corner held or pointed at more, as the game's
                    // buttons light up
                    let lit = if self.drag.is_some() { self.drag.map(|d| d.zone) } else { Some(self.over) };
                    match lit {
                        Some(Zone::Frame) => glow(&mut over, &art.dialog, places.frame, places.art_scale, None),
                        Some(Zone::Corner(corner)) => glow(&mut over, &art.dialog, places.frame, places.art_scale, Some(corner)),
                        _ => {}
                    }
                }
                self.composed = Some(places);
                Some((places.bottom, places.art_scale))
            }
            None => {
                self.composed = None;
                self.shown_frame = None;
                classic.map(|bottom| (bottom, (bottom.height / 240.0).round().max(1.0)))
            }
        };

        // the touch cursor over the bottom screen
        let alpha = self.cursor_alpha();
        if let (Some(art), Some((bottom, scale)), true) = (&self.art, bottom_place, alpha > 0.0) {
            let (x, y) = panel::window_point(bottom, self.cursor.x, self.cursor.y);
            // the hand presses in a little while tapping
            let (dx, dy) = if self.cursor.tapping { (scale * 1.5, scale * 1.0) } else { (0.0, 0.0) };
            let left = x - FINGERTIP.0 * scale + dx;
            let top = y - FINGERTIP.1 * scale + dy;
            let [hw, hh] = art.hand.size;
            over.quad(HAND, art.hand.size, [left, top, left + hw as f32 * scale, top + hh as f32 * scale], [0.0, 0.0, hw as f32, hh as f32], shade(1.0, alpha));
        }

        let textures = std::mem::take(&mut composition.under.textures);
        composition.under = under.overlay();
        composition.under.textures = textures;
        composition.over = over.overlay();
        Some(composition)
    }
}

/// the frame around the bottom screen: Dialog.png's border, its outer twelve
/// art pixels with the checker corners, never over the screen itself.
fn frame(layer: &mut Layer, dialog: &Picture, at: Viewport, scale: f32, alpha: f32) {
    let size = dialog.size;
    let color = shade(1.0, alpha);
    let b = panel::FRAME_INSET;
    let (x0, y0, x1, y1) = (at.x, at.y, at.x + at.width, at.y + at.height);
    // art: 64x64, the border 12 deep, its corners' checker reaching 24 in,
    // the middle 24..40 plain enough to stretch
    let (c, s) = (24.0, scale);
    // top and bottom strips: corner, stretched middle, corner
    for (v0, y) in [(0.0, y0), (64.0 - b, y1 - b * s)] {
        layer.quad(DIALOG, size, [x0, y, x0 + c * s, y + b * s], [0.0, v0, c, v0 + b], color);
        layer.quad(DIALOG, size, [x0 + c * s, y, x1 - c * s, y + b * s], [c, v0, 64.0 - c, v0 + b], color);
        layer.quad(DIALOG, size, [x1 - c * s, y, x1, y + b * s], [64.0 - c, v0, 64.0, v0 + b], color);
    }
    // left and right strips between them
    for (u0, x) in [(0.0, x0), (64.0 - b, x1 - b * s)] {
        layer.quad(DIALOG, size, [x, y0 + b * s, x + b * s, y0 + c * s], [u0, b, u0 + b, c], color);
        layer.quad(DIALOG, size, [x, y0 + c * s, x + b * s, y1 - c * s], [u0, c, u0 + b, 64.0 - c], color);
        layer.quad(DIALOG, size, [x, y1 - c * s, x + b * s, y1 - b * s], [u0, 64.0 - c, u0 + b, 64.0 - b], color);
    }
}

/// bottom2: the frame lit, added over it, the whole border or one corner
/// brighter: the warm light of the game's chosen buttons.
fn glow(layer: &mut Layer, dialog: &Picture, at: Viewport, scale: f32, corner: Option<u8>) {
    // alpha 0 adds the colour (the overlay blends premultiplied)
    let soft = [70, 52, 22, 0];
    let strong = [150, 112, 48, 0];
    let mut lit = Layer { meshes: Vec::new(), clip: layer.clip };
    let size = dialog.size;
    let s = scale;
    let (c, b) = (24.0, panel::FRAME_INSET);
    let (x0, y0, x1, y1) = (at.x, at.y, at.x + at.width, at.y + at.height);
    match corner {
        None => {
            frame(&mut lit, dialog, at, scale, 1.0);
            for mesh in &mut lit.meshes {
                for vertex in &mut mesh.vertices {
                    vertex.color = soft;
                }
            }
        }
        Some(corner) => {
            frame(&mut lit, dialog, at, scale, 1.0);
            for mesh in &mut lit.meshes {
                for vertex in &mut mesh.vertices {
                    vertex.color = soft;
                }
            }
            // the corner's checker piece, brighter
            let (u0, x) = if corner & 1 == 1 { (64.0 - c, x1 - c * s) } else { (0.0, x0) };
            let (v0, y) = if corner & 2 == 2 { (64.0 - c, y1 - c * s) } else { (0.0, y0) };
            let _ = b;
            lit.quad(DIALOG, size, [x, y, x + c * s, y + c * s], [u0, v0, u0 + c, v0 + c], strong);
        }
    }
    layer.meshes.extend(lit.meshes);
}

/// the top screen's shape: the picture's own when the game draws one wider
/// than the console's, else the shape the settings stretch it to.
pub fn top_aspect(image: (u32, u32), shape_width: f32) -> f32 {
    let (w, h) = image;
    if w > 0 && h > 0 {
        let own = w as f32 / h as f32;
        if (own - 5.0 / 3.0).abs() > 0.02 {
            return own;
        }
    }
    shape_width / 240.0
}

/// a window that fits the screens at scale times the console's size.
pub fn window_size(mode: PanelMode, settings: &Settings, scale: f32) -> (u32, u32) {
    let height = 240.0 * scale;
    let width = height * settings.top_shape.width() / 240.0;
    match mode {
        PanelMode::Panel => {
            let size = settings.pc.bottom_size.clamp(0.3, 1.0);
            let margin = (height * 0.02).round().max(4.0);
            let panel = size * height * (320.0 + 2.0 * panel::FRAME_INSET) / 240.0;
            ((width + panel + 2.0 * margin + 8.0).ceil() as u32, height as u32)
        }
        _ => (width.ceil() as u32, height as u32),
    }
}

/// F9's next arrangement (bottom2): Auto, Always shown, Panel, Manual,
/// then the console's arrangements, then round to Auto.
pub fn next_arrangement(mode: Mode, screens: Screens) -> (Mode, Screens) {
    match mode {
        Mode::Auto => (Mode::Overlay, screens),
        Mode::Overlay => (Mode::Panel, screens),
        Mode::Panel => (Mode::OnDemand, screens),
        Mode::OnDemand => (Mode::Classic, Screens::Stacked),
        Mode::Classic if screens == *Screens::ALL.last().expect("layouts") => (Mode::Auto, screens),
        Mode::Classic => (Mode::Classic, screens.next()),
    }
}

/// bottom2: the mode's name as the options menu shows it.
pub fn mode_name(mode: Mode) -> &'static str {
    match mode {
        Mode::Auto => "Auto",
        Mode::Overlay => "Always Shown",
        Mode::Panel => "Panel",
        Mode::OnDemand => "Manual",
        Mode::Classic => "Classic",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f9_goes_round_every_arrangement() {
        let mut at = (Mode::Auto, Screens::Stacked);
        let mut seen = Vec::new();
        for _ in 0..9 {
            at = next_arrangement(at.0, at.1);
            seen.push(at);
        }
        assert_eq!(seen[0].0, Mode::Overlay);
        assert_eq!(seen[1].0, Mode::Panel);
        assert_eq!(seen[2].0, Mode::OnDemand);
        assert_eq!(seen[3], (Mode::Classic, Screens::Stacked));
        assert_eq!(seen[7], (Mode::Classic, Screens::BottomOnly));
        assert_eq!(seen[8].0, Mode::Auto);
    }

    #[test]
    fn the_top_screens_shape_follows_its_picture() {
        assert!((top_aspect((400, 240), 400.0) - 5.0 / 3.0).abs() < 1e-4);
        assert!((top_aspect((1200, 720), 560.0) - 560.0 / 240.0).abs() < 1e-4);
        // a wider picture, from the widescreen work, is used as it is
        assert!((top_aspect((1280, 720), 400.0) - 16.0 / 9.0).abs() < 1e-4);
        assert!((top_aspect((0, 0), 400.0) - 5.0 / 3.0).abs() < 1e-4);
    }

    /// bottom2: Auto comes out for what the game shows, goes after, and a
    /// tap of the button while it is out puts it away until that ends.
    #[test]
    fn auto_follows_the_game() {
        let settings = Settings::default();
        assert_eq!(settings.pc.bottom_screen, Mode::Auto);
        let mut bottom = BottomScreen::new();
        bottom.ignore_pads = true;
        bottom.poll(&settings, true);
        assert!(!bottom.wanted(PanelMode::OnDemand));
        bottom.game_state(Reading { want: Some(Reason::Tutorial), quiet: false }, 1);
        bottom.poll(&settings, true);
        assert_eq!(bottom.auto_reason(), Some(Reason::Tutorial));
        assert!(bottom.wanted(PanelMode::OnDemand));
        bottom.button(true, &settings, true);
        bottom.button(false, &settings, true);
        assert!(!bottom.wanted(PanelMode::OnDemand), "tapped away");
        bottom.poll(&settings, true);
        assert!(!bottom.wanted(PanelMode::OnDemand), "stays away for the same tutorial");
        // a movie's lower screen is never brought out
        bottom.game_state(Reading { want: None, quiet: true }, 2);
        bottom.poll(&settings, true);
        assert!(!bottom.wanted(PanelMode::OnDemand));
        bottom.game_state(Reading { want: Some(Reason::Target), quiet: false }, 3);
        bottom.poll(&settings, true);
        assert!(bottom.wanted(PanelMode::OnDemand), "the next reason brings it out");
        // Manual ignores the game
        let mut manual = settings.clone();
        manual.pc.bottom_screen = Mode::OnDemand;
        let mut bottom = BottomScreen::new();
        bottom.game_state(Reading { want: Some(Reason::Tutorial), quiet: false }, 1);
        bottom.poll(&manual, true);
        assert!(!bottom.wanted(PanelMode::OnDemand));
    }

    /// bottom2: the frame is taken by the mouse without touching the game,
    /// moved and snapped, its corner sizes it, and the screen inside still
    /// touches.
    #[test]
    fn the_frame_drags_and_the_screen_touches() {
        let mut settings = Settings::default();
        settings.pc.bottom_screen = Mode::Overlay;
        let mut bottom = BottomScreen::new();
        bottom.ignore_pads = true; // l3toggle: the input API is shared with other tests
        bottom.art_tried = true;
        let window = (1920, 1080);
        bottom.poll(&settings, true);
        bottom.compose(window, &settings, true, (400, 240), None).expect("composes");
        let places = bottom.composed.expect("placed");
        // the right edge's middle, a third of the window high
        assert!((places.frame.x + places.frame.width - (1920.0 - 0.02 * 1080.0)).abs() <= 1.0, "{places:?}");
        assert!((places.bottom.height - (0.33f32 * 1080.0).floor()).abs() <= 1.0);
        // the screen inside is not the frame
        let (cx, cy) = panel::window_point(places.bottom, 160.0, 120.0);
        assert!(!bottom.press(cx, cy));
        // the top edge of the frame, in from the corners, moves it
        let (fx, fy) = (places.frame.x + places.frame.width / 2.0, places.frame.y + 2.0);
        bottom.mouse_at(fx, fy, &settings);
        assert_eq!(bottom.over, Zone::Frame);
        assert!(bottom.press(fx, fy));
        // dragged near the top left, it snaps to the margin there
        bottom.drag_to(fx - places.frame.x + 9.0, fy - places.frame.y + 13.0);
        let place = bottom.release().expect("placed");
        assert_eq!(place.anchor, Anchor::TopLeft);
        assert!((place.dx * 1080.0 - 22.0).abs() < 1.0 && (place.dy * 1080.0 - 22.0).abs() < 1.0, "{place:?}");
        store_placement(&mut settings.pc, place);
        bottom.compose(window, &settings, true, (400, 240), None);
        let moved = bottom.composed.expect("placed");
        assert_eq!((moved.frame.x, moved.frame.y), (22.0, 22.0));
        // the lower right corner, pulled out, makes it bigger, still 4:3
        let (rx, ry) = (moved.frame.x + moved.frame.width - 3.0, moved.frame.y + moved.frame.height - 3.0);
        bottom.mouse_at(rx, ry, &settings);
        assert_eq!(bottom.over, Zone::Corner(3));
        assert!(bottom.press(rx, ry));
        bottom.drag_to(rx + 200.0, ry + 150.0);
        let place = bottom.release().expect("sized");
        store_placement(&mut settings.pc, place);
        bottom.compose(window, &settings, true, (400, 240), None);
        let bigger = bottom.composed.expect("placed");
        assert!(bigger.bottom.height > moved.bottom.height + 100.0, "{bigger:?}");
        assert!((bigger.bottom.width / bigger.bottom.height - 4.0 / 3.0).abs() < 0.01);
        assert!((bigger.frame.x - moved.frame.x).abs() <= 1.0 && (bigger.frame.y - moved.frame.y).abs() <= 1.0, "the opposite corner stays");
        // a pixel inside still maps to the game
        let (px, py) = panel::window_point(bigger.bottom, 200.0, 33.0);
        assert_eq!(panel::touch_point(bigger.bottom, window, px, py), Some((200, 33)));
        // the docked panel cannot be dragged
        settings.pc.bottom_screen = Mode::Panel;
        bottom.poll(&settings, true);
        bottom.compose(window, &settings, true, (400, 240), None);
        let docked = bottom.composed.expect("placed");
        assert!(!bottom.press(docked.frame.x + docked.frame.width / 2.0, docked.frame.y + 2.0));
    }

    #[test]
    fn a_tap_toggles_and_a_hold_shows_while_held() {
        let mut settings = Settings::default();
        settings.pc.bottom_screen = Mode::OnDemand;
        let mut bottom = BottomScreen::new();
        assert!(!bottom.is_shown(PanelMode::OnDemand));
        bottom.button(true, &settings, true);
        bottom.button(false, &settings, true);
        assert!(bottom.is_shown(PanelMode::OnDemand), "a tap toggles it on");
        bottom.button(true, &settings, true);
        bottom.held_since = Some(Instant::now() - HOLD * 2);
        bottom.held_polls = HOLD_POLLS;
        assert!(!bottom.is_shown(PanelMode::OnDemand));
        bottom.button(false, &settings, true);
        assert!(bottom.is_shown(PanelMode::OnDemand), "a hold goes back to how it was");
    }

    /// l3toggle: an input script's frames, through the real keyboard and
    /// controller mapping and the input API, into the panel's poll, each
    /// frame taking `frame_ms`, but the one `slow` names its own time (a
    /// hitch). The frames at which what is wanted changed.
    fn panel_changes(script: &str, mode: Mode, frames: u64, frame_ms: u64, slow: Option<(u64, u64)>) -> Vec<(u64, bool)> {
        let mut settings = Settings::default();
        settings.pc.bottom_screen = mode;
        crate::input_api::configure(true, settings.pc.glyphs, settings.pc.button_layout, &settings.keys, &settings.pad);
        let mut script = crate::input_script::Script::from_text(script, "l3toggle test", &settings);
        let mut bottom = BottomScreen::new();
        bottom.ignore_pads = false;
        let panel = BottomScreen::mode(&settings, true).expect("a panel mode");
        let mut was = None;
        let mut changes = Vec::new();
        for frame in 0..frames {
            script.input(frame);
            bottom.poll(&settings, true);
            let wanted = bottom.wanted(panel);
            if was.is_some_and(|was| was != wanted) {
                changes.push((frame, wanted));
            }
            was = Some(wanted);
            let ms = if slow.is_some_and(|(at, _)| at == frame) { slow.map_or(0, |(_, ms)| ms) } else { frame_ms };
            std::thread::sleep(Duration::from_millis(ms));
        }
        // nothing held is left for the other tests
        crate::input_api::update(zakuro_core::services::hid::PadState::empty(), [false; crate::controls::PcAction::COUNT], [0.0; 2], &crate::controls::PadOutput::default(), crate::controls::Device::Keyboard, Vec::new());
        changes
    }

    /// l3toggle: a player's report, "L3 activates on both a press and
    /// release": one press of L3 (or M) is one toggle, on the press; held,
    /// it does not repeat, and letting go does nothing, however long it
    /// was held, in every mode but On Demand's hold-to-peek.
    #[test]
    fn l3_toggles_once_per_press_not_on_release() {
        let _lock = crate::input_api::test_lock();
        // L3 on a PlayStation pad, held 20 frames of 16 ms (~330 ms, a firm click)
        let l3 = "10 pad playstation LeftThumb down
30 pad playstation LeftThumb up
";
        assert_eq!(panel_changes(l3, Mode::Auto, 45, 16, None), vec![(10, true)], "Auto: shown on the press, still shown after the release");
        // a quick click, and a second one: two toggles, one each
        let two = "10 pad xbox LeftThumb down
14 pad xbox LeftThumb up
24 pad xbox LeftThumb down
40 pad xbox LeftThumb up
";
        assert_eq!(panel_changes(two, Mode::Auto, 50, 16, None), vec![(10, true), (24, false)], "Auto: one toggle per press");
        // held a long while: no repeat, and letting go leaves it
        let long = "5 pad nintendo LeftThumb down
70 pad nintendo LeftThumb up
";
        assert_eq!(panel_changes(long, Mode::Auto, 80, 16, None), vec![(5, true)], "Auto: a long hold toggles once");
        // the keyboard's M, the same action
        let m = "10 key KeyM down
30 key KeyM up
";
        assert_eq!(panel_changes(m, Mode::Auto, 40, 16, None), vec![(10, true)], "Auto: M toggles once");
        // the always-shown modes: hidden on the press, and stay hidden
        assert_eq!(panel_changes(l3, Mode::Overlay, 40, 16, None), vec![(10, false)], "Overlay");
        assert_eq!(panel_changes(l3, Mode::Panel, 40, 16, None), vec![(10, false)], "Panel");
        // On Demand: a click across a hitch (one frame of 600 ms) is a
        // click, not a hold, so it stays out
        let hitch = "10 pad xbox LeftThumb down
12 pad xbox LeftThumb up
";
        assert_eq!(panel_changes(hitch, Mode::OnDemand, 20, 16, Some((10, 600))), vec![(10, true)], "On Demand: a click across a hitch");
        assert_eq!(panel_changes(l3, Mode::OnDemand, 40, 12, None), vec![(10, true)], "On Demand: a firm click");
        // On Demand's hold-to-peek, as its option says: out while held
        let peek = "10 pad xbox LeftThumb down
60 pad xbox LeftThumb up
";
        assert_eq!(panel_changes(peek, Mode::OnDemand, 70, 16, None), vec![(10, true), (60, false)], "On Demand: a long hold peeks");
    }

    /// l3toggle: live, through SDL3: a virtual pad's (dev/tools/vpad) or a
    /// real one's L3, polled as the window polls it, each toggle printed.
    ///   FATES_PROBE_SECS=9 cargo test --lib l3_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn l3_live_through_sdl() {
        let _lock = crate::input_api::test_lock();
        let secs: u64 = std::env::var("FATES_PROBE_SECS").ok().and_then(|s| s.parse().ok()).unwrap_or(10);
        let settings = Settings::default();
        crate::input_api::configure(true, settings.pc.glyphs, settings.pc.button_layout, &settings.keys, &settings.pad);
        let mut pads = crate::pads::Pads::new(settings.pad.clone(), settings.pc.button_layout, settings.pc.stick_deadzone);
        let mut bottom = BottomScreen::new();
        bottom.ignore_pads = false;
        let panel = BottomScreen::mode(&settings, true).expect("a panel mode");
        let start = Instant::now();
        let (mut was, mut held, mut toggles, mut known) = (bottom.wanted(panel), false, 0, 0);
        while start.elapsed().as_secs() < secs {
            pads.poll();
            let list = pads.list();
            if list.len() != known {
                known = list.len();
                println!("live {:>5} ms: {known} controllers {:?}", start.elapsed().as_millis(), list.iter().map(|pad| (&pad.name, pad.kind)).collect::<Vec<_>>());
            }
            let (out, last) = pads.output();
            let device = last.map_or(crate::controls::Device::Keyboard, |(family, _)| crate::controls::Device::Pad(family));
            crate::input_api::update(zakuro_core::services::hid::PadState::empty(), [false; crate::controls::PcAction::COUNT], out.circle, &out, device, list);
            let l3 = crate::input_api::action_held(crate::controls::PcAction::BottomScreen);
            if l3 != held {
                held = l3;
                println!("live {:>5} ms: L3 {}", start.elapsed().as_millis(), if l3 { "down" } else { "up" });
            }
            bottom.poll(&settings, true);
            let wanted = bottom.wanted(panel);
            if wanted != was {
                was = wanted;
                toggles += 1;
                println!("live {:>5} ms: the panel {} (toggle {toggles})", start.elapsed().as_millis(), if wanted { "comes out" } else { "goes away" });
            }
            std::thread::sleep(Duration::from_millis(16));
        }
        println!("live: {toggles} toggles");
    }

    /// the composition's bottom screen is where touches are mapped from, and
    /// the cursor's tap goes to the pixel it points at.
    #[test]
    fn the_cursor_taps_where_it_points() {
        let mut settings = Settings::default();
        settings.pc.bottom_screen = Mode::Panel;
        let mut bottom = BottomScreen::new();
        bottom.art_tried = true;
        let composition = bottom.compose((3440, 1440), &settings, true, (400, 240), None).expect("composes");
        let (top, place) = composition.screens.expect("places the screens");
        assert!(top.is_some());
        assert_eq!(place, bottom.composed_bottom());
        bottom.cursor.x = 200.0;
        bottom.cursor.y = 33.0;
        bottom.cursor.tapping = true;
        let mut state = InputState::default();
        bottom.apply_input(&mut state);
        assert_eq!(state.touch, Some((200, 33)));
        // the mouse's touch wins
        let mut state = InputState { touch: Some((5, 6)), ..InputState::default() };
        bottom.apply_input(&mut state);
        assert_eq!(state.touch, Some((5, 6)));
    }
}
