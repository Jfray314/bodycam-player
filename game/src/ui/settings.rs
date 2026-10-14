//! Player settings (the tablet's SETTINGS screens): persisted to a JSON config file and applied to the running game.
//! Keybinds are applied by remapping `ButtonInput<KeyCode>` before the gameplay systems read it (`remap`).
use bevy::audio::{GlobalVolume, Volume};
use bevy::camera::MainPassResolutionOverride;
use bevy::input::keyboard::KeyCode;
use bevy::prelude::*;
use bevy::render::view::ColorGrading;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, WindowMode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};

pub const RESOLUTIONS: [(u32, u32); 6] = [(1280, 720), (1600, 900), (1920, 1080), (2560, 1080), (2560, 1440), (3840, 2160)];
pub const WINDOW_MODES: [&str; 3] = ["Windowed", "Borderless", "Fullscreen"];

#[derive(Resource, Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    /// Multiplier on the base look sensitivity (player.rs SENS).
    pub sensitivity: f32,
    pub fov: f32,
    pub master: f32,
    pub sfx: f32,
    pub window_mode: usize,
    pub resolution: usize,
    pub res_scale: f32,
    pub vsync: bool,
    /// 0 = unlimited.
    pub fps_limit: u32,
    pub gamma: f32,
    pub lens_fx: bool,
    /// Action id -> key name (only changed binds are stored).
    pub binds: BTreeMap<String, String>,
    pub primary: String,
    pub secondary: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sensitivity: 1.0,
            fov: 120.0,
            master: 1.0,
            sfx: 1.0,
            window_mode: 0,
            resolution: 2,
            res_scale: 1.0,
            vsync: true,
            fps_limit: 120,
            gamma: 2.2,
            lens_fx: true,
            binds: BTreeMap::new(),
            primary: "M4A1".into(),
            secondary: "Glock17".into(),
        }
    }
}

pub fn path() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("BODYCAM_CONFIG") {
        return p.into();
    }
    let dir = std::env::var("APPDATA").map(|a| std::path::PathBuf::from(a).join("bodycam-player")).unwrap_or_default();
    let _ = std::fs::create_dir_all(&dir);
    dir.join("settings.json")
}

pub fn load() -> Settings {
    std::fs::read_to_string(path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn save(s: &Settings) {
    let _ = std::fs::write(path(), serde_json::to_string_pretty(s).unwrap_or_default());
}

static SENS: AtomicU32 = AtomicU32::new(0x3f800000); // 1.0f32
/// Look sensitivity multiplier (read by player.rs every frame).
pub fn sens_mult() -> f32 {
    f32::from_bits(SENS.load(Ordering::Relaxed))
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Set {
    WindowMode,
    Resolution,
    ResScale,
    VSync,
    FpsLimit,
    Gamma,
    LensFx,
    Master,
    Sfx,
    Sensitivity,
    Fov,
}

pub enum Kind {
    Slider { min: f32, max: f32, step: f32 },
    Select,
    Toggle,
}

pub struct Spec {
    pub label: &'static str,
    pub kind: Kind,
    pub help: &'static str,
}

pub fn spec(s: Set) -> Spec {
    use Kind::*;
    let (label, kind, help) = match s {
        Set::WindowMode => ("Window Mode", Select, "Windowed, borderless fullscreen or exclusive fullscreen."),
        Set::Resolution => ("Resolution", Select, "Window size in windowed mode. Fullscreen modes use the desktop resolution."),
        Set::ResScale => ("Resolution Scale", Slider { min: 0.5, max: 1.0, step: 0.05 }, "Renders the 3D view at a fraction of the window resolution."),
        Set::VSync => ("VSync", Toggle, "Waits for the display refresh. Disable it for the lowest input latency."),
        Set::FpsLimit => ("Framerate Limit", Slider { min: 30.0, max: 360.0, step: 10.0 }, "Caps the framerate. The top of the slider means unlimited."),
        Set::Gamma => ("Gamma", Slider { min: 1.6, max: 3.0, step: 0.1 }, "Display gamma. 2.2 is the standard value."),
        Set::LensFx => ("Bodycam Lens Effect", Toggle, "Lens distortion, chromatic aberration, disk blur, film grain and the camera exposure (F2)."),
        Set::Master => ("Master Volume", Slider { min: 0.0, max: 1.0, step: 0.05 }, "Volume of everything you hear."),
        Set::Sfx => ("SFX Volume", Slider { min: 0.0, max: 1.0, step: 0.05 }, "Volume of weapons, footsteps and impacts."),
        Set::Sensitivity => ("Mouse Sensitivity", Slider { min: 0.1, max: 3.0, step: 0.05 }, "Multiplier on mouse look. 1.00 matches the original game."),
        Set::Fov => ("Field of View", Slider { min: 90.0, max: 130.0, step: 1.0 }, "Horizontal field of view. The bodycam camera uses 120."),
    };
    Spec { label, kind, help }
}

impl Settings {
    /// Current value as a number (select = option index, toggle = 0/1).
    pub fn get(&self, s: Set) -> f32 {
        match s {
            Set::WindowMode => self.window_mode as f32,
            Set::Resolution => self.resolution as f32,
            Set::ResScale => self.res_scale,
            Set::VSync => self.vsync as u8 as f32,
            Set::FpsLimit => if self.fps_limit == 0 { 360.0 } else { self.fps_limit as f32 },
            Set::Gamma => self.gamma,
            Set::LensFx => self.lens_fx as u8 as f32,
            Set::Master => self.master,
            Set::Sfx => self.sfx,
            Set::Sensitivity => self.sensitivity,
            Set::Fov => self.fov,
        }
    }

    pub fn set(&mut self, s: Set, v: f32) {
        match s {
            Set::WindowMode => self.window_mode = (v as usize).min(WINDOW_MODES.len() - 1),
            Set::Resolution => self.resolution = (v as usize).min(RESOLUTIONS.len() - 1),
            Set::ResScale => self.res_scale = v,
            Set::VSync => self.vsync = v >= 0.5,
            Set::FpsLimit => self.fps_limit = if v >= 359.0 { 0 } else { v as u32 },
            Set::Gamma => self.gamma = v,
            Set::LensFx => self.lens_fx = v >= 0.5,
            Set::Master => self.master = v,
            Set::Sfx => self.sfx = v,
            Set::Sensitivity => self.sensitivity = v,
            Set::Fov => self.fov = v,
        }
    }

    pub fn reset(&mut self, s: Set) {
        let d = Settings::default();
        self.set(s, d.get(s));
    }

    pub fn text(&self, s: Set) -> String {
        match s {
            Set::WindowMode => WINDOW_MODES[self.window_mode].into(),
            Set::Resolution => format!("{}x{}", RESOLUTIONS[self.resolution].0, RESOLUTIONS[self.resolution].1),
            Set::ResScale => format!("{:.0}%", self.res_scale * 100.0),
            Set::VSync | Set::LensFx => if self.get(s) >= 0.5 { "On" } else { "Off" }.into(),
            Set::FpsLimit => if self.fps_limit == 0 { "Unlimited".into() } else { self.fps_limit.to_string() },
            Set::Gamma | Set::Sensitivity => format!("{:.2}", self.get(s)),
            Set::Master | Set::Sfx => format!("{:.0}%", self.get(s) * 100.0),
            Set::Fov => format!("{:.0}", self.fov),
        }
    }

    pub fn options(s: Set) -> usize {
        match s {
            Set::WindowMode => WINDOW_MODES.len(),
            Set::Resolution => RESOLUTIONS.len(),
            _ => 0,
        }
    }
}

/// (id, label, default key) of every rebindable keyboard action (IMC_KBM_CharacterOnFoot).
pub const ACTIONS: [(&str, &str, KeyCode); 16] = [
    ("forward", "Move Forward", KeyCode::KeyW),
    ("left", "Move Left", KeyCode::KeyA),
    ("back", "Move Back", KeyCode::KeyS),
    ("right", "Move Right", KeyCode::KeyD),
    ("sprint", "Sprint", KeyCode::ShiftLeft),
    ("crouch", "Crouch", KeyCode::ControlLeft),
    ("jump", "Jump", KeyCode::Space),
    ("lean_l", "Lean Left", KeyCode::KeyQ),
    ("lean_r", "Lean Right", KeyCode::KeyE),
    ("reload", "Reload / Check Ammo", KeyCode::KeyR),
    ("inspect", "Inspect Weapon", KeyCode::KeyJ),
    ("take", "Take", KeyCode::KeyF),
    ("drop", "Drop", KeyCode::KeyG),
    ("melee", "Melee", KeyCode::KeyX),
    ("flashlight", "Flashlight", KeyCode::KeyC),
    ("laser", "Laser", KeyCode::KeyZ),
];

/// Keys offered to the rebind prompt (letters, digits, common modifiers), with the names stored in the config.
pub fn key_table() -> Vec<(KeyCode, String)> {
    use KeyCode::*;
    let mut v: Vec<(KeyCode, String)> = vec![
        (KeyA, "A"), (KeyB, "B"), (KeyC, "C"), (KeyD, "D"), (KeyE, "E"), (KeyF, "F"), (KeyG, "G"), (KeyH, "H"), (KeyI, "I"), (KeyJ, "J"), (KeyK, "K"), (KeyL, "L"),
        (KeyM, "M"), (KeyN, "N"), (KeyO, "O"), (KeyP, "P"), (KeyQ, "Q"), (KeyR, "R"), (KeyS, "S"), (KeyT, "T"), (KeyU, "U"), (KeyV, "V"), (KeyW, "W"), (KeyX, "X"),
        (KeyY, "Y"), (KeyZ, "Z"), (Digit1, "1"), (Digit2, "2"), (Digit3, "3"), (Digit4, "4"), (Digit5, "5"), (Digit6, "6"), (Digit7, "7"), (Digit8, "8"), (Digit9, "9"),
        (Digit0, "0"), (Space, "Space"), (ShiftLeft, "Left Shift"), (ShiftRight, "Right Shift"), (ControlLeft, "Left Ctrl"), (ControlRight, "Right Ctrl"),
        (AltLeft, "Left Alt"), (CapsLock, "Caps Lock"), (ArrowUp, "Up"), (ArrowDown, "Down"), (ArrowLeft, "Left"), (ArrowRight, "Right"), (Backquote, "`"),
    ]
    .into_iter()
    .map(|(k, n)| (k, n.to_string()))
    .collect();
    v.shrink_to_fit();
    v
}

pub fn key_name(k: KeyCode) -> String {
    key_table().into_iter().find(|(c, _)| *c == k).map(|(_, n)| n).unwrap_or_else(|| format!("{k:?}"))
}

/// The key currently bound to an action id.
pub fn bound(s: &Settings, id: &str) -> KeyCode {
    let def = ACTIONS.iter().find(|a| a.0 == id).map(|a| a.2).unwrap_or(KeyCode::Escape);
    s.binds.get(id).and_then(|n| key_table().into_iter().find(|(_, kn)| kn == n)).map(|(k, _)| k).unwrap_or(def)
}

/// Pressing the custom key acts as the action's default key. Runs after input and before the game reads it.
// ponytail: a chain of swaps (A->B and B->A) works, longer cycles of overlapping custom keys do not matter in practice.
pub fn remap(s: &Settings, keys: &mut ButtonInput<KeyCode>) {
    let pairs: Vec<(KeyCode, KeyCode)> = ACTIONS.iter().map(|a| (a.2, bound(s, a.0))).filter(|(d, c)| d != c).collect();
    if pairs.is_empty() {
        return;
    }
    let orig = |keys: &ButtonInput<KeyCode>, k: KeyCode| (keys.pressed(k), keys.just_pressed(k));
    let mut new: Vec<(KeyCode, (bool, bool))> = pairs.iter().map(|&(d, c)| (d, orig(keys, c))).collect();
    for &(_, c) in &pairs {
        if !new.iter().any(|(k, _)| *k == c) {
            new.push((c, (false, false))); // a custom key that is nobody's default stops acting as itself
        }
    }
    for (k, (on, jp)) in new {
        keys.reset(k);
        if on {
            keys.press(k);
            if !jp {
                keys.clear_just_pressed(k);
            }
        }
    }
}

/// Applies the settings to window, audio, camera and the sensitivity global.
pub fn apply(
    s: Res<Settings>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
    mut vol: ResMut<GlobalVolume>,
    mut fx: ResMut<crate::postfx::PostFxState>,
    mut cmd: Commands,
    cams: Query<Entity, With<crate::player::BodycamCamera>>,
    mut last: Local<Option<Settings>>,
) {
    if last.as_ref() == Some(&*s) {
        return;
    }
    let first = last.is_none();
    if !first {
        save(&s);
    }
    let prev = last.replace(s.clone());
    SENS.store(s.sensitivity.to_bits(), Ordering::Relaxed);
    vol.volume = Volume::Linear(s.master * s.sfx);
    fx.base_fov_deg = s.fov;
    window.present_mode = if s.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
    // Window mode/size: leave the launch window alone until the player changes something.
    if prev.as_ref().is_none_or(|p| p.window_mode != s.window_mode || p.resolution != s.resolution) && !(first && s.window_mode == 0) {
        window.mode = match s.window_mode {
            1 => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            2 => WindowMode::Fullscreen(MonitorSelection::Current, bevy::window::VideoModeSelection::Current),
            _ => WindowMode::Windowed,
        };
        if s.window_mode == 0 {
            let (w, h) = RESOLUTIONS[s.resolution];
            window.resolution.set(w as f32, h as f32);
        }
    }
    for e in &cams {
        let mut grading = ColorGrading::default();
        grading.midtones.gamma = s.gamma / 2.2;
        cmd.entity(e).insert(grading);
    }
    if first && !s.lens_fx {
        fx.enabled = false;
    }
}

/// Resolution scale: render the 3D view at a fraction of the window.
pub fn res_scale(s: Res<Settings>, window: Single<&Window, With<PrimaryWindow>>, mut cmd: Commands, cams: Query<Entity, With<crate::player::BodycamCamera>>) {
    for e in &cams {
        if s.res_scale < 0.999 {
            let size = UVec2::new(window.physical_width(), window.physical_height());
            cmd.entity(e).insert(MainPassResolutionOverride((size.as_vec2() * s.res_scale).round().as_uvec2().max(UVec2::splat(64))));
        } else {
            cmd.entity(e).remove::<MainPassResolutionOverride>();
        }
    }
}

/// Framerate limiter (sleeps off the rest of the frame). With vsync on the display already paces below most limits.
pub fn limit_fps(s: Res<Settings>, mut last: Local<Option<std::time::Instant>>) {
    if s.fps_limit > 0 {
        let frame = std::time::Duration::from_secs_f64(1.0 / s.fps_limit as f64);
        if let Some(t) = *last {
            let spent = t.elapsed();
            if spent < frame {
                std::thread::sleep(frame - spent);
            }
        }
    }
    *last = Some(std::time::Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_remap() {
        let mut s = Settings::default();
        s.binds.insert("jump".into(), "V".into());
        s.fps_limit = 0;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back, s);
        assert_eq!(bound(&s, "jump"), KeyCode::KeyV);
        assert_eq!(bound(&s, "reload"), KeyCode::KeyR);
        let mut keys = ButtonInput::<KeyCode>::default();
        keys.press(KeyCode::KeyV);
        remap(&s, &mut keys);
        assert!(keys.pressed(KeyCode::Space) && keys.just_pressed(KeyCode::Space) && !keys.pressed(KeyCode::KeyV));
    }

    #[test]
    fn setters_clamp() {
        let mut s = Settings::default();
        s.set(Set::FpsLimit, 360.0);
        assert_eq!(s.fps_limit, 0);
        s.set(Set::WindowMode, 9.0);
        assert_eq!(s.window_mode, 2);
    }
}
