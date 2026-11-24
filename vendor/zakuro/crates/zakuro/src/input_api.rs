//! input: the public input API for the rest of the port.
//!
//! The frontend updates it once per app frame (`App::poll_gamepads`); any
//! module may read it from any thread. Other agents use it for:
//!
//! - **bottom screen / touch cursor**: [`right_stick`], [`action_held`] and
//!   [`take_action`] with [`PcAction::TouchClick`] (default R3) and
//!   [`PcAction::BottomScreen`] (default L3). The right stick is *not* fed
//!   to the game's C-stick; it is reserved for the virtual touch pointer.
//! - **ui prompts and menus**: [`glyph_set`] (resolves `pc.glyphs`, Auto =
//!   the device used last), [`controller_family`], [`prompt_name`] (the
//!   key or button a 3DS button is on, in the current glyph set),
//!   [`snapshot`] for the merged, layout-corrected 3DS buttons (so an
//!   options menu can navigate with A = confirm and B = back on any pad),
//!   [`take_action`] with [`PcAction::Options`] (default Guide/Home; the
//!   keyboard's Escape is handled by the ui agent directly).
//! - **rumble**: [`rumble`] from Rust, or `fates_pc_rumble` from C game
//!   overrides. Honours `pc.rumble`.

use std::sync::Mutex;

use zakuro_core::services::hid::PadState;

pub use crate::controls::{Device, Family, GlyphSet, PadButton, PcAction};
use crate::controls::{Keys, PadButtons, PadOutput};
use crate::settings::{ButtonLayout, Glyphs};

/// a connected controller, as the backend reported it.
#[derive(Debug, Clone, PartialEq)]
pub struct PadInfo {
    pub name: String,
    pub family: Family,
    /// SDL_GamepadType's name, "Switch Pro", "Joy-Con pair", "DualSense"...
    pub kind: &'static str,
    /// "sdl" or "gilrs".
    pub backend: &'static str,
    pub rumble: bool,
}

/// everything the API knows, as of the last frame.
#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    /// the console's buttons held on every device together, after the
    /// bindings and the button layout.
    pub buttons: PadState,
    /// the ones newly pressed in the last frame.
    pub pressed: PadState,
    /// the circle pad: left stick or keys, -1..1, y up.
    pub circle: [f32; 2],
    /// the right stick after the dead zone, -1..1, y up.
    pub right_stick: [f32; 2],
    pub actions: [bool; PcAction::COUNT], // cameras: was 4
    pub last_device: Device,
    pub glyphs: GlyphSet,
    pub pads: Vec<PadInfo>,
}

#[derive(Default)]
struct State {
    snapshot: Snapshot,
    /// actions pressed since they were last taken.
    latched: [bool; PcAction::COUNT], // cameras: was 4
    rumble_on: bool,
    glyph_setting: Glyphs,
    layout: ButtonLayout,
    keys: Keys,
    pad: PadButtons,
    /// rumble asked for and not yet sent: low, high (0..=65535), ms.
    rumble_queue: Vec<(u16, u16, u32)>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);

/// l3toggle: tests that drive the shared input state hold this, so they
/// do not see each other's frames.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn with<R>(f: impl FnOnce(&mut State) -> R) -> R {
    let mut guard = STATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    f(guard.get_or_insert_with(|| State { rumble_on: true, ..State::default() }))
}

fn index(action: PcAction) -> usize {
    PcAction::ALL.iter().position(|a| *a == action).unwrap()
}

/// a copy of the whole state.
pub fn snapshot() -> Snapshot {
    with(|state| state.snapshot.clone())
}

/// the button pictures prompts should show now.
pub fn glyph_set() -> GlyphSet {
    with(|state| state.snapshot.glyphs)
}

/// the family of the controller used last, none while the keyboard was.
pub fn controller_family() -> Option<Family> {
    with(|state| match state.snapshot.last_device {
        Device::Pad(family) => Some(family),
        Device::Keyboard => None,
    })
}

pub fn last_device() -> Device {
    with(|state| state.snapshot.last_device)
}

/// the controllers connected now.
pub fn connected_pads() -> Vec<PadInfo> {
    with(|state| state.snapshot.pads.clone())
}

/// the right stick, -1..1 each way, y up, after `pc.stick_deadzone`.
pub fn right_stick() -> [f32; 2] {
    with(|state| state.snapshot.right_stick)
}

pub fn action_held(action: PcAction) -> bool {
    with(|state| state.snapshot.actions[index(action)])
}

/// statesnd: an action pressed as its key or button would press it (the UI
/// script's `quick save` and `quick load`).
pub fn press_action(action: PcAction) {
    with(|state| state.latched[index(action)] = true);
}

/// whether the action was pressed since the last call for it.
pub fn take_action(action: PcAction) -> bool {
    with(|state| std::mem::take(&mut state.latched[index(action)]))
}

/// the key or controller button a console button is on, in the current
/// glyph set: "A", "Cross", "Enter"...
pub fn prompt_name(button: PadState) -> String {
    with(|state| crate::controls::prompt_name(button, state.snapshot.glyphs, &state.keys, &state.pad, state.layout))
}

/// rumbles every connected controller that can: `low` and `high` are the
/// two motors' strengths, 0..1, for `ms` milliseconds. Does nothing when
/// `pc.rumble` is off. Safe from any thread; sent on the next frame.
pub fn rumble(low: f32, high: f32, ms: u32) {
    let scale = |value: f32| (value.clamp(0.0, 1.0) * 65535.0) as u16;
    with(|state| {
        if state.rumble_on && state.rumble_queue.len() < 16 {
            state.rumble_queue.push((scale(low), scale(high), ms.min(5000)));
        }
    });
}

/// for game overrides in C: `void fates_pc_rumble(uint16_t low, uint16_t
/// high, uint32_t ms);`
#[no_mangle]
pub extern "C" fn fates_pc_rumble(low: u16, high: u16, ms: u32) {
    rumble(low as f32 / 65535.0, high as f32 / 65535.0, ms);
}

// ---- for the frontend ----

/// the settings the API resolves with.
pub(crate) fn configure(rumble: bool, glyphs: Glyphs, layout: ButtonLayout, keys: &Keys, pad: &PadButtons) {
    with(|state| {
        state.rumble_on = rumble;
        state.glyph_setting = glyphs;
        state.layout = layout;
        state.keys = keys.clone();
        state.pad = pad.clone();
        if !rumble {
            state.rumble_queue.clear();
        }
        state.snapshot.glyphs = GlyphSet::resolve(glyphs, state.snapshot.last_device);
    });
}

/// one frame's input: the keyboard's console buttons and actions, the pads'
/// merged output, and which device was used last.
pub(crate) fn update(keyboard: PadState, keyboard_actions: [bool; PcAction::COUNT], circle: [f32; 2], pads: &PadOutput, last: Device, list: Vec<PadInfo>) {
    with(|state| {
        let buttons = keyboard | pads.buttons;
        let mut actions = pads.actions;
        for (action, keyboard) in actions.iter_mut().zip(keyboard_actions) {
            *action |= keyboard;
        }
        for (i, held) in actions.iter().enumerate() {
            if *held && !state.snapshot.actions[i] {
                state.latched[i] = true;
            }
        }
        let snapshot = &mut state.snapshot;
        snapshot.pressed = buttons & !snapshot.buttons;
        snapshot.buttons = buttons;
        snapshot.circle = circle;
        snapshot.right_stick = pads.right_stick;
        snapshot.actions = actions;
        if snapshot.last_device != last {
            log::debug!("input: now using {last:?}");
        }
        snapshot.last_device = last;
        snapshot.glyphs = GlyphSet::resolve(state.glyph_setting, last);
        snapshot.pads = list;
    });
}

/// the rumble asked for since the last call.
pub(crate) fn take_rumble() -> Vec<(u16, u16, u32)> {
    with(|state| std::mem::take(&mut state.rumble_queue))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_latch_until_taken_and_rumble_queues() {
        let _lock = test_lock();
        configure(true, Glyphs::Auto, ButtonLayout::Auto, &Keys::default(), &PadButtons::default());
        let mut pads = PadOutput::default();
        pads.actions[index(PcAction::TouchClick)] = true;
        pads.right_stick = [0.5, -0.5];
        update(PadState::A, [false; PcAction::COUNT], [0.0; 2], &pads, Device::Pad(Family::PlayStation), Vec::new());
        assert_eq!(right_stick(), [0.5, -0.5]);
        assert!(action_held(PcAction::TouchClick));
        assert!(take_action(PcAction::TouchClick));
        assert!(!take_action(PcAction::TouchClick), "taken once");
        assert_eq!(glyph_set(), GlyphSet::PlayStation);
        assert_eq!(controller_family(), Some(Family::PlayStation));
        assert_eq!(prompt_name(PadState::A), "Cross");
        assert_eq!(snapshot().pressed, PadState::A);
        update(PadState::A, [false; PcAction::COUNT], [0.0; 2], &PadOutput::default(), Device::Keyboard, Vec::new());
        assert_eq!(snapshot().pressed, PadState::empty(), "held, not newly pressed");
        assert_eq!(prompt_name(PadState::A), "Enter");
        fates_pc_rumble(65535, 0, 200);
        assert_eq!(take_rumble(), vec![(65535, 0, 200)]);
        configure(false, Glyphs::Auto, ButtonLayout::Auto, &Keys::default(), &PadButtons::default());
        rumble(1.0, 1.0, 100);
        assert!(take_rumble().is_empty(), "pc.rumble off");
    }
}
