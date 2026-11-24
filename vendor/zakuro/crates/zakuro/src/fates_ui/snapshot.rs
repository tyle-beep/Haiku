//! ui: pictures of the interface over the game's last frame, at the window
//! sizes it has to work at, drawn without a window or GPU: egui's meshes are
//! filled on the CPU the way the overlay pipeline fills them (premultiplied
//! colour times texture, blended over). FATES_UI_SNAPSHOT=<dir> on a
//! headless run writes them, for checking the look without a display.

use std::collections::HashMap;
use std::path::Path;

use egui::epaint::{Primitive, Vertex};
use egui::{Color32, Pos2, Rect, TextureId, Vec2};
use zakuro_common::Screen;
use zakuro_core::System;

/// a screen's picture: its pixels, width and height.
type Shot = zakuro_core::Screen;

use super::options::{Env, Page};
use super::{FatesUi, View};
use crate::settings::{Screens, Settings};

struct Texture {
    size: [usize; 2],
    pixels: Vec<Color32>,
}

/// an RGBA picture in floats, premultiplied.
struct Frame {
    width: usize,
    height: usize,
    pixels: Vec<[f32; 4]>,
}

impl Frame {
    fn blend(&mut self, x: usize, y: usize, src: [f32; 4]) {
        let dst = &mut self.pixels[y * self.width + x];
        for c in 0..4 {
            dst[c] = src[c] + dst[c] * (1.0 - src[3]);
        }
    }

    fn save(&self, path: &Path) {
        let bytes: Vec<u8> = self.pixels.iter().flat_map(|p| [p[0], p[1], p[2], 1.0].map(|v| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8)).collect();
        if let Err(error) = image::save_buffer(path, &bytes, self.width as u32, self.height as u32, image::ColorType::Rgba8) {
            eprintln!("could not write {}: {error}", path.display());
        }
    }
}

fn sample(texture: &Texture, uv: [f32; 2]) -> [f32; 4] {
    let [w, h] = texture.size;
    let x = (uv[0] * w as f32 - 0.5).clamp(0.0, w as f32 - 1.0);
    let y = (uv[1] * h as f32 - 0.5).clamp(0.0, h as f32 - 1.0);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let get = |x: usize, y: usize| texture.pixels[y * w + x].to_array().map(|v| v as f32 / 255.0);
    let (a, b, c, d) = (get(x0, y0), get(x1, y0), get(x0, y1), get(x1, y1));
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
    }
    out
}

fn raster(frame: &mut Frame, textures: &HashMap<TextureId, Texture>, clip: Rect, texture: TextureId, v: [&Vertex; 3]) {
    let Some(texture) = textures.get(&texture) else { return };
    let min_x = v.iter().map(|v| v.pos.x).fold(f32::MAX, f32::min).max(clip.min.x).max(0.0).floor() as i64;
    let max_x = v.iter().map(|v| v.pos.x).fold(f32::MIN, f32::max).min(clip.max.x).min(frame.width as f32).ceil() as i64;
    let min_y = v.iter().map(|v| v.pos.y).fold(f32::MAX, f32::min).max(clip.min.y).max(0.0).floor() as i64;
    let max_y = v.iter().map(|v| v.pos.y).fold(f32::MIN, f32::max).min(clip.max.y).min(frame.height as f32).ceil() as i64;
    let (a, b, c) = (v[0].pos, v[1].pos, v[2].pos);
    let area = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    if area.abs() < 1e-6 {
        return;
    }
    for y in min_y..max_y {
        for x in min_x..max_x {
            let p = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((b.x - p.x) * (c.y - p.y) - (b.y - p.y) * (c.x - p.x)) / area;
            let w1 = ((c.x - p.x) * (a.y - p.y) - (c.y - p.y) * (a.x - p.x)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let uv = [
                v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2,
                v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2,
            ];
            let colour = |i: usize| (v[0].color.to_array()[i] as f32 * w0 + v[1].color.to_array()[i] as f32 * w1 + v[2].color.to_array()[i] as f32 * w2) / 255.0;
            let t = sample(texture, uv);
            let src = [colour(0) * t[0], colour(1) * t[1], colour(2) * t[2], colour(3) * t[3]];
            if src[3] > 0.0 || src[0] > 0.0 {
                frame.blend(x as usize, y as usize, src);
            }
        }
    }
}

/// qol: the top screen's width at the console's height, its picture's own
/// shape (wider than 400 with a widescreen aspect).
fn top_width(top: &Shot) -> f32 {
    let (_, w, h) = top;
    if *w == 0 || *h == 0 { 400.0 } else { (240.0 * *w as f32 / *h as f32).max(400.0) }
}

/// the game's screens laid out in a window of size, as the presenter
/// places them.
fn backdrop(size: [usize; 2], top: &Shot, bottom: &Shot) -> Frame {
    let mut frame = Frame { width: size[0], height: size[1], pixels: vec![[0.0, 0.0, 0.0, 1.0]; size[0] * size[1]] };
    let (top_view, bottom_view) =
        zakuro_gpu::layout(size[0] as u32, size[1] as u32, Screens::LargeTop.screens(), zakuro_gpu::ScreenOrientation::North, top_width(top), false);
    for (view, (pixels, w, h)) in [(top_view, top), (bottom_view, bottom)] {
        let Some(view) = view else { continue };
        if *w == 0 || *h == 0 || pixels.len() < (*w * *h * 4) as usize {
            continue;
        }
        let image = Texture {
            size: [*w as usize, *h as usize],
            pixels: pixels.chunks_exact(4).map(|p| Color32::from_rgba_premultiplied(p[0], p[1], p[2], 255)).collect(),
        };
        for y in view.y.max(0.0) as usize..((view.y + view.height) as usize).min(size[1]) {
            for x in view.x.max(0.0) as usize..((view.x + view.width) as usize).min(size[0]) {
                let uv = [(x as f32 + 0.5 - view.x) / view.width, (y as f32 + 0.5 - view.y) / view.height];
                frame.pixels[y * size[0] + x] = sample(&image, uv);
            }
        }
    }
    frame
}

/// a scene to picture: what is open, and what the frame shows.
struct Scene {
    name: &'static str,
    setup: fn(&mut FatesUi, &mut View, &mut Settings),
}

fn options_at(fates: &mut FatesUi, tab: usize, page: Page, row: usize) {
    super::cheats::snapshot_reset(); // cheats: no question left from a scene before
    crate::haiku_voices::set_asking(false); // jpvoice: nor the voices' restart question
    crate::haiku_setup_entry::set_asking(false); // oneexe: nor Haiku Setup's
    crate::haiku_versions::set_asking(false); // versions: nor the version's restart
    super::states::snapshot_reset(); // statecheck: nor a save state's dialog
    fates.options.show(0.0);
    fates.options.select(tab, page, row);
}

const SCENES: &[Scene] = &[
    Scene { name: "options-display", setup: |f, _, _| options_at(f, 0, Page::Main, 1) },
    // wide-ui: the Ultrawide Sides row (after Aspect Ratio; one row later with the Renderer row)
    Scene { name: "options-sides", setup: |f, _, _| options_at(f, 0, Page::Main, if cfg!(feature = "emulation") { 6 } else { 5 }) },
    // transblur: the Battle Transition Blur row (after Full Draw Distance;
    // hfr60: one row later with Smooth 30 FPS Scenes; vrrcap: and Variable Refresh)
    Scene { name: "options-transition-blur", setup: |f, _, _| options_at(f, 0, Page::Main, if cfg!(feature = "emulation") { 15 } else { 14 }) },
    // hfr60: the Smooth 30 FPS Scenes row (after FPS Cap; vrrcap: and Variable Refresh)
    Scene { name: "options-smooth30", setup: |f, _, _| options_at(f, 0, Page::Main, if cfg!(feature = "emulation") { 11 } else { 10 }) },
    // vrrcap: the Variable Refresh row (after FPS Cap)
    Scene { name: "options-vrr", setup: |f, _, _| options_at(f, 0, Page::Main, if cfg!(feature = "emulation") { 10 } else { 9 }) },
    // gfx-hfr: the Renderer row, set to the emulated GPU (native-final:
    // development builds only; the pure build has no such row)
    #[cfg(feature = "emulation")]
    Scene { name: "options-renderer", setup: |f, _, s| {
        options_at(f, 0, Page::Main, 4);
        s.pc.native_graphics = false;
    } },
    Scene { name: "options-bottom", setup: |f, _, _| options_at(f, 1, Page::Main, 0) },
    Scene { name: "options-audio", setup: |f, _, _| options_at(f, 2, Page::Main, 2) },
    // jpvoice: Voices (with the voice layer installed), as the game runs;
    // then changed, with the restart row, and the restart's question
    Scene { name: "options-voices", setup: |f, _, s| {
        s.pc.voices = crate::haiku_voices::running();
        crate::haiku_voices::chosen(s.pc.voices);
        options_at(f, 2, Page::Main, 4);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-voices-pending", setup: |f, _, s| {
        s.pc.voices = crate::haiku_voices::other(crate::haiku_voices::running());
        crate::haiku_voices::chosen(s.pc.voices);
        options_at(f, 2, Page::Main, 5);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-voices-confirm", setup: |f, _, s| {
        s.pc.voices = crate::haiku_voices::other(crate::haiku_voices::running());
        crate::haiku_voices::chosen(s.pc.voices);
        options_at(f, 2, Page::Main, 5);
        crate::haiku_voices::set_asking(true);
        f.options.ask_restart();
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-controls", setup: |f, _, _| options_at(f, 3, Page::Main, 5) },
    Scene { name: "options-keyboard", setup: |f, _, _| options_at(f, 3, Page::Keyboard, 9) },
    Scene { name: "options-controller-pad", setup: |f, _, s| {
        options_at(f, 3, Page::Controller, 0);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-interface-ps", setup: |f, _, s| {
        options_at(f, 5, Page::Main, 0); // qol: Gameplay is the fifth tab now
        s.pc.glyphs = crate::settings::Glyphs::PlayStation;
    } },
    // mods: the Mods page as a tab, on its own from the Main Menu, on the
    // keyboard, and the restart dialog
    // qol: the Gameplay tab, and the boon/bane panel over the frame
    Scene { name: "options-gameplay", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 0);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // tones: Speaking Style (with a translation on), the row on each style
    // of the loaded game's Corrin (as it is when the run ends, gender kept)
    Scene { name: "options-tones-1", setup: |f, _, _| tones_scene(f, 1) },
    Scene { name: "options-tones-2", setup: |f, _, _| tones_scene(f, 2) },
    Scene { name: "options-tones-3", setup: |f, _, _| tones_scene(f, 3) },
    // tones: the Gameplay tab on Kana's Gender (row 10): what follows it
    Scene { name: "options-gameplay-kana", setup: |f, _, _| options_at(f, 4, Page::Main, 10) },
    Scene { name: "options-gameplay-skip", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 2);
        s.pc.skip_story = true;
        s.pc.game_speed = 3;
        s.pc.game_speed_fast = 3;
        s.pc.glyphs = crate::settings::Glyphs::Keyboard;
    } },
    Scene { name: "boonbane", setup: |_, _, _| {} },
    // info-ui: the growth button over the lower screen, off and on
    Scene { name: "growth-button", setup: |_, _, _| {} },
    Scene { name: "growth-view", setup: |_, _, _| {} },
    // info-ui: as the run left it: the button only while the unit status
    // shows, the view on when it was pressed (a touch, the key)
    Scene { name: "growth-live", setup: |_, _, _| {} },
    // info-ui: the Gameplay tab's two rows
    // qol3: the rows after Skip Story moved up one with SpotPass gone
    // qol4: on or off
    Scene { name: "options-targeting", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 7);
        s.pc.targeting_lines = crate::settings::TargetingLines::On;
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-targeting-off", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 7);
        s.pc.targeting_lines = crate::settings::TargetingLines::Off;
        s.pc.glyphs = crate::settings::Glyphs::Keyboard;
    } },
    Scene { name: "options-growth", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 8);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // qol4: Return to Title before Exit to Desktop, and its question
    Scene { name: "options-title", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 9);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-title-confirm", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 9);
        f.options.ask_title();
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-exit", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 10);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-exit-confirm", setup: |f, _, s| {
        options_at(f, 4, Page::Main, 10);
        f.options.ask_exit();
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // oneexe: Interface's Haiku Setup row (a release install; the picture
    // run sets HAIKU_SETUP_ENTRY=1), and its Yes/No
    Scene { name: "options-setup", setup: |f, _, s| {
        options_at(f, 5, Page::Main, 1);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-setup-confirm", setup: |f, _, s| {
        options_at(f, 5, Page::Main, 1);
        f.options.ask_setup();
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // qol4: the options menu's own key and button, on the Controls pages
    Scene { name: "options-keyboard-options-key", setup: |f, _, s| {
        options_at(f, 3, Page::Keyboard, crate::controls::Keys::default().all_mut().len() - 1);
        s.keys.assign(crate::controls::Keys::default().all_mut().len() - 1, winit::keyboard::KeyCode::F2);
        s.pc.glyphs = crate::settings::Glyphs::Keyboard;
    } },
    Scene { name: "options-controller-options-button", setup: |f, _, s| {
        options_at(f, 3, Page::Controller, 15);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // qol3: the title Main Menu's Exit to Desktop: the question alone
    Scene { name: "title-exit", setup: |f, _, s| {
        f.options.show_exit(0.0);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "options-mods", setup: |f, _, s| {
        options_at(f, 6, Page::Main, 1);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "mods-title", setup: |f, _, s| {
        f.options.show_mods(0.0);
        f.options.select(6, Page::Main, 0);
        s.pc.glyphs = crate::settings::Glyphs::Nintendo;
    } },
    Scene { name: "mods-keyboard", setup: |f, _, s| {
        options_at(f, 6, Page::Main, 2);
        s.pc.glyphs = crate::settings::Glyphs::Keyboard;
    } },
    Scene { name: "mods-apply", setup: |f, _, s| {
        options_at(f, 6, Page::Main, 5);
        s.pc.glyphs = crate::settings::Glyphs::Nintendo;
    } },
    Scene { name: "mods-confirm", setup: |f, _, s| {
        options_at(f, 6, Page::Main, 5);
        f.options.ask_restart();
        s.pc.glyphs = crate::settings::Glyphs::Nintendo;
    } },
    // hdpacks: a download being unpacked, over the Mods page
    Scene { name: "mods-unpacking", setup: |f, _, s| {
        f.env.mods.rows.push(super::mods::ModRow {
            name: "Combined HD Pack for Fates".into(),
            description: "A texture pack for Citra, downloaded in 3 parts, 7 zips inside.".into(),
            enabled: true,
            can_enable: true,
            notes: vec![crate::haiku_mods::Note { warning: false, text: "Unpacking 3 zips, 2.7 GB into mods/Combined HD Pack for Fates...".into() }],
            ..Default::default()
        });
        options_at(f, 6, Page::Main, 4);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // codemods: a code mod's notes, the compiler question, the compile
    Scene { name: "mods-code", setup: |f, _, s| {
        options_at(f, 6, Page::Main, 3);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // codemods2: a code mod that can't be compiled, its note in plain words
    Scene { name: "mods-code-failed", setup: |f, _, s| {
        let failure = crate::haiku_codemods::Failure {
            text: "It uses 1 instruction the PC can't run (the first at 0x00419A48).".into(),
            mods: vec!["Broken Mod".into()],
        };
        f.env.mods.rows.push(super::mods::ModRow {
            name: "Broken Mod".into(),
            version: "1.0".into(),
            author: "Someone".into(),
            description: "A code mod Haiku can't compile.".into(),
            enabled: true,
            can_enable: true,
            notes: vec![crate::haiku_mods::Note { warning: true, text: crate::haiku_codemods::failure_note(&failure) }],
        });
        options_at(f, 6, Page::Main, 4);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // fanpatch: Team If's translation, and a Japanese code patch left out
    Scene { name: "mods-fanpatch", setup: |f, _, s| {
        f.env.mods.rows.extend(crate::haiku_fanpatch::sample_rows());
        options_at(f, 6, Page::Main, 4);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "mods-fanpatch-code", setup: |f, _, s| {
        f.env.mods.rows.extend(crate::haiku_fanpatch::sample_rows());
        options_at(f, 6, Page::Main, 5);
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "mods-download", setup: |f, _, s| {
        options_at(f, 6, Page::Main, 5);
        f.options.ask_download(super::codemods::Ask { mb: 96, setup: false });
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    // zigdl: the choice between Zig and the compiler that came with Haiku
    Scene { name: "mods-compiler-choice", setup: |f, _, s| {
        options_at(f, 6, Page::Main, 5);
        f.options.ask_download(super::codemods::Ask { mb: 96, setup: true });
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "mods-compiling", setup: |f, _, s| {
        options_at(f, 6, Page::Main, 5);
        f.env.mods.job = Some(super::codemods::JobView { title: "Preparing Gameplay Tweaks".into(), line: "Compiling... 2 of 3".into(), fraction: 0.72, hint: String::new() });
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "keyboard", setup: |_, v, _| {
        v.keyboard = Some(zakuro_core::services::keyboard::Request {
            hint: "Enter your name.".into(),
            text: "Corrin".into(),
            max_length: 8,
            buttons: vec!["Back".into(), "OK".into()],
            valid: zakuro_core::services::keyboard::Valid::NotEmptyNotBlank,
        });
    } },
    Scene { name: "keyboard-nintendo", setup: |_, v, s| {
        s.pc.glyphs = crate::settings::Glyphs::Nintendo;
        v.keyboard = Some(zakuro_core::services::keyboard::Request {
            hint: String::new(),
            text: String::new(),
            max_length: 8,
            buttons: vec!["OK".into()],
            valid: zakuro_core::services::keyboard::Valid::NotEmptyNotBlank,
        });
    } },
    Scene { name: "message", setup: |_, v, _| {
        v.message = Some("The 3D can't be drawn on the GPU, as Vulkan isn't working on this computer, so it's drawn in software. Updating the graphics driver usually fixes it.".into());
    } },
    Scene { name: "notices", setup: |_, v, s| {
        s.show_fps = true;
        v.fps = Some(60.0);
        v.toast = Some("Saved to slot 1".into());
        v.corner = Some("Chapter 3".into());
    } },
    // cheats: the Cheats tab, its four pages, its lists and a question
    Scene { name: "cheats-tab", setup: |f, _, _| options_at(f, 7, Page::Main, 0) },
    Scene { name: "cheats-units", setup: |f, _, _| options_at(f, 7, Page::Cheats(0), 5) },
    Scene { name: "cheats-units-more", setup: |f, _, s| {
        s.pc.cheats.ignore_caps = true;
        options_at(f, 7, Page::Cheats(0), 13);
    } },
    Scene { name: "cheats-battle", setup: |f, _, s| {
        s.pc.cheats.crit = 1;
        s.pc.cheats.always_hit = true;
        s.pc.cheats.exp_multiplier = 5;
        options_at(f, 7, Page::Cheats(1), 0);
    } },
    Scene { name: "cheats-items", setup: |f, _, _| options_at(f, 7, Page::Cheats(2), 0) },
    // pulse: Divine Pulse's rows, the Save States section and a slot's
    // dialog, and the rewind screen (as the run left it) and its question
    Scene { name: "options-pulse", setup: |f, _, s| {
        s.pc.divine_pulse = true;
        options_at(f, 4, Page::Main, 11);
    } },
    Scene { name: "cheats-states-group", setup: |f, _, _| options_at(f, 7, Page::Main, 4) },
    Scene { name: "cheats-states", setup: |f, _, s| {
        s.pc.save_states = true;
        options_at(f, 7, Page::Cheats(4), 2);
    } },
    Scene { name: "cheats-states-dialog", setup: |f, _, s| {
        s.pc.save_states = true;
        options_at(f, 7, Page::Cheats(4), 2);
        super::states::snapshot_menu(0);
    } },
    // statecheck: a slot made with another file layer (its "!" and card),
    // one made before Haiku checked, and why it can't be loaded: on the
    // page, and in the game's message box (quick load during play)
    Scene { name: "cheats-states-old", setup: |f, _, s| {
        s.pc.save_states = true;
        options_at(f, 7, Page::Cheats(4), 4);
    } },
    Scene { name: "cheats-states-refused", setup: |f, _, s| {
        s.pc.save_states = true;
        options_at(f, 7, Page::Cheats(4), 2);
        super::states::snapshot_info(0);
    } },
    Scene { name: "states-refused", setup: |_, v, _| {
        let first = crate::states::slots("", "").into_iter().flatten().find_map(|slot| slot.mismatch.and(slot.problem));
        v.message = Some(first.unwrap_or_else(|| crate::statecheck::message(&[crate::statecheck::Change::Off("Team If translation".into())])));
    } },
    Scene { name: "pulse", setup: |_, _, _| crate::pulse::set_asking(None) },
    Scene { name: "pulse-confirm", setup: |_, _, _| crate::pulse::set_asking(Some(true)) },
    // cheats2: the Battle page's last rows (Dragon Veins: Reuse, Capture Without a Prison)
    Scene { name: "cheats-battle-more", setup: |f, _, s| {
        s.pc.cheats.vein_reuse = true;
        s.pc.cheats.capture_recruit = true;
        options_at(f, 7, Page::Cheats(1), 9);
    } },
    Scene { name: "cheats-items-buildings", setup: |f, _, _| options_at(f, 7, Page::Cheats(2), 5) },
    // castlegifts: the Gameplay tab's Online Rewards row, on and off; the
    // Cheats tab's Add an Item, and its list on True Ganglari (the second
    // "Ganglari", noted Battle Bonus)
    Scene { name: "options-online-rewards", setup: |f, _, _| options_at(f, 4, Page::Main, 12) },
    Scene { name: "options-online-rewards-off", setup: |f, _, s| {
        s.pc.online_rewards = false;
        options_at(f, 4, Page::Main, 12);
    } },
    Scene { name: "cheats-items-give", setup: |f, _, _| options_at(f, 7, Page::Cheats(2), 1) },
    Scene { name: "cheats-item-list", setup: |f, _, _| {
        super::cheats::snapshot_pick(super::cheats::PickKind::Item);
        let row = crate::cheats::with_state(|s| s.items.iter().rposition(|(_, n)| n == "Ganglari").unwrap_or(0));
        options_at(f, 7, Page::Pick, row);
    } },
    Scene { name: "cheats-story", setup: |f, _, s| {
        s.pc.cheats.support_log = true;
        options_at(f, 7, Page::Cheats(3), 0);
    } },
    Scene { name: "cheats-class", setup: |f, _, _| {
        super::cheats::snapshot_pick(super::cheats::PickKind::Class);
        options_at(f, 7, Page::Pick, 20);
    } },
    Scene { name: "cheats-join", setup: |f, _, _| {
        super::cheats::snapshot_pick(super::cheats::PickKind::Character);
        options_at(f, 7, Page::Pick, 40);
    } },
    Scene { name: "cheats-skill", setup: |f, _, _| {
        super::cheats::snapshot_pick(super::cheats::PickKind::Skill);
        options_at(f, 7, Page::Pick, 10);
    } },
    Scene { name: "cheats-chapter", setup: |f, _, _| {
        super::cheats::snapshot_pick(super::cheats::PickKind::Chapter);
        options_at(f, 7, Page::Pick, 12);
    } },
    Scene { name: "cheats-chapter-confirm", setup: |f, _, _| {
        super::cheats::snapshot_pick(super::cheats::PickKind::Chapter);
        options_at(f, 7, Page::Pick, 12);
        super::cheats::snapshot_confirm("Go to Chapter 12: In Search of Mettle? The chapter starts now. Your story progress will change, so save to a NEW slot afterwards to keep your current save.");
    } },
    // online: the host's code over the game's wait (the run's own code when
    // it hosted), without a router mapping, the code box, a mistyped code,
    // connecting, and a refusal
    // updater: the Main Menu's notice (keyboard and Xbox prompts), the
    // notes, scrolled, Skip chosen, a release without a signature, the
    // download, the check, a failure with the download page, one without
    Scene { name: "update-notice", setup: |_, _, _| crate::haiku_update::snapshot_notice(Some(crate::haiku_update::snapshot_release())) },
    Scene { name: "update-notice-pad", setup: |_, _, s| {
        crate::haiku_update::snapshot_notice(Some(crate::haiku_update::snapshot_release()));
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "update-notes", setup: |f, _, _| f.update.snapshot(crate::haiku_update::snapshot_release(), "notes") },
    Scene { name: "update-notes-scrolled", setup: |f, _, _| f.update.snapshot(crate::haiku_update::snapshot_release(), "scrolled") },
    Scene { name: "update-notes-skip", setup: |f, _, _| f.update.snapshot(crate::haiku_update::snapshot_release(), "skip") },
    Scene { name: "update-unsigned", setup: |f, _, _| {
        let release = crate::haiku_update::Release { sig_url: String::new(), ..crate::haiku_update::snapshot_release() };
        f.update.snapshot(release, "notes");
    } },
    Scene { name: "update-downloading", setup: |f, _, s| {
        f.update.snapshot(crate::haiku_update::snapshot_release(), "downloading");
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "update-checking", setup: |f, _, _| f.update.snapshot(crate::haiku_update::snapshot_release(), "checking") },
    Scene { name: "update-failed", setup: |f, _, _| f.update.snapshot(crate::haiku_update::snapshot_release(), "failed") },
    Scene { name: "update-option", setup: |f, _, _| options_at(f, 5, Page::Main, 1) },
    Scene { name: "update-option-off", setup: |f, _, s| {
        s.pc.check_updates = false;
        options_at(f, 5, Page::Main, 1);
    } },
    Scene { name: "update-offline", setup: |f, _, _| f.update.snapshot(crate::haiku_update::snapshot_release(), "offline") },
    Scene { name: "online-host", setup: |_, v, s| {
        v.online = Some(crate::online_ui::host_panel().unwrap_or(crate::online_ui::HostPanel { code: "4HGJ-MZR0-000M-7JW6-ZBFG-YY1S".into(), mapped: true, copied: false, allow_cheats: false }));
        s.pc.glyphs = crate::settings::Glyphs::Keyboard;
    } },
    Scene { name: "online-host-pad", setup: |_, v, s| {
        v.online = Some(crate::online_ui::HostPanel { code: "4HGJ-MZR0-000M-7JW6-ZBFG-YY1S".into(), mapped: false, copied: false, allow_cheats: true });
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "online-join", setup: |f, v, s| {
        v.keyboard = Some(crate::online_ui::sample_request("4HGJ-MZR0-000M-7JW6"));
        f.keyboard.help = crate::online_ui::keyboard_help();
        s.pc.glyphs = crate::settings::Glyphs::Keyboard;
    } },
    Scene { name: "online-join-mistyped", setup: |f, v, s| {
        v.keyboard = Some(crate::online_ui::sample_request("4HGJ-MZR0-000M-7JW6-ZBFG-YY1T"));
        f.keyboard.help = Some("That code isn't right. Check it and try again.".into());
        s.pc.glyphs = crate::settings::Glyphs::Xbox;
    } },
    Scene { name: "online-connecting", setup: |_, v, _| {
        v.online_status = Some("Connecting to the host…".into());
    } },
    Scene { name: "online-version", setup: |_, v, _| {
        v.message = Some(crate::online_ui::failure_text(&zakuro_core::netplay::Failure::Version { host: "0.1.0+0.2.32".into(), ours: "0.0.9-test".into() }));
    } },
    Scene { name: "online-unreachable", setup: |_, v, _| {
        v.message = Some(crate::online_ui::failure_text(&zakuro_core::netplay::Failure::Unreachable));
    } },
    // online2: the gameplay mods differ (the run's own message when it was
    // refused), and the teams refused for a character changed by cheats
    Scene { name: "online-mods", setup: |_, v, _| {
        let sample = zakuro_core::netplay::ModsDiff { theirs: vec!["Test Gameplay Mod".into()], ..Default::default() };
        v.message = Some(crate::online_ui::last_message().filter(|text| text.contains("mods")).unwrap_or_else(|| crate::online_ui::failure_text(&zakuro_core::netplay::Failure::Mods(sample))));
    } },
    Scene { name: "online-cheated-teams", setup: |_, v, _| {
        v.message = Some(crate::online_ui::last_message().filter(|text| text.contains("cheats")).unwrap_or_else(|| format!("{} Your opponent's team has one, so the battle can't start.", crate::online_ui::CHEATS_NOT_ALLOWED)));
    } },
];

/// tones: the Gameplay tab on Speaking Style (after Kana's Gender: row 11),
/// Corrin's style shown as `style`.
fn tones_scene(f: &mut FatesUi, style: u8) {
    zakuro_core::tones::show(style);
    options_at(f, 4, Page::Main, 11);
}

/// writes each scene at each size into dir.
pub fn run(dir: &Path, system: &mut System) {
    let _ = std::fs::create_dir_all(dir);
    let top = system.read_screen_scaled(Screen::Top);
    let bottom = system.read_screen_scaled(Screen::Bottom);
    let sizes: Vec<[usize; 2]> = std::env::var("FATES_UI_SIZES")
        .ok()
        .map(|list| list.split(',').filter_map(|s| s.split_once('x')).filter_map(|(w, h)| Some([w.trim().parse().ok()?, h.trim().parse().ok()?])).collect())
        .unwrap_or_else(|| vec![[1920, 1080], [3440, 1440], [3840, 2160]]);
    let only = std::env::var("FATES_UI_SCENES").ok();
    // qol: the boon/bane panel as the game's memory has it now
    let chara = crate::qol::panel_now(&mut system.memory);
    // info-ui: the growth rates of the unit the lower screen shows now
    let growth = crate::info_ui::growth();
    for size in sizes {
        for scene in SCENES.iter().filter(|scene| only.as_deref().is_none_or(|only| only.split(',').any(|o| o == scene.name))) {
            let path = dir.join(format!("{}-{}x{}.png", scene.name, size[0], size[1]));
            let frame = picture(size, &top, &bottom, scene, &chara, &growth);
            frame.save(&path);
            println!("wrote {}", path.display());
        }
    }
}

fn picture(size: [usize; 2], top: &Shot, bottom: &Shot, scene: &Scene, chara: &Option<crate::qol::CharaPanel>, growth: &Option<crate::info_ui::Growth>) -> Frame {
    let ctx = egui::Context::default();
    let mut fates = FatesUi::default();
    let mut settings = Settings::default();
    let mut view = View { game: true, ..View::default() };
    fates.env = Env {
        monitors: vec!["Display 1 (3440\u{d7}1440)".into(), "Display 2 (2560\u{d7}1440)".into()],
        modes: vec![(1920, 1080, 60), (2560, 1080, 60), (3440, 1440, 60), (3440, 1440, 144), (3840, 2160, 60)],
        audio_devices: vec!["Speakers (Realtek(R) Audio)".into(), "Headphones (USB Audio)".into()],
        mods: super::mods::sample(), // mods
    };
    crate::haiku_import::snapshot_progress(scene.name == "mods-unpacking"); // hdpacks
    crate::haiku_update::snapshot_notice(None); // updater: a scene of its own sets one
    (scene.setup)(&mut fates, &mut view, &mut settings);
    // qol: the top screen where backdrop puts it, and the panel
    // (pulse: the rewind screen sits on it too)
    if scene.name == "boonbane" || scene.name.starts_with("pulse") {
        let (top_view, _) = zakuro_gpu::layout(size[0] as u32, size[1] as u32, Screens::LargeTop.screens(), zakuro_gpu::ScreenOrientation::North, top_width(top), false);
        view.top_screen = top_view.map(|v| Rect::from_min_size(Pos2::new(v.x, v.y), Vec2::new(v.width, v.height)));
        view.chara = chara.clone();
    }
    // info-ui: the lower screen where backdrop puts it, and the growth button
    if scene.name.starts_with("growth") {
        let (_, bottom_view) = zakuro_gpu::layout(size[0] as u32, size[1] as u32, Screens::LargeTop.screens(), zakuro_gpu::ScreenOrientation::North, top_width(top), false);
        view.bottom_screen = bottom_view.map(|v| Rect::from_min_size(Pos2::new(v.x, v.y), Vec2::new(v.width, v.height)));
        let rates = growth.as_ref().map_or([0; 8], |g| g.rates);
        let detailed = std::env::var("FATES_UI_GROWTH_DETAILED").is_ok_and(|v| v == "1");
        view.growth = Some(super::growth::Panel { rates, on: scene.name == "growth-view", hover: false, detailed });
        if scene.name == "growth-live" {
            view.growth = crate::info_ui::with_state(|state| state.panel());
        }
    }
    view.glyphs = crate::controls::GlyphSet::resolve(settings.pc.glyphs, crate::controls::Device::Keyboard);
    let mut textures: HashMap<TextureId, Texture> = HashMap::new();
    let mut primitives = Vec::new();
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(size[0] as f32, size[1] as f32));
    // a few frames, so the fades finish; the last one is pictured
    for step in 0..4 {
        let time = 0.1 + step as f64 * 0.25;
        let mut input = egui::RawInput { screen_rect: Some(screen), time: Some(time), ..Default::default() };
        if scene.name == "notices" {
            // the pointer moving shows the corner button
            input.events.push(egui::Event::PointerMoved(Pos2::new(size[0] as f32 * 0.5 + step as f32, size[1] as f32 * 0.5)));
        }
        // info-ui: FATES_UI_GROWTH_HOVER=1 points at the growth button
        if let (true, Some(bottom)) = (scene.name.starts_with("growth") && std::env::var("FATES_UI_GROWTH_HOVER").is_ok_and(|v| v == "1"), view.bottom_screen) {
            let unit = bottom.height() / 240.0;
            let (x, y, w, h) = super::growth::BUTTON;
            input.events.push(egui::Event::PointerMoved(Pos2::new(bottom.min.x + (x + w / 2.0) * unit, bottom.min.y + (y + h / 2.0) * unit)));
        }
        if scene.name.starts_with("options") {
            fates.options.show(0.0);
        }
        if scene.name.starts_with("mods") && !fates.options.open {
            fates.options.show(0.0); // mods
        }
        let output = ctx.run_ui(input, |ui| {
            fates.run(ui.ctx(), &view, &mut settings);
        });
        let mut output = output;
        for (id, delta) in std::mem::take(&mut output.textures_delta.set).into_iter().flat_map(|(id, deltas)| deltas.into_iter().map(move |d| (id, d))) {
            let egui::ImageData::Color(image) = &delta.image;
            match delta.pos {
                None => {
                    textures.insert(id, Texture { size: image.size, pixels: image.pixels.clone() });
                }
                Some([x, y]) => {
                    if let Some(texture) = textures.get_mut(&id) {
                        for row in 0..image.size[1] {
                            for col in 0..image.size[0] {
                                texture.pixels[(y + row) * texture.size[0] + x + col] = image.pixels[row * image.size[0] + col];
                            }
                        }
                    }
                }
            }
        }
        primitives = ctx.tessellate(std::mem::take(&mut output.shapes), output.pixels_per_point);
    }
    let mut frame = backdrop(size, top, bottom);
    for clipped in primitives {
        let Primitive::Mesh(mesh) = clipped.primitive else { continue };
        for triangle in mesh.indices.chunks_exact(3) {
            let v = [&mesh.vertices[triangle[0] as usize], &mesh.vertices[triangle[1] as usize], &mesh.vertices[triangle[2] as usize]];
            raster(&mut frame, &textures, clipped.clip_rect, mesh.texture_id, v);
        }
    }
    frame
}
