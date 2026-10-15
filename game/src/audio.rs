//! Player audio. SoundCues are evaluated exactly like the engine's cue graph (Modulator / Random / Mixer / Delay /
//! Attenuation / DistanceCrossFade / ParamCrossFade / Branch / Switch / WavePlayer), from `assets-local/audio-data/audio.json`
//! (baked by tools/audio-bake from the cooked assets). Playback goes through bevy_audio; wavs are decoded BINKA
//! (tools/binka) under `assets-local/audio/<path under Content, minus leading Audio/>.wav`.
//!
//! Source effect chains: SourceEffectChain_Bodycam_Filter_Test (EQ / HP / LP / convolution with the phone IR) has every
//! entry bypassed in the cooked asset, so it is an identity and nothing is applied. The gun chain SE_Compressor_Guns
//! (distortion + dynamics) is not implemented (see `GUN_CHAIN_GAP`).
use crate::player::{Footstep, Player, StepKind};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastVisibility};
use crate::status::NearMiss;
use crate::weapon::{DryFire, Fired, MontageStarted, ShellLanded};
use bevy::audio::{AudioSinkPlayback, Volume};
use bevy::prelude::*;
use rand::{Rng, SeedableRng};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{HashMap, HashSet, VecDeque};

/// ponytail: SE_Compressor_Guns (soft-knee 4:1 dynamics + distortion preset) skipped; add as a baked per-wave pass if gun loudness feels off.
#[allow(dead_code)]
const GUN_CHAIN_GAP: () = ();
/// Listener to muzzle, cm (camera to gun muzzle ~0.5 m).
const MUZZLE_DIST_CM: f32 = 50.0;
/// Listener to feet, cm.
const FOOT_DIST_CM: f32 = 100.0;

#[derive(Deserialize)]
struct Node {
    class: String,
    p: Value,
}

#[derive(Deserialize)]
struct Cue {
    first: Option<String>,
    volume: f32,
    pitch: f32,
    att: Option<String>,
    max_count: Option<usize>,
    stop_oldest: bool,
    nodes: HashMap<String, Node>,
}

#[derive(Deserialize)]
struct Baked {
    cues: HashMap<String, Option<Cue>>,
    att: HashMap<String, Value>,
    weapons: HashMap<String, HashMap<String, String>>,
    footsteps: HashMap<String, Option<String>>,
}

/// One wave to start: gain, pitch ratio and start delay (s) after evaluating a cue.
#[derive(Debug, Clone)]
pub struct VoiceSpec {
    pub wave: String,
    pub volume: f32,
    pub pitch: f32,
    pub delay: f32,
}

/// Cue parameters set by the game (SetBoolParameter etc.). Defaults describe the indoor test range.
#[derive(Default, Clone)]
pub struct Params {
    pub bools: HashMap<&'static str, bool>,
    pub ints: HashMap<&'static str, i32>,
    pub floats: HashMap<&'static str, f32>,
}

fn num(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}

fn children(p: &Value) -> Vec<Option<&str>> {
    p["ChildNodes"].as_array().map_or(vec![], |a| a.iter().map(Value::as_str).collect())
}

/// UE crossfade weight of a value against one input's fade-in / fade-out ranges.
fn fade(d: f32, i: &Value) -> f32 {
    let (a, b, c, e) = (num(i, "FadeInDistanceStart", 0.0), num(i, "FadeInDistanceEnd", 0.0), num(i, "FadeOutDistanceStart", 0.0), num(i, "FadeOutDistanceEnd", 0.0));
    let w = if d < a || d > e && e > c || d > e && d > c && e == c && d > e { 0.0 }
    else if d < b { if b > a { (d - a) / (b - a) } else { 1.0 } }
    else if d < c || e <= c { 1.0 }
    else { 1.0 - (d - c) / (e - c) };
    w.clamp(0.0, 1.0) * num(i, "Volume", 1.0)
}

/// Distance attenuation gain at `d` cm from the listener.
// ponytail: only the inner radius / falloff distance are honoured; every curve type uses linear (natural: inner/d). Exact at the <1 m distances played here.
fn att_gain(a: &Value, d: f32) -> f32 {
    if a.get("bAttenuate").and_then(Value::as_bool) == Some(false) {
        return 1.0;
    }
    let inner = a["AttenuationShapeExtents"]["x"].as_f64().map_or(400.0, |v| v as f32);
    let falloff = num(a, "FalloffDistance", 3600.0);
    if d <= inner {
        return 1.0;
    }
    let x = (d - inner) / falloff;
    if x >= 1.0 {
        return 0.0;
    }
    if a["DistanceAlgorithm"].as_str().is_some_and(|s| s.ends_with("NaturalSound")) { (inner / d) * (1.0 - x) } else { 1.0 - x }
}

#[derive(Resource)]
pub struct Bank {
    baked: Baked,
    /// Random nodes with "randomize without replacement": indices still unused, per (cue, node).
    unused: HashMap<(String, String), Vec<usize>>,
    pub env: Params,
}

impl Bank {
    pub fn load(root: &std::path::Path) -> Option<Self> {
        let s = std::fs::read_to_string(root.join("audio-data/audio.json")).ok()?;
        let baked: Baked = serde_json::from_str(&s).map_err(|e| error!("audio.json: {e}")).ok()?;
        let mut env = Params::default();
        env.bools.insert("Interior", true); // closed test range
        env.floats.insert("OrganiqueReverb", 0.3);
        env.floats.insert("AirPoints", 0.0);
        env.floats.insert("Elevation", 0.0);
        Some(Bank { baked, unused: HashMap::new(), env })
    }

    pub fn has(&self, cue: &str) -> bool {
        matches!(self.baked.cues.get(cue), Some(Some(_)))
    }

    /// Evaluate `cue` once as the engine would on PlaySound at `dist_cm` from the listener.
    pub fn evaluate(&mut self, cue: &str, params: &Params, dist_cm: f32, rng: &mut impl Rng) -> Vec<VoiceSpec> {
        let Some(Some(c)) = self.baked.cues.get(cue) else { return vec![] };
        let mut vol = c.volume;
        if let Some(a) = c.att.as_ref().and_then(|a| self.baked.att.get(a)) {
            vol *= att_gain(a, dist_cm);
        }
        let mut out = vec![];
        let mut cx = Ctx { cue, c, att: &self.baked.att, params, env: &self.env, dist: dist_cm, unused: &mut self.unused, rng, out: &mut out };
        cx.walk(c.first.as_deref(), vol, c.pitch, 0.0);
        out
    }

    /// Every wave a cue can play (for preloading).
    pub fn waves_of(&self, cue: &str) -> HashSet<String> {
        let mut s = HashSet::new();
        if let Some(Some(c)) = self.baked.cues.get(cue) {
            for n in c.nodes.values() {
                if let Some(w) = n.p["SoundWaveAssetPtr"].as_str() {
                    s.insert(w.to_string());
                }
            }
        }
        s
    }

    pub fn weapon_cue(&self, weapon: &str, kind: &str) -> Option<&str> {
        let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        let w = key(weapon);
        self.baked.weapons.iter().find(|(r, _)| key(r) == w)?.1.get(kind).map(String::as_str)
    }

    fn footstep_cue(&self, surface: &str) -> Option<&str> {
        self.baked.footsteps.get(surface).or(self.baked.footsteps.get("SurfaceType_Default"))?.as_deref()
    }
}

struct Ctx<'a, R: Rng> {
    cue: &'a str,
    c: &'a Cue,
    att: &'a HashMap<String, Value>,
    params: &'a Params,
    env: &'a Params,
    dist: f32,
    unused: &'a mut HashMap<(String, String), Vec<usize>>,
    rng: &'a mut R,
    out: &'a mut Vec<VoiceSpec>,
}

impl<R: Rng> Ctx<'_, R> {
    fn bool(&self, k: &str) -> Option<bool> {
        self.params.bools.iter().chain(self.env.bools.iter()).find(|(n, _)| **n == k).map(|(_, v)| *v)
    }
    fn float(&self, k: &str) -> Option<f32> {
        self.params.floats.iter().chain(self.env.floats.iter()).find(|(n, _)| **n == k).map(|(_, v)| *v)
    }
    fn int(&self, k: &str) -> Option<i32> {
        self.params.ints.iter().chain(self.env.ints.iter()).find(|(n, _)| **n == k).map(|(_, v)| *v)
    }
    fn range(&mut self, a: f32, b: f32) -> f32 {
        if b > a { self.rng.random_range(a..=b) } else { a }
    }

    fn walk(&mut self, name: Option<&str>, vol: f32, pitch: f32, delay: f32) {
        let Some(name) = name else { return };
        let Some(n) = self.c.nodes.get(name) else { return };
        let (p, kids) = (&n.p, children(&n.p));
        match n.class.as_str() {
            "SoundNodeWavePlayer" => {
                if let Some(w) = p["SoundWaveAssetPtr"].as_str() {
                    self.out.push(VoiceSpec { wave: w.into(), volume: vol, pitch, delay });
                }
            }
            "SoundNodeModulator" => {
                let v = self.range(num(p, "VolumeMin", 0.95), num(p, "VolumeMax", 1.05));
                let s = self.range(num(p, "PitchMin", 0.95), num(p, "PitchMax", 1.05));
                self.walk(kids.first().copied().flatten(), vol * v, pitch * s, delay);
            }
            "SoundNodeDelay" => {
                let d = self.range(num(p, "DelayMin", 0.0), num(p, "DelayMax", 0.0));
                self.walk(kids.first().copied().flatten(), vol, pitch, delay + d);
            }
            "SoundNodeMixer" => {
                let iv = p["InputVolume"].as_array();
                for (i, k) in kids.iter().enumerate() {
                    let g = iv.and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(1.0) as f32;
                    self.walk(*k, vol * g, pitch, delay);
                }
            }
            "SoundNodeRandom" => {
                let w: Vec<f32> = (0..kids.len()).map(|i| p["Weights"].as_array().and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(1.0) as f32).collect();
                let key = (self.cue.to_string(), name.to_string());
                let pool: Vec<usize> = if p["bRandomizeWithoutReplacement"].as_bool() == Some(true) {
                    let u = self.unused.entry(key.clone()).or_default();
                    if u.is_empty() {
                        *u = (0..kids.len()).collect();
                    }
                    u.clone()
                } else {
                    (0..kids.len()).collect()
                };
                let total: f32 = pool.iter().map(|&i| w[i]).sum();
                let mut r = self.rng.random_range(0.0..total.max(f32::MIN_POSITIVE));
                let mut pick = *pool.last().unwrap_or(&0);
                for &i in &pool {
                    if r < w[i] {
                        pick = i;
                        break;
                    }
                    r -= w[i];
                }
                if let Some(u) = self.unused.get_mut(&key) {
                    u.retain(|&i| i != pick);
                }
                self.walk(kids.get(pick).copied().flatten(), vol, pitch, delay);
            }
            "SoundNodeAttenuation" => {
                let g = p["AttenuationSettings"].as_str().and_then(|a| self.att.get(a)).map_or(1.0, |a| att_gain(a, self.dist));
                self.walk(kids.first().copied().flatten(), vol * g, pitch, delay);
            }
            "SoundNodeDistanceCrossFade" => self.crossfade(p, &kids, self.dist, vol, pitch, delay),
            "SoundNodeParamCrossFade" => {
                let v = self.float(p["ParamName"].as_str().unwrap_or("")).unwrap_or(0.0);
                self.crossfade(p, &kids, v, vol, pitch, delay)
            }
            "SoundNodeBranch" => {
                let i = match self.bool(p["BoolParameterName"].as_str().unwrap_or("")) {
                    Some(true) => 0,
                    Some(false) => 1,
                    None => 2,
                };
                self.walk(kids.get(i).copied().flatten(), vol, pitch, delay);
            }
            "SoundNodeSwitch" => {
                let i = self.int(p["IntParameterName"].as_str().unwrap_or("")).unwrap_or(0).max(0) as usize;
                self.walk(kids.get(i).or(kids.first()).copied().flatten(), vol, pitch, delay);
            }
            // ponytail: Looping plays its child once (only the wind / cloth loop cues use it).
            "SoundNodeLooping" => self.walk(kids.first().copied().flatten(), vol, pitch, delay),
            other => warn_once_unknown(other),
        }
    }

    fn crossfade(&mut self, p: &Value, kids: &[Option<&str>], d: f32, vol: f32, pitch: f32, delay: f32) {
        let inputs = p["CrossFadeInput"].as_array().cloned().unwrap_or_default();
        for (i, k) in kids.iter().enumerate() {
            let w = inputs.get(i).map_or(1.0, |inp| fade(d, inp));
            if w > 0.0 {
                self.walk(*k, vol * w, pitch, delay);
            }
        }
    }
}

fn warn_once_unknown(c: &str) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut s = SEEN.lock().unwrap();
    if !s.iter().any(|x| x == c) {
        warn!("audio: unsupported sound node {c}");
        s.push(c.into());
    }
}

/// `/Game/Audio/X` -> `audio/X.wav`, `/Game/Y` -> `audio/Y.wav`, `/Plugin/Z` -> `audio/Plugin/Z.wav` (matches tools/audio-bake).
pub fn wav_path(pkg: &str) -> String {
    let pkg = pkg.split('.').next().unwrap();
    let p = pkg.strip_prefix("/Game/").map(|r| r.strip_prefix("Audio/").unwrap_or(r)).unwrap_or(&pkg[1..]);
    format!("audio/{p}.wav")
}

// ---------- events ----------

/// Play a SoundCue (or a bare SoundWave package) once: items.rs / perks.rs (grenade blast, knife hit, drone crash...).
/// `dist_cm` feeds the cue's attenuation/crossfade ranges (0 = at the listener).
#[derive(Message, Clone)]
pub struct PlayCue {
    pub cue: String,
    pub dist_cm: f32,
}

fn play_cues(mut cmd: Commands, mut ev: MessageReader<PlayCue>, mut bank: ResMut<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>, alive: Query<Entity>) {
    for e in ev.read() {
        if bank.has(&e.cue) {
            trigger(&mut cmd, &mut bank, &mut live, &assets, &alive, &e.cue, &Params::default(), e.dist_cm);
        } else {
            let handle = load_wave(&mut live, &assets, &e.cue);
            cmd.spawn(Pending { wait: 0.0, handle, volume: 1.0, pitch: 1.0 });
        }
    }
}

// ---------- playback ----------

#[derive(Component)]
struct Pending {
    wait: f32,
    handle: Handle<AudioSource>,
    volume: f32,
    pitch: f32,
}

#[derive(Resource, Default)]
struct Live {
    groups: HashMap<String, VecDeque<Vec<Entity>>>,
    handles: HashMap<String, Handle<AudioSource>>,
    preloaded: HashSet<String>,
    aiming: bool,
    /// Notify sounds waiting for their time: (seconds to go, package path).
    queue: Vec<(f32, String)>,
}

pub struct AudioPlugin;
impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        let root = std::env::var("BODYCAM_ASSETS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../assets-local").into());
        match Bank::load(std::path::Path::new(&root)) {
            Some(b) => {
                app.insert_resource(b);
            }
            None => {
                warn!("audio: {root}/audio-data/audio.json missing (run tools/audio-bake); audio disabled");
                return;
            }
        }
        app.init_resource::<Live>()
            .insert_resource(Tremor(0.0))
            .init_resource::<Loops>()
            .add_systems(PostStartup, preload_player)
            .add_systems(Update, (footsteps, weapon_sfx, remote_sfx, near_miss_sfx, shell_sfx, montage_sfx, play_cues, run_queue, aim_sfx, start_pending, preload_weapons, loops, selftest).chain());
    }
}

/// BODYCAM_AUDIO_SELFTEST=1: emit steps and dry-fires without input so a headless run exercises the cues.
fn selftest(time: Res<Time>, mut steps: MessageWriter<Footstep>, mut dry: MessageWriter<DryFire>, mut n: Local<u32>) {
    if std::env::var_os("BODYCAM_AUDIO_SELFTEST").is_none() {
        return;
    }
    let k = (time.elapsed_secs() * 2.0) as u32;
    if k > *n && k > 4 {
        *n = k;
        match k % 4 {
            0 => { dry.write(DryFire { row: "M4A1".into() }); }
            1 => { steps.write(Footstep { kind: StepKind::Walk, stride_start: true }); }
            2 => { steps.write(Footstep { kind: StepKind::Run, stride_start: false }); }
            _ => { steps.write(Footstep { kind: StepKind::Land, stride_start: false }); }
        }
    }
}

fn load_wave(live: &mut Live, assets: &AssetServer, wave: &str) -> Handle<AudioSource> {
    live.handles.entry(wave.into()).or_insert_with(|| assets.load(wav_path(wave))).clone()
}

/// Trigger a cue: evaluate, enforce its concurrency (StopOldest), queue the voices.
fn trigger(cmd: &mut Commands, bank: &mut Bank, live: &mut Live, assets: &AssetServer, alive: &Query<Entity>, cue: &str, params: &Params, dist: f32) {
    let voices = bank.evaluate(cue, params, dist, &mut rand::rng());
    if voices.is_empty() {
        return;
    }
    let (max, stop_oldest) = match bank.baked.cues.get(cue) {
        Some(Some(c)) => (c.max_count, c.stop_oldest),
        _ => (None, false),
    };
    let g = live.groups.entry(cue.into()).or_default();
    g.retain(|v| v.iter().any(|e| alive.get(*e).is_ok()));
    if let Some(m) = max
        && stop_oldest
        && g.len() >= m
        && let Some(old) = g.pop_front()
    {
        for e in old {
            if let Ok(mut ec) = cmd.get_entity(e) {
                ec.despawn();
            }
        }
    }
    let mut ids = vec![];
    for v in &voices {
        let handle = load_wave(live, assets, &v.wave);
        ids.push(cmd.spawn(Pending { wait: v.delay, handle, volume: v.volume * params.floats.get("PlayGain").copied().unwrap_or(1.0), pitch: v.pitch }).id());
    }
    live.groups.entry(cue.into()).or_default().push_back(ids);
    info!("audio: {cue} -> {} voice(s) [{}]", voices.len(), voices.iter().take(3).map(|v| format!("{} v{:.2} p{:.2} d{:.2}", v.wave.rsplit('/').next().unwrap(), v.volume, v.pitch, v.delay)).collect::<Vec<_>>().join(", "));
}

fn start_pending(mut cmd: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Pending)>) {
    for (e, mut p) in &mut q {
        p.wait -= time.delta_secs();
        if p.wait <= 0.0 {
            cmd.entity(e).remove::<Pending>().insert((AudioPlayer::<AudioSource>(p.handle.clone()), PlaybackSettings::DESPAWN.with_volume(Volume::Linear(p.volume)).with_speed(p.pitch)));
        }
    }
}

const FOLEY: [(&str, &str); 4] = [
    ("/Game/Audio/Player/Foley/Armor/Foley_Cloth_Move_Gear_", "step"),
    ("/Game/Audio/Player/Foley/Legs/Foley_Cloth_Move_Leg_", "step"),
    ("/Game/Audio/Player/Foley/Arms/Foley_Cloth_Move_Arms_", "stride"),
    ("/Game/Audio/Player/Foley/Bodycam/Foley_Bodycam_", "stride"),
];

/// Original steps come from animation notifies; here from the gait phase (player.rs `Footstep`).
fn footsteps(mut cmd: Commands, mut ev: MessageReader<Footstep>, mut bank: ResMut<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>, alive: Query<Entity>, mut ray: MeshRayCast, player: Query<&GlobalTransform, With<Player>>, surf: Query<&crate::weapon::SurfaceMat>) {
    for f in ev.read() {
        let mut params = Params::default();
        params.ints.insert("FootstepType", f.kind as i32);
        params.bools.insert("Turning?", false);
        // Surface under the feet (ALS: line trace down, PhysMat SurfaceType -> DT_Footstep_Player row); unmarked = Default.
        let under = player.iter().next().and_then(|p| {
            ray.cast_ray(Ray3d::new(p.translation() + Vec3::Y * 0.2, Dir3::NEG_Y), &MeshRayCastSettings::default().with_filter(&|e| surf.contains(e)).with_visibility(RayCastVisibility::Any))
                .first()
                .and_then(|(e, h)| (h.distance < 1.0).then(|| surf.get(*e).ok().map(|s| s.0)).flatten())
        });
        let key = under.map_or("SurfaceType_Default".to_string(), |n| format!("SurfaceType{n}"));
        if let Some(c) = bank.footstep_cue(&key).map(str::to_string) {
            trigger(&mut cmd, &mut bank, &mut live, &assets, &alive, &c, &params, FOOT_DIST_CM);
        }
        if f.kind == StepKind::Land {
            continue;
        }
        let gait = match f.kind {
            StepKind::Crouch => "Crouch",
            StepKind::Walk => "Walk",
            _ => "Run",
        };
        for (prefix, when) in FOLEY {
            if when == "step" || f.stride_start {
                trigger(&mut cmd, &mut bank, &mut live, &assets, &alive, &format!("{prefix}{gait}"), &Params::default(), 50.0);
            }
        }
    }
}

/// Another player's weapon sound (net.rs): `kind` Fire / FireSuppressed, attenuated by `dist_cm`.
#[derive(Message, Clone)]
pub struct RemoteSfx {
    pub row: String,
    pub kind: &'static str,
    pub dist_cm: f32,
}

fn remote_sfx(mut cmd: Commands, mut ev: MessageReader<RemoteSfx>, mut bank: ResMut<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>, alive: Query<Entity>) {
    for e in ev.read() {
        if let Some(c) = bank.weapon_cue(&e.row, e.kind).map(str::to_string) {
            trigger(&mut cmd, &mut bank, &mut live, &assets, &alive, &c, &Params::default(), e.dist_cm);
        }
    }
}

fn weapon_sfx(mut cmd: Commands, mut fired: MessageReader<Fired>, mut dry: MessageReader<DryFire>, mut bank: ResMut<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>, alive: Query<Entity>) {
    let shots = fired.read().map(|f| (f.row.clone(), if f.suppressed { "FireSuppressed" } else { "Fire" })).chain(dry.read().map(|d| (d.row.clone(), "FireDry")));
    for (row, kind) in shots.collect::<Vec<_>>() {
        if let Some(c) = bank.weapon_cue(&row, kind).map(str::to_string) {
            trigger(&mut cmd, &mut bank, &mut live, &assets, &alive, &c, &Params::default(), MUZZLE_DIST_CM);
        }
    }
}

/// AC_BodycamCamera.GettingShot: the shooter weapon's BulletCrack at the muzzle and BulletWizz at the closest point, both volume 3.
fn near_miss_sfx(mut cmd: Commands, mut ev: MessageReader<NearMiss>, mut bank: ResMut<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>, alive: Query<Entity>) {
    let mut params = Params::default();
    params.floats.insert("PlayGain", 3.0);
    for m in ev.read() {
        for (kind, dist) in [("BulletCrack", m.origin_dist_cm), ("BulletWizz", m.closest_dist_cm)] {
            if let Some(c) = bank.weapon_cue(&m.row, kind).map(str::to_string) {
                trigger(&mut cmd, &mut bank, &mut live, &assets, &alive, &c, &params, dist);
            }
        }
    }
}

/// SurfaceType switch: the shell floor carries no PhysMat, so index 0 (Default -> cement).
fn shell_sfx(mut ev: MessageReader<ShellLanded>, mut live: ResMut<Live>) {
    for _ in ev.read() {
        live.queue.push((0.0, "/Game/BodycamWeapons/Core/Blueprint/Emitters/Particles/Emitters/Shell/Sounds_Shell/Shell_Pistol_Cue".into()));
    }
}

/// Montage `PlayWepSounds_C` notifies: a cue is evaluated, a bare SoundWave plays as is, both at the notify time.
fn montage_sfx(mut ev: MessageReader<MontageStarted>, mut live: ResMut<Live>) {
    for m in ev.read() {
        for (t, name, props) in &m.montage.notifies {
            if name == "PlayWepSounds_C" && let Some(s) = props["Sound"].as_str() {
                live.queue.push((*t, s.split('.').next().unwrap().to_string()));
            }
        }
    }
}

/// Fires queued notify sounds once their time has come (the queue holds time-to-go).
fn run_queue(mut cmd: Commands, time: Res<Time>, mut bank: ResMut<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>, alive: Query<Entity>) {
    let dt = time.delta_secs();
    let q = std::mem::take(&mut live.queue);
    for (t, s) in q {
        if t > dt {
            live.queue.push((t - dt, s));
        } else if bank.has(&s) {
            trigger(&mut cmd, &mut bank, &mut live, &assets, &alive, &s, &Params::default(), MUZZLE_DIST_CM);
        } else {
            let handle = load_wave(&mut live, &assets, &s);
            cmd.spawn(Pending { wait: 0.0, handle, volume: 1.0, pitch: 1.0 });
            info!("audio: wave {s}");
        }
    }
}

fn aim_sfx(mut cmd: Commands, p: Single<&Player>, mut bank: ResMut<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>, alive: Query<Entity>) {
    if p.aiming != live.aiming {
        live.aiming = p.aiming;
        let c = if p.aiming { "/Game/Ultimate_SFX/ClothesSounds/Aim_Sounds" } else { "/Game/Ultimate_SFX/ClothesSounds/UnAim_Sounds" };
        trigger(&mut cmd, &mut bank, &mut live, &assets, &alive, c, &Params::default(), 50.0);
    }
}

// ---------- AC_BodycamCamera persistent sounds (UpdateWindCameraSounds / UpdateFoleySounds / UpdateBreathingSounds / UpdateTremorSounds) ----------

/// AC_BodycamCamera.Tremor: set by hits / explosions, decays with FInterpTo(.., 0, dt, 3).
#[derive(Resource)]
pub struct Tremor(pub f32);

fn map_clamped(v: f32, a: f32, b: f32, c: f32, d: f32) -> f32 {
    c + (d - c) * ((v - a) / (b - a)).clamp(0.0, 1.0)
}

use crate::status::{finterp_to as finterp, SurvivorStatus};

/// A cue played as an AudioComponent: re-evaluated every frame with its live parameters and a fixed random seed (so Random
/// nodes keep their pick), every wave looping; waves whose crossfade weight is 0 stay running at zero volume.
#[derive(Default)]
struct Cont {
    ents: HashMap<String, Entity>,
}

impl Cont {
    fn update(&mut self, cmd: &mut Commands, bank: &mut Bank, assets: &AssetServer, sinks: &mut Query<&mut AudioSink>, cue: &str, params: &Params, mult: f32) {
        let mut want: HashMap<String, (f32, f32)> = HashMap::new();
        for v in bank.evaluate(cue, params, 0.0, &mut rand::rngs::StdRng::seed_from_u64(7)) {
            let e = want.entry(v.wave).or_insert((0.0, v.pitch));
            e.0 += v.volume * mult;
        }
        for (wave, (vol, pitch)) in &want {
            match self.ents.get(wave).and_then(|e| sinks.get_mut(*e).ok()) {
                Some(mut sink) => sink.set_volume(Volume::Linear(*vol)),
                None if self.ents.contains_key(wave) => {} // sink not created yet
                None => {
                    // Mute-start waves that are not audible yet but may become so.
                    let id = cmd.spawn((AudioPlayer::<AudioSource>(assets.load(wav_path(wave))), PlaybackSettings::LOOP.with_volume(Volume::Linear(*vol)).with_speed(*pitch))).id();
                    self.ents.insert(wave.clone(), id);
                }
            }
        }
        for (wave, e) in &self.ents {
            if !want.contains_key(wave) && let Ok(mut sink) = sinks.get_mut(*e) {
                sink.set_volume(Volume::Linear(0.0));
            }
        }
    }
}

#[derive(Resource)]
struct Loops {
    wind: Cont,
    foley: Cont,
    breath: Cont,
    tremor: Cont,
    foley_vol: f32,
    wind_vol: f32,
    tremor_vol: f32,
    high_rotation: f32,
    last_look: Option<Vec2>,
}
impl Default for Loops {
    fn default() -> Self {
        Loops { wind: Cont::default(), foley: Cont::default(), breath: Cont::default(), tremor: Cont::default(), foley_vol: 0.0, wind_vol: 0.0, tremor_vol: 1.0, high_rotation: 0.0, last_look: None }
    }
}

#[allow(clippy::too_many_arguments)]
fn loops(
    mut cmd: Commands,
    time: Res<Time>,
    p: Single<&Player>,
    cam: Single<&GlobalTransform, With<crate::player::BodycamCamera>>,
    mut ray: bevy::picking::mesh_picking::ray_cast::MeshRayCast,
    mut bank: ResMut<Bank>,
    mut st: ResMut<Loops>,
    vit: Res<SurvivorStatus>,
    mut tremor: ResMut<Tremor>,
    assets: Res<AssetServer>,
    mut sinks: Query<&mut AudioSink>,
) {
    let dt = time.delta_secs();
    // Yaw/Pitch(DeltaTime): per-frame control-rotation change in degrees.
    let look = Vec2::new(p.yaw, p.pitch);
    let d = st.last_look.map_or(Vec2::ZERO, |l| (look - l).abs() * (180.0 / std::f32::consts::PI));
    st.last_look = Some(look);
    let (dy, dp) = (d.x, d.y);
    let speed_cm = Vec2::new(p.vel.x, p.vel.z).length() * 100.0;

    // UpdateHighRotation
    st.high_rotation = finterp(st.high_rotation, dy + dp, dt, 5.0);

    // Wind: 5 m trace up decides the interior factor (WindInteriorMultiply 0.5).
    let up = ray.cast_ray(Ray3d::new(cam.translation(), Dir3::Y), &default()).first().map(|(_, h)| h.distance * 100.0).filter(|d| *d <= 500.0);
    let interior = map_clamped(up.unwrap_or(500.0), 200.0, 500.0, 0.5, 1.0);
    let wind_target = map_clamped(dy + dp + map_clamped(speed_cm, 150.0, 300.0, 0.1, 3.0), 0.0, 3.0, 0.0, interior);
    st.wind_vol = finterp(st.wind_vol, wind_target, dt, 10.0);
    let mut wp = Params::default();
    wp.bools.insert("highSpeed", speed_cm > 150.0);
    wp.floats.insert("highRotation", st.high_rotation);
    let wind_vol = st.wind_vol;
    st.wind.update(&mut cmd, &mut bank, &assets, &mut sinks, "/Game/Audio/Player/Foley/Bodycam/Wind_Bodycam", &wp, wind_vol);

    // Foley (Bodycam cloth loop)
    st.foley_vol = finterp(st.foley_vol, map_clamped(dy + dp, 0.0, 3.0, 0.0, 1.0), dt, 10.0);
    let mut fp = Params::default();
    fp.floats.insert("highRotation", st.high_rotation);
    let foley_vol = st.foley_vol;
    st.foley.update(&mut cmd, &mut bank, &assets, &mut sinks, "/Game/Audio/Player/Foley/Bodycam/Bodycam_Foley_Player_Bodycam_Cloth_loop_test_Cue", &fp, foley_vol);

    // Breathing (UpdateBreathingSounds)
    let speed = (vit.stress + map_clamped(vit.health, 25.0, 100.0, 0.8, 0.0) + map_clamped(vit.stamina, 0.0, 1.0, 0.0, 1.0)).clamp(0.0, 1.0);
    let mut bp = Params::default();
    bp.floats.insert("Speed", speed);
    bp.bools.insert("Running?", speed_cm > 150.0);
    st.breath.update(&mut cmd, &mut bank, &assets, &mut sinks, "/Game/Ultimate_SFX/Repiration/Breathing_cue", &bp, 1.0);

    // Mic tremor
    st.tremor_vol = finterp(st.tremor_vol, tremor.0, dt, 10.0);
    tremor.0 = finterp(tremor.0, 0.0, dt, 3.0);
    let tv = st.tremor_vol;
    st.tremor.update(&mut cmd, &mut bank, &assets, &mut sinks, "/Game/Audio/Player/Foley/Bodycam/MicTremor_Bodycam", &Params::default(), tv);
}

/// Preload what the first seconds need: default-surface footsteps, foley, aim, breathing.
fn preload_player(bank: Res<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>) {
    let mut cues: Vec<String> = bank.footstep_cue("SurfaceType_Default").map(str::to_string).into_iter().collect();
    for gait in ["Walk", "Run", "Crouch"] {
        cues.extend(FOLEY.iter().map(|(p, _)| format!("{p}{gait}")));
    }
    cues.push("/Game/Ultimate_SFX/ClothesSounds/Aim_Sounds".into());
    cues.push("/Game/Ultimate_SFX/ClothesSounds/UnAim_Sounds".into());
    for c in &cues {
        for w in bank.waves_of(c) {
            load_wave(&mut live, &assets, &w);
        }
    }
}

/// Preload the fire / dry-fire waves of the weapon in hand the first time it is equipped.
fn preload_weapons(inv: Option<Single<&crate::weapon::Inventory>>, bank: Res<Bank>, mut live: ResMut<Live>, assets: Res<AssetServer>) {
    let Some(inv) = inv else { return };
    let w = &inv.slots[inv.current].name;
    if !live.preloaded.insert(w.clone()) {
        return;
    }
    for kind in ["Fire", "FireDry"] {
        if let Some(c) = bank.weapon_cue(w, kind) {
            for wave in bank.waves_of(c) {
                load_wave(&mut live, &assets, &wave);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn m4_cue_evaluates() {
        let root = std::env::var("BODYCAM_ASSETS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../assets-local").into());
        let Some(mut bank) = Bank::load(std::path::Path::new(&root)) else {
            eprintln!("audio.json not baked; skipping");
            return;
        };
        let cue = bank.weapon_cue("M4A1", "Fire").expect("M4A1 fire cue").to_string();
        assert!(bank.has(&cue));
        let v = bank.evaluate(&cue, &Params::default(), MUZZLE_DIST_CM, &mut rand::rng());
        assert!(!v.is_empty());
        for s in &v {
            assert!(s.volume > 0.0 && s.volume < 20.0 && s.pitch > 0.5 && s.pitch < 2.0 && s.delay >= 0.0, "{s:?}");
            assert!(s.wave.starts_with("/Game/"));
        }
        // mechanical + close crack present at 50 cm
        assert!(v.iter().any(|s| s.wave.contains("m4_Close") || s.wave.contains("Close")));
    }

    #[test]
    fn near_miss_cues_evaluate() {
        let root = std::env::var("BODYCAM_ASSETS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../assets-local").into());
        let Some(mut bank) = Bank::load(std::path::Path::new(&root)) else { return };
        for kind in ["BulletCrack", "BulletWizz"] {
            let cue = bank.weapon_cue("M4A1", kind).expect(kind).to_string();
            assert!(!bank.evaluate(&cue, &Params::default(), 300.0, &mut rand::rng()).is_empty(), "{kind}");
        }
    }

    #[test]
    fn crossfade_ranges() {
        let i = serde_json::json!({"FadeInDistanceStart":0.0,"FadeInDistanceEnd":1.0,"FadeOutDistanceStart":2000.0,"FadeOutDistanceEnd":3500.0,"Volume":1.0});
        assert_eq!(fade(50.0, &i), 1.0);
        assert!(fade(2750.0, &i) > 0.4 && fade(2750.0, &i) < 0.6);
        assert_eq!(fade(4000.0, &i), 0.0);
    }
}
