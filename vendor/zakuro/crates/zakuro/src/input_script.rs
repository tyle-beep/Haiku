//! input: scripted keyboard and controller input for headless tests, to
//! drive the real keyboard and controller mapping (bindings, layout, dead
//! zone, merging) through the game without devices. FATES_INPUT_SCRIPT=file,
//! one event per line, `#` comments:
//!
//! ```text
//! 600 key Enter down          # a key, by winit KeyCode name
//! 606 key Enter up
//! 700 pad xbox South down     # a pad of a family (nintendo|xbox|playstation|generic), a button by position
//! 706 pad xbox South up
//! 800 pad playstation left 0.0 1.0     # a stick, x y, y up
//! 900 pad xbox triggers 0.0 1.0        # the triggers, 0..1
//! 950 cam drag 40 -10          # cameras: a left drag of 40, -10 pixels
//! 960 cam pan 30 0             # cameras: a right drag
//! 970 cam wheel 2              # cameras: the wheel, lines (up +)
//! 980 rando click 200 140      # randomizer: a left click on the upper screen, console 400x240
//! 990 rando back               # randomizer: the right button on a New Game mode screen
//! ```
//!
//! The game clock still comes from ZAKURO_REPLAY when set (a file holding
//! just its `clock` line will do). Each change of the console's buttons is
//! printed, so a run's log shows what the script made of the keys.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::de::IntoDeserializer;
use serde::Deserialize;
use winit::keyboard::{KeyCode, PhysicalKey};
use zakuro_core::services::hid::{InputState, PadState};

use crate::controls::{self, Device, Family, PadButton, PadOutput, PadReading};
use crate::input::Keyboard;
use crate::settings::Settings;

enum Event {
    Key(KeyCode, bool),
    Button(Family, PadButton, bool),
    Stick(Family, bool, [f32; 2]),
    Triggers(Family, [f32; 2]),
    /// offline-msg: a left click on the game, off the lower screen
    Click,
    /// cameras: a mouse drag or the wheel, for the free camera
    Cam(&'static str, f32, f32),
    /// randomizer: a click on a New Game mode screen's panel, or back
    RandoClick(f32, f32),
    RandoBack,
}

pub struct Script {
    events: Vec<(u64, Event)>,
    next: usize,
    keyboard: Keyboard,
    pads: HashMap<Family, PadReading>,
    settings: Settings,
    last: Device,
    shown: Option<InputState>,
    /// offline-msg: a click that came up, for take_click
    clicked: bool,
}

fn family(name: &str) -> Option<Family> {
    Some(match name {
        "nintendo" => Family::Nintendo,
        "xbox" => Family::Xbox,
        "playstation" | "ps" => Family::PlayStation,
        "generic" => Family::Generic,
        _ => return None,
    })
}

fn parse<T: for<'de> Deserialize<'de>>(name: &str) -> Option<T> {
    T::deserialize(IntoDeserializer::<serde::de::value::Error>::into_deserializer(name)).ok()
}

impl Script {
    pub fn open(settings: &Settings) -> Option<Script> {
        let path = PathBuf::from(std::env::var_os("FATES_INPUT_SCRIPT")?);
        let text = std::fs::read_to_string(&path).inspect_err(|error| eprintln!("input script {}: {error}", path.display())).ok()?;
        Some(Script::from_text(&text, &path.display().to_string(), settings))
    }

    /// l3toggle: a script from its text, so tests can drive the same
    /// keyboard and controller mapping without a file.
    pub fn from_text(text: &str, name: &str, settings: &Settings) -> Script {
        let mut events = Vec::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.split('#').next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let fields: Vec<&str> = line.split_whitespace().collect();
            let event = (|| {
                let frame: u64 = fields.first()?.parse().ok()?;
                let float = |i: usize| fields.get(i).and_then(|v| v.parse::<f32>().ok());
                let event = match fields.get(1..)? {
                    ["key", key, state] => Event::Key(parse(key)?, *state == "down"),
                    ["click"] => Event::Click, // offline-msg
                    // cameras
                    ["cam", "drag", ..] => Event::Cam("drag", float(3)?, float(4)?),
                    ["cam", "pan", ..] => Event::Cam("pan", float(3)?, float(4)?),
                    ["cam", "wheel", ..] => Event::Cam("wheel", float(3)?, 0.0),
                    ["rando", "click", ..] => Event::RandoClick(float(3)?, float(4)?),
                    ["rando", "back"] => Event::RandoBack,
                    ["pad", f, "left", ..] => Event::Stick(family(f)?, false, [float(4)?, float(5)?]),
                    ["pad", f, "right", ..] => Event::Stick(family(f)?, true, [float(4)?, float(5)?]),
                    ["pad", f, "triggers", ..] => Event::Triggers(family(f)?, [float(4)?, float(5)?]),
                    ["pad", f, button, state] => Event::Button(family(f)?, parse(button)?, *state == "down"),
                    _ => return None,
                };
                Some((frame, event))
            })();
            match event {
                Some(event) => events.push(event),
                None => eprintln!("input script line {}: cannot read \"{line}\"", number + 1),
            }
        }
        events.sort_by_key(|(frame, _)| *frame);
        println!("input script {name}: {} events", events.len());
        Script {
            events,
            next: 0,
            keyboard: Keyboard::new(settings.keys.clone()),
            pads: HashMap::new(),
            settings: settings.clone(),
            last: Device::Keyboard,
            shown: None,
            clicked: false,
        }
    }

    /// offline-msg: whether a click came up since the last call.
    pub fn take_click(&mut self) -> bool {
        std::mem::take(&mut self.clicked)
    }

    /// the input for `frame`, after every event up to it.
    pub fn input(&mut self, frame: u64) -> InputState {
        while let Some((at, event)) = self.events.get(self.next) {
            if *at > frame {
                break;
            }
            match *event {
                Event::Key(key, down) => {
                    self.keyboard.key(PhysicalKey::Code(key), down);
                    self.last = Device::Keyboard;
                }
                Event::Button(family, button, down) => {
                    self.pads.entry(family).or_default().set(button, down);
                    self.last = Device::Pad(family);
                }
                Event::Stick(family, right, value) => {
                    let pad = self.pads.entry(family).or_default();
                    if right { pad.right = value } else { pad.left = value }
                    self.last = Device::Pad(family);
                }
                Event::Triggers(family, value) => {
                    self.pads.entry(family).or_default().triggers = value;
                    self.last = Device::Pad(family);
                }
                Event::Click => {
                    println!("input script frame {frame}: click");
                    self.clicked = true;
                }
                Event::Cam(kind, a, b) => {
                    println!("input script frame {frame}: camera {kind} {a} {b}");
                    crate::cameras::script(kind, a, b);
                }
                Event::RandoClick(x, y) => {
                    let hit = zakuro_core::randomizer::menu::menu_click(x, y);
                    println!("input script frame {frame}: randomizer click {x} {y}, on a panel: {hit}");
                }
                Event::RandoBack => {
                    let up = zakuro_core::randomizer::menu::menu_back();
                    println!("input script frame {frame}: randomizer back, a mode screen up: {up}");
                }
            }
            self.next += 1;
        }
        let pc = &self.settings.pc;
        let mut pads = PadOutput::default();
        for (family, reading) in &self.pads {
            pads.merge(&controls::resolve(&self.settings.pad, family.positional(pc.button_layout), reading, pc.stick_deadzone));
        }
        let state = crate::input::combine(self.keyboard.state(), &pads);
        crate::input_api::update(self.keyboard.buttons(), self.keyboard.actions(), [state.circle_x, state.circle_y], &pads, self.last, Vec::new());
        if self.shown.is_none_or(|shown| shown.buttons != state.buttons || (shown.circle_x, shown.circle_y) != (state.circle_x, state.circle_y)) {
            println!(
                "input script frame {frame}: 3DS buttons {} circle ({:.2}, {:.2}) right stick ({:.2}, {:.2}) actions {:?} device {:?}",
                names(state.buttons),
                state.circle_x,
                state.circle_y,
                pads.right_stick[0],
                pads.right_stick[1],
                crate::controls::PcAction::ALL.iter().filter(|a| pads.action(**a)).collect::<Vec<_>>(),
                self.last,
            );
            self.shown = Some(state);
        }
        state
    }
}

fn names(buttons: PadState) -> String {
    let list: Vec<&str> = buttons.iter_names().map(|(name, _)| name).collect();
    if list.is_empty() { "-".into() } else { list.join("+") }
}
