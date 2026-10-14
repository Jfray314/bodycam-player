//! Port of GA_Fire_Base / GA_Reload_Base / GA_MagCheck / GA_Inspect / GA_Character_Aim / GA_Character_SwapItem.
//! Stats: WeaponAttributeSet from GE_WeaponStat_* (assets/weapon_stats.json). Weapons, action montages and
//! notify times: DT_WeaponItems rows + BAS_* animation sets (assets/weapons/<Row>.json, tools/weapon_data.py).
//! Inputs follow IMC_KBM_CharacterOnFoot: R tap (<0.25 s, IA_Reload) reloads, R hold 0.25 s (IA_InspectAmmos)
//! mag-checks, J inspects, 1-6 / wheel swap, LMB fire, RMB aim.
use crate::player::{BodycamCamera, Player};
use crate::viewmodel::GunRoot;
use bevy::camera::primitives::MeshAabb;
use bevy::input::mouse::MouseWheel;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use rand::Rng;
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

const STATS_JSON: &str = include_str!("../../assets/weapon_stats.json");

/// WeaponAttributeSet. Missing attributes inherit GE_WeaponStat_Default, like the BP parent class.
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "PascalCase")]
pub struct Stats {
    pub damage: Option<f32>,
    pub fire_rate: Option<f32>,
    pub instability: Option<f32>,
    pub mag_amount: Option<f32>,
    pub ammo_capacity: Option<f32>,
    pub pellet_count: Option<f32>,
    pub ragdoll_force: Option<f32>,
    pub spread_hip_x: Option<f32>,
    pub spread_hip_y: Option<f32>,
    pub spread_aim_x: Option<f32>,
    pub spread_aim_y: Option<f32>,
    #[serde(rename = "fire_modes")]
    pub fire_modes: Vec<String>,
    /// GE_WeaponStat_* granted tags (Gameplay.Weapons.InfiniteMag).
    #[serde(default, rename = "tags")]
    pub tags: Vec<String>,
}

impl Stats {
    fn over(self, d: &Stats) -> Stats {
        Stats {
            damage: self.damage.or(d.damage),
            fire_rate: self.fire_rate.or(d.fire_rate),
            instability: self.instability.or(d.instability),
            mag_amount: self.mag_amount.or(d.mag_amount),
            ammo_capacity: self.ammo_capacity.or(d.ammo_capacity),
            pellet_count: self.pellet_count.or(d.pellet_count),
            ragdoll_force: self.ragdoll_force.or(d.ragdoll_force),
            spread_hip_x: self.spread_hip_x.or(d.spread_hip_x),
            spread_hip_y: self.spread_hip_y.or(d.spread_hip_y),
            spread_aim_x: self.spread_aim_x.or(d.spread_aim_x),
            spread_aim_y: self.spread_aim_y.or(d.spread_aim_y),
            fire_modes: if self.fire_modes.is_empty() { d.fire_modes.clone() } else { self.fire_modes },
            tags: if self.tags.is_empty() { d.tags.clone() } else { self.tags },
        }
    }
}

pub fn load_stats() -> HashMap<String, Stats> {
    let mut all: HashMap<String, Stats> = serde_json::from_str(STATS_JSON).expect("weapon_stats.json");
    let d = all.remove("Default").unwrap_or_default();
    all.into_iter().map(|(k, s)| (k, s.over(&d))).collect()
}

/// A character montage from a BAS animation set: length and notifies (time, name, props).
#[derive(Clone, Debug)]
pub struct Montage {
    pub path: String,
    pub length: f32,
    pub notifies: Vec<(f32, String, Value)>,
    /// FAlphaBlend BlendIn/BlendOut times (s) of the character montage.
    pub blend_in: f32,
    pub blend_out: f32,
    /// Gun-side montage asset path (BAS `Weapon` key), if any.
    pub weapon: Option<String>,
}

impl Montage {
    fn from(v: &Value) -> Option<Self> {
        // Gun-only montages (FireOut) have no Character key; the gun-side clip stands in for it.
        let c = v.get("Character").or_else(|| v.get("Weapon")).unwrap_or(v);
        Some(Montage {
            path: c.get("path")?.as_str()?.into(),
            length: c.get("length")?.as_f64()? as f32,
            blend_in: c.get("blend_in").and_then(|b| b.as_f64()).unwrap_or(0.25) as f32,
            blend_out: c.get("blend_out").and_then(|b| b.as_f64()).unwrap_or(0.25) as f32,
            weapon: v.get("Weapon").and_then(|w| w.get("path")).and_then(|p| p.as_str()).map(Into::into),
            notifies: c
                .get("notifies")?
                .as_array()?
                .iter()
                .map(|n| (n["time"].as_f64().unwrap_or(0.0) as f32, n["name"].as_str().unwrap_or("").into(), n["props"].clone()))
                .collect(),
        })
    }

    /// Mag-check HUD window from the SetMagCheckUIVisibility notifies (shown, then "Show?" = false).
    pub fn mag_ui_window(&self) -> Option<(f32, f32)> {
        let vis = |hide: bool| {
            self.notifies
                .iter()
                .find(|n| n.1 == "SetMagCheckUIVisibility_C" && (n.2.get("Show?") == Some(&Value::Bool(false))) == hide)
                .map(|n| n.0)
        };
        Some((vis(false)?, vis(true)?))
    }
}

pub struct Weapon {
    /// DT_WeaponItems row name.
    pub name: String,
    pub stats: Stats,
    pub ammo: u32,
    /// Spare magazines (rounds in each), like MagazineManager.
    pub mags: Vec<u32>,
    pub full_auto: bool,
    /// BAS_* animation set (baked JSON).
    pub anims: Value,
    /// FirstEquip plays only the first time the weapon is drawn.
    equipped_once: bool,
}

impl Weapon {
    fn cap(&self) -> u32 {
        self.stats.ammo_capacity.unwrap_or(30.0) as u32
    }
    /// GA_Reload_Base.HasInfiniteMags (tag half; the lobby game-mode / game-rule halves are not modelled).
    fn infinite_mags(&self) -> bool {
        self.stats.tags.iter().any(|t| t == "Gameplay.Weapons.InfiniteMag")
    }
    fn can_reload(&self) -> bool {
        self.ammo < self.cap() && (self.infinite_mags() || !self.mags.is_empty())
    }
    /// GA_Reload_Base at the refill point: the best (fullest) mag is chosen, a non-empty chamber/mag is returned
    /// to the spares as its own mag (tactical reload), the best mag is consumed unless mags are infinite.
    fn reload_done(&mut self) {
        let best = self.mags.iter().copied().enumerate().max_by_key(|&(_, m)| m);
        let Some((idx, amount)) = best else { return };
        if self.ammo > 0 {
            self.mags.push(self.ammo);
        }
        if !self.infinite_mags() {
            self.mags.remove(idx);
        }
        self.ammo = amount;
    }
    /// Bolt-action guns (GA_Reload_Bolt): per-round reload loop (BAS ReloadLoop*/ReloadStart*) instead of a mag swap.
    fn is_bolt(&self) -> bool {
        self.anims.get("ReloadLoopCalm").is_some() && self.anims.get("ReloadEmptyCalm").is_none()
    }
    /// MagazineManager.PickAmmo: one round out of the spares (fullest mag first), unless mags are infinite.
    fn pick_round(&mut self) {
        if self.infinite_mags() {
            return;
        }
        if let Some((i, _)) = self.mags.iter().copied().enumerate().max_by_key(|&(_, m)| m) {
            self.mags[i] -= 1;
            if self.mags[i] == 0 {
                self.mags.remove(i);
            }
        }
    }
    /// First montage of a list key (Fire: the one the viewmodel plays).
    fn one_of(&self, key: &str) -> Option<Montage> {
        Montage::from(self.anims.get(key)?.get(0)?)
    }
    fn one(&self, key: &str) -> Option<Montage> {
        Montage::from(self.anims.get(key)?)
    }
    /// Random entry of a montage list (ReloadTacCalm, ReloadEmptyCalm, ...).
    fn pick(&self, key: &str) -> Option<Montage> {
        let list = self.anims.get(key)?.as_array()?;
        Montage::from(list.get(rand::rng().random_range(0..list.len().max(1)))?)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionKind {
    Reload,
    MagCheck,
    Inspect,
    Holster,
    Draw,
    /// WEP.PullSliderLogic on the last round: gun-only FireOut montage (never an Inventory action).
    FireOut,
    /// AC_BodycamCamera GoPro adjust montage (camadjust.rs); not an Inventory action.
    CameraAdjust,
}

#[derive(Clone, Debug)]
pub struct Action {
    pub kind: ActionKind,
    pub montage: Montage,
    pub t: f32,
    /// Reload only: the ammo refill happens at 0.7 x montage length (GA_Reload_Base WaitDelay), once.
    refill_at: f32,
    /// Slot to draw after a holster.
    next_slot: usize,
    /// GA_Reload_Bolt step this montage belongs to (None = ordinary action).
    bolt: Option<Bolt>,
}

/// GA_Reload_Bolt montages: StartReload (Enter, `empty` = bIsMagEmpty), then per-round loops, the last round plays the End montage.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Bolt {
    Start { empty: bool, stressed: bool },
    Loop { stressed: bool },
    End,
}

/// PRAS RecoilData.ControllerRecoil: per-shot (min, max) steps in degrees and the RecoilSmoothing interp speed.
#[derive(Clone, Debug)]
pub struct ControllerRecoil {
    pub horizontal: (f32, f32),
    pub vertical: (f32, f32),
    pub smoothing: f32,
    /// Return-to-zero rate (1/s) and player-input credit (0..1).
    pub damping: f32,
    pub compensation: f32,
}

impl ControllerRecoil {
    pub(crate) fn from(anims: &Value) -> Self {
        let c = &anims["RecoilData"]["ControllerRecoil"];
        let r = |k: &str| (c[k]["x"].as_f64().unwrap_or(0.0) as f32, c[k]["y"].as_f64().unwrap_or(0.0) as f32);
        let (lo, hi) = r("HorizontalRecoilStep");
        let (vlo, vhi) = r("VerticalRecoilStep");
        ControllerRecoil { horizontal: (lo.min(hi), lo.max(hi)), vertical: (vlo.min(vhi), vlo.max(vhi)), smoothing: r("RecoilSmoothing").1.max(1.0), damping: c["Damping"].as_f64().unwrap_or(0.0) as f32, compensation: c["Compensation"].as_f64().unwrap_or(1.0) as f32 }
    }
}

/// One shot fired by the player (camera recoil/shake, audio, viewmodel recoil).
#[derive(Message, Clone, Debug)]
pub struct Fired {
    pub row: String,
    pub controller: ControllerRecoil,
    pub aiming: bool,
    /// FireRate attribute (seconds between shots); drives the stress gain.
    pub fire_rate: f32,
    /// Suppressor mounted: audio plays Cosmetic.Weapon.SFX.FireSuppressed.
    pub suppressed: bool,
}

/// AC_BodycamAnimationComponent.PlayEndFireRegripMotion / ForceReGripMotion(delay): play a random ReGripMotions
/// montage after a (retriggerable) delay.
#[derive(Message, Clone, Debug)]
pub struct Regrip {
    pub delay: f32,
}

/// AC.PlayHandFatigue overflow (HandFatigue > 10): A_FP_shakingHand_Montage.
#[derive(Message, Clone, Debug)]
pub struct HandShake;

/// Trigger pulled on an empty weapon (audio plays FireDry).
#[derive(Message, Clone, Debug)]
pub struct DryFire {
    pub row: String,
}

/// Fired when a weapon action montage starts (audio plays its PlayWepSounds notifies, viewmodel plays the anim).
#[derive(Message, Clone, Debug)]
pub struct MontageStarted {
    pub row: String,
    pub kind: ActionKind,
    pub montage: Montage,
}

#[derive(Component)]
pub struct Inventory {
    pub slots: Vec<Weapon>,
    pub current: usize,
    pub cooldown: f32,
    pub action: Option<Action>,
    /// Seconds R has been held (tap = reload, hold = mag check).
    r_held: f32,
    /// GA_Fire_Base FireStartTime of the current trigger pull and LastFireTime (previous pull's start), seconds.
    fire_start: Option<f32>,
    last_fire_start: f32,
    /// Seconds until each pending WEP.PullSliderLogic (0 = this shot; PullSlider notify time for bUsePullSliderNotify guns).
    pull_slider: Vec<f32>,
    /// Seconds until the cancelled bolt reload's CancelLoop montage plays.
    cancel_in: Option<f32>,
}

impl Inventory {
    /// Loadout screen: the chosen primary and sidearm become slots 1 and 2 (the rest keep their order) and the primary is current.
    pub fn equip(&mut self, primary: &str, secondary: &str) {
        self.slots.sort_by_key(|w| (w.name != primary, w.name != secondary));
        (self.current, self.action) = (0, None);
    }
    /// Draws the current weapon again (after the tablet is put away).
    pub fn redraw(&mut self, out: &mut MessageWriter<MontageStarted>) {
        let cur = self.current;
        let w = &mut self.slots[cur];
        let m = if w.equipped_once { w.one("Draw") } else { w.one("FirstEquip") };
        w.equipped_once = true;
        self.action = None;
        self.start(ActionKind::Draw, m, cur, out);
    }
    pub fn busy(&self) -> bool {
        self.action.is_some()
    }
    fn start(&mut self, kind: ActionKind, montage: Option<Montage>, next_slot: usize, out: &mut MessageWriter<MontageStarted>) {
        let Some(montage) = montage else { return };
        out.write(MontageStarted { row: self.slots[self.current].name.clone(), kind, montage: montage.clone() });
        let refill_at = if kind == ActionKind::Reload { montage.length * 0.7 } else { f32::MAX };
        self.action = Some(Action { kind, montage, t: 0.0, refill_at, next_slot, bolt: None });
    }
    /// GA_Reload_Bolt L43: another round needed -> the End montage for the last one, else a random Loop; none -> ability ends.
    fn bolt_next(&mut self, stressed: bool, out: &mut MessageWriter<MontageStarted>) {
        let w = &self.slots[self.current];
        if !(w.ammo < w.cap() && (w.infinite_mags() || !w.mags.is_empty())) {
            return;
        }
        let (m, bolt) = if w.ammo + 1 == w.cap() {
            (w.one(if stressed { "ReloadEndStressed" } else { "ReloadEndCalm" }), Bolt::End)
        } else {
            (w.pick(if stressed { "ReloadLoopStressed" } else { "ReloadLoopCalm" }), Bolt::Loop { stressed })
        };
        self.start(ActionKind::Reload, m, self.current, out);
        if let Some(a) = &mut self.action {
            a.refill_at = f32::MAX;
            a.bolt = Some(bolt);
        }
    }
    /// GA_Reload_Bolt.K2_OnEndAbility(cancelled) -> GA_CancelBoltReload: the running montage plays to its WaitDelay end
    /// (length - 0.1, no round added), then CancelLoop.
    fn cancel_bolt(&mut self) {
        if let Some(a) = &self.action
            && a.bolt.is_some()
        {
            self.cancel_in = Some((a.montage.length - 0.1 - a.t).max(0.0));
            self.action = None;
        }
    }
}

/// CurveFloat_RegripProbability (extracted/json/.../Fire/CurveFloat_RegripProbability.json), keys as [t, v, cubic, arrive, leave].
const REGRIP_CURVE: [crate::curve::Key; 2] = [[0.2, 0.1, 2.0, 0.035_373_74, 0.035_373_74], [1.2, 1.0, 2.0, 0.000_488_48, 0.000_488_48]];

#[derive(Component)]
pub struct Health(pub f32);

#[derive(Component)]
pub(crate) struct Fading(pub f32);

/// Moving effect (smoke puff, impact dust/debris): velocity, gravity (m/s2), growth rate (scale/s), age/life, start alpha
/// (material is per-particle so alpha can fade), camera-facing sprite or not.
#[derive(Component)]
struct Particle {
    vel: Vec3,
    gravity: f32,
    grow: f32,
    age: f32,
    life: f32,
    alpha: f32,
    mat: Option<Handle<crate::soft::SoftMat>>,
    billboard: bool,
}

/// Surface class of a hit: BallisticsVFX FXMaterialDataTable_GenericBullets rows by EPhysicalSurface.
/// Default / SurfaceType2 / 50 = Concrete (Concrete_impact_Dyn + Decals_Concrete), 6/36/37/51 = Wood (Wood_impact_Dyn + Decals_Wood),
/// 15/16 = Metal (MetalDull/Metal_impact_Dyn2 + Decals_Metal). Other rows fall back to concrete.
#[derive(Clone, Copy)]
enum Surf {
    Concrete,
    Metal,
    Wood,
}
impl Surf {
    fn of(surface: Option<u8>) -> Self {
        match surface {
            Some(15 | 16) => Surf::Metal,
            Some(6 | 36 | 37 | 51) => Surf::Wood,
            _ => Surf::Concrete,
        }
    }
}

/// Fire effects: tracer, per-surface bullet-hole decal (4 atlas quadrants each), smoke sprite, debris bit, shell.
#[derive(Resource)]
pub(crate) struct FireFx {
    pub tracer: (Handle<Mesh>, Handle<StandardMaterial>),
    decals: [(Vec<Handle<Mesh>>, Handle<StandardMaterial>); 3],
    quad: Handle<Mesh>,
    smoke: Handle<Image>,
    bit: Handle<Mesh>,
    shell: (Handle<Mesh>, Handle<StandardMaterial>),
}

/// Decal quad showing one quadrant of the 2x2 decal atlas (Decals/Textures/*_1: 512 px, 4 hole variants in RGBA).
fn atlas_quad(meshes: &mut Assets<Mesh>, size: f32, qx: f32, qy: f32) -> Handle<Mesh> {
    let mut m = Rectangle::new(size, size).mesh().build();
    // Rectangle vertex order: +x+y, -x+y, -x-y, +x-y.
    let uv: Vec<[f32; 2]> = [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]].iter().map(|u| [(qx + u[0]) * 0.5, (qy + u[1]) * 0.5]).collect();
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    meshes.add(m)
}

fn setup_fx(mut cmd: Commands, assets: Res<AssetServer>, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let brass = mats.add(StandardMaterial { base_color: Color::srgb(0.78, 0.6, 0.3), metallic: 1.0, perceptual_roughness: 0.35, ..default() });
    let tracer = mats.add(StandardMaterial { base_color: Color::srgb(1.0, 0.9, 0.5), emissive: LinearRgba::rgb(8.0, 6.0, 2.0), unlit: true, cull_mode: None, ..default() });
    // DecalSize (5,5,5) cm half-extents in every FXMaterialData row -> 10 cm quad.
    let mut decal = |tex: &str| {
        let m = mats.add(StandardMaterial {
            base_color_texture: Some(assets.load(format!("textures/fx/{tex}.png"))),
            // Lit (not unlit): unlit colours ignore scene lighting, so auto exposure (+5 EV indoors) blew them out to white.
            base_color: Color::srgb(0.12, 0.12, 0.12),
            perceptual_roughness: 1.0,
            reflectance: 0.0,
            alpha_mode: AlphaMode::Blend,
            cull_mode: None,
            ..default()
        });
        let quads = (0..4).map(|i| atlas_quad(&mut meshes, 0.10, (i % 2) as f32, (i / 2) as f32)).collect();
        (quads, m)
    };
    let decals = [decal("decal_concrete"), decal("decal_metal"), decal("decal_wood")];
    cmd.insert_resource(FireFx {
        tracer: (meshes.add(Cuboid::new(0.01, 0.01, 1.0)), tracer),
        decals,
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        // whisp_One_512_8x8 frame (4,3): the smoke/dust sprite of the Dust emitters (Whisps_trans) and muzzle smoke.
        smoke: assets.load("textures/fx/smoke.png"),
        bit: meshes.add(Cuboid::new(0.012, 0.012, 0.012)),
        // SM_Shell_9mm_Empty (tools/shell_glb.py), the SpawnShell default mesh, in the gun frame.
        // Brass instead of the glb's material: Bevy 0.20 glb sub-assets load as GltfMaterial, not StandardMaterial.
        shell: (assets.load("models/shell.glb#Mesh0/Primitive0"), brass),
    });
}

pub struct WeaponPlugin;
impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MontageStarted>()
            .add_message::<Fired>()
            .add_message::<DryFire>()
            .add_message::<Regrip>()
            .add_message::<HandShake>()
            .add_message::<ShellLanded>()
            .add_systems(Startup, setup_fx)
            .add_systems(PostStartup, give_loadout)
            .add_systems(Update, ((actions, swap, aim, fire, eject_shells, fade, particles, shells).chain(), log_montages));
    }
}

/// Every DT_WeaponItems row that has an animation set, with stats from its GE_WeaponStat_* effect.
fn load_weapons() -> Vec<Weapon> {
    let stats = load_stats();
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/weapons");
    let mut out = vec![];
    for e in std::fs::read_dir(dir).expect("assets/weapons").flatten() {
        let v: Value = serde_json::from_str(&std::fs::read_to_string(e.path()).unwrap()).unwrap();
        let ge = v["stats_effects"].get(0).and_then(Value::as_str).unwrap_or("");
        let key = ge.rsplit("GE_WeaponStat_").next().unwrap_or("").trim_end_matches("_C").trim_start_matches('_');
        let Some(s) = stats.get(key) else { continue };
        out.push(Weapon {
            name: v["row"].as_str().unwrap_or(key).into(),
            ammo: s.ammo_capacity.unwrap_or(30.0) as u32,
            mags: vec![s.ammo_capacity.unwrap_or(30.0) as u32; s.mag_amount.unwrap_or(4.0) as usize],
            full_auto: s.fire_modes.iter().any(|m| m == "FullAuto"),
            stats: s.clone(),
            anims: v["anim_set"].clone(),
            equipped_once: false,
        });
    }
    out
}

fn give_loadout(mut cmd: Commands, player: Single<Entity, With<Player>>, mut out: MessageWriter<MontageStarted>) {
    let mut slots = load_weapons();
    // Default loadout: primary M4A1 + sidearm Glock17, then every other gun for testing.
    // Dev: BODYCAM_START=<name> equips that gun first (gun audits).
    let cfg = crate::ui::settings::load(); // loadout chosen on the tablet
    let norm = |s: &str| s.to_lowercase().replace([' ', '-'], "_");
    let start = norm(&std::env::var("BODYCAM_START").unwrap_or_else(|_| cfg.primary.clone()));
    slots.sort_by_key(|w| (norm(&w.name) != start, w.name != cfg.primary, w.name != cfg.secondary, w.name.clone()));
    let mut inv = Inventory { slots, current: 0, cooldown: 0.0, action: None, r_held: 0.0, fire_start: None, last_fire_start: 0.0, pull_slider: vec![], cancel_in: None };
    let first = inv.slots[0].one("FirstEquip");
    inv.slots[0].equipped_once = true;
    inv.start(ActionKind::Draw, first, 0, &mut out);
    cmd.entity(*player).insert(inv);
}

/// Advances the current action; handles R tap/hold and J.
fn actions(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, status: Res<crate::status::SurvivorStatus>, mut inv: Single<&mut Inventory>, mut out: MessageWriter<MontageStarted>) {
    let dt = time.delta_secs();
    let mut refill = false;
    if let Some(a) = &mut inv.action {
        a.t += dt;
        if a.t >= a.refill_at {
            a.refill_at = f32::MAX;
            refill = true;
        }
    }
    if refill {
        let cur = inv.current;
        inv.slots[cur].reload_done();
    }
    // GA_Reload_Bolt chain: each montage hands over at its WaitDelay (Start length - 0.05, Loop/End length - 0.1).
    if let Some(a) = &inv.action
        && let Some(b) = a.bolt
        && a.t >= a.montage.length - if matches!(b, Bolt::Start { .. }) { 0.05 } else { 0.1 }
    {
        let cur = inv.current;
        inv.action = None;
        match b {
            Bolt::Start { empty, stressed } => {
                if empty {
                    inv.slots[cur].ammo += 1; // RefillAmmo(1.0)
                }
                inv.slots[cur].pick_round();
                inv.bolt_next(stressed, &mut out);
            }
            Bolt::Loop { stressed } => {
                inv.slots[cur].ammo += 1;
                inv.slots[cur].pick_round();
                inv.bolt_next(stressed, &mut out);
            }
            Bolt::End => {
                inv.slots[cur].ammo += 1;
                inv.slots[cur].pick_round();
            }
        }
    }
    if let Some(t) = &mut inv.cancel_in {
        *t -= dt;
        if *t <= 0.0 {
            inv.cancel_in = None;
            let w = &inv.slots[inv.current];
            if let Some(m) = w.one("CancelLoop") {
                out.write(MontageStarted { row: w.name.clone(), kind: ActionKind::Reload, montage: m });
            }
        }
    }
    if let Some(a) = &inv.action
        && a.bolt.is_none()
    {
        if a.t >= a.montage.length {
            let (kind, next) = (a.kind, a.next_slot);
            inv.action = None;
            match kind {
                ActionKind::Holster => {
                    inv.current = next;
                    let w = &mut inv.slots[next];
                    let m = if w.equipped_once { w.one("Draw") } else { w.one("FirstEquip") };
                    w.equipped_once = true;
                    inv.start(ActionKind::Draw, m, next, &mut out);
                }
                _ => {}
            }
        }
    }
    // IA_Reload: InputTriggerTap 0.25 s. IA_InspectAmmos: InputTriggerHold 0.25 s, one-shot.
    const TAP: f32 = 0.25;
    let cur = inv.current;
    if keys.pressed(KeyCode::KeyR) {
        inv.r_held += dt;
        if inv.r_held >= TAP && inv.r_held - dt < TAP && !inv.busy() {
            let w = &inv.slots[cur];
            let m = if w.ammo == 0 { w.one("MagCheckEmpty") } else { w.one("MagCheckFull") };
            inv.start(ActionKind::MagCheck, m, cur, &mut out);
        }
    } else if inv.r_held > 0.0 {
        let tapped = inv.r_held < TAP;
        inv.r_held = 0.0;
        let w = &inv.slots[cur];
        if tapped && !inv.busy() && w.is_bolt() && w.can_reload() {
            // GA_Reload_Bolt.StartReload: Enter montage by bIsStressed / bIsMagEmpty.
            let stressed = rand::rng().random::<f32>() < status.stress;
            let empty = w.ammo == 0;
            let key = match (empty, stressed) {
                (false, false) => "ReloadStartCalm",
                (false, true) => "ReloadStartStressed",
                (true, false) => "ReloadStartCalmEmpty",
                (true, true) => "ReloadStartStressedEmpty",
            };
            let m = w.pick(key);
            inv.start(ActionKind::Reload, m, cur, &mut out);
            if let Some(a) = &mut inv.action {
                a.refill_at = f32::MAX;
                a.bolt = Some(Bolt::Start { empty, stressed });
            }
        } else if tapped && !inv.busy() && w.can_reload() {
            // GA_Reload_Base.DetermineStressedStatus: bIsStressed = RandomBoolWithWeight(Stress); GetReloadMontage picks the
            // Stressed or Calm array. ponytail: Tac-reload falls back to the Empty array when a gun has none.
            let stressed = rand::rng().random::<f32>() < status.stress;
            let (empty, tac) = if stressed { ("ReloadEmptyStressed", "ReloadTacStressed") } else { ("ReloadEmptyCalm", "ReloadTacCalm") };
            let m = if w.ammo == 0 { w.pick(empty) } else { w.pick(tac) };
            let m = m.or_else(|| w.pick(empty)).or_else(|| w.pick("ReloadEmptyCalm"));
            inv.start(ActionKind::Reload, m, cur, &mut out);
        }
    }
    if keys.just_pressed(KeyCode::KeyJ) && !inv.busy() {
        let m = inv.slots[cur].one("Inspect");
        inv.start(ActionKind::Inspect, m, cur, &mut out);
    }
}

/// GA_Character_SwapItem: holster the current weapon (cancelling reload/inspect), then draw the next.
fn swap(keys: Res<ButtonInput<KeyCode>>, mut inv: Single<&mut Inventory>, mut wheel: MessageReader<MouseWheel>, mut out: MessageWriter<MontageStarted>) {
    let n = inv.slots.len();
    let mut next = inv.current;
    let digits = [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5, KeyCode::Digit6];
    for (i, k) in digits.iter().enumerate() {
        if keys.just_pressed(*k) && i < n {
            next = i;
        }
    }
    for w in wheel.read() {
        next = if w.y < 0.0 { (next + 1) % n } else { (next + n - 1) % n };
    }
    let swapping = matches!(inv.action, Some(Action { kind: ActionKind::Holster | ActionKind::Draw, .. }));
    if next != inv.current && !swapping {
        let m = inv.slots[inv.current].one("holster");
        let cur = inv.current;
        inv.start(ActionKind::Holster, m, next, &mut out);
        if inv.action.is_none() {
            inv.current = next; // no holster montage: switch instantly
        }
        let _ = cur;
    }
}

/// Aiming doesn't zoom: the bodycam FOV stays 120 (measured live); the gun comes up to the camera instead.
fn aim(mouse: Res<ButtonInput<MouseButton>>, mut p: Single<&mut Player>, mut regrip: MessageWriter<Regrip>) {
    p.aiming = mouse.pressed(MouseButton::Right);
    // GA_Character_Aim end: AC.ForceReGripMotion(0.2).
    if mouse.just_released(MouseButton::Right) {
        regrip.write(Regrip { delay: 0.2 });
    }
}

/// Elliptical-cone half angles in degrees -> UE FMath::VRandCone(Dir, HorizontalHalfAngle, VerticalHalfAngle)
/// (RandomUnitVectorInEllipticalConeInDegrees(Dir, MaxYaw, MaxPitch)).
pub fn random_in_elliptical_cone(dir: Vec3, yaw_deg: f32, pitch_deg: f32, rng: &mut impl Rng) -> Vec3 {
    let (h, v) = (yaw_deg.to_radians(), pitch_deg.to_radians());
    if h <= 0.0 && v <= 0.0 {
        return dir;
    }
    let theta = std::f32::consts::TAU * rng.random::<f32>();
    let mut phi = (2.0 * rng.random::<f32>() - 1.0).acos();
    // Cone half angle as a function of theta: polar form of the ellipse (x/v)^2 + (y/h)^2 = 1; circular if one axis is 0.
    let half = if h > 0.0 && v > 0.0 { (1.0 / ((theta.cos() / v).powi(2) + (theta.sin() / h).powi(2))).sqrt() } else { h.max(v) };
    phi %= half;
    let right = dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
    (Quat::from_axis_angle(dir, theta) * Quat::from_axis_angle(right, phi) * dir).normalize()
}

/// GetBoneDamageMultiplier (GA_Fire_Base); 1.0 for anything that isn't a named bone.
pub fn bone_multiplier(bone: &str) -> f32 {
    match bone.to_ascii_lowercase().as_str() {
        "head" | "neck" => 10.0,
        "spine_01" => 2.5,
        "spine_02" | "spine_03" => 4.0,
        _ => 1.0,
    }
}

/// Absorbance by EPhysicalSurface index (FXMaterialDataTable_GenericBullets rows).
pub fn surface_absorbance(surface: u8) -> f32 {
    match surface {
        3 => 0.1,
        6 | 15 | 36 | 51 => 0.6,
        12 => 1.0,
        13 => 0.3,
        _ => 0.0,
    }
}

/// Physical surface of a world mesh (what its PhysMat's SurfaceType would be); absent = `DEFAULT_ABSORBANCE`.
#[derive(Component)]
pub struct SurfaceMat(pub u8);

/// GA_Fire_Base: BulletDamageMultiplier *= row.Absorbance (0 = stops). ponytail: the lobby map's meshes carry no PhysMat
/// yet; unmarked geometry is treated as FXMaterialDataTable "Default" (concrete, 0) and stops the bullet.
const DEFAULT_ABSORBANCE: f32 = 0.0;
const FIRE_RANGE: f32 = 5000.0; // GA_Fire_Base.FireRange 500000 cm
const MIN_BULLET_MULT: f32 = 0.25;

/// The GA_Fire_Base hit loop: hits in trace order; each new actor records the current multiplier, then absorbs.
/// Stops once the multiplier is <= 0.25. Returns (hit index, multiplier at the hit).
pub fn penetrate<T: Eq + std::hash::Hash + Copy>(hits: &[(T, f32)]) -> Vec<(usize, f32)> {
    let (mut mult, mut seen, mut out) = (1.0f32, std::collections::HashSet::new(), vec![]);
    for (i, (actor, absorb)) in hits.iter().enumerate() {
        if mult <= MIN_BULLET_MULT {
            break;
        }
        if seen.insert(*actor) {
            out.push((i, mult));
            mult *= absorb;
        }
    }
    out
}

/// WEP.GetBulletSpawnOrigin / GetInitialFireDirection: origin = BulletSpawnOrigin (WEP `Scene`, yaw 90 so its X is the
/// barrel axis) snapped to the barrel part's FireLocation socket (assets/fire_sockets.json, tools/fire_sockets.py);
/// yaw = barrel yaw, pitch = barrel pitch when aiming, else camera pitch. The barrel axis is the gun-local axis most
/// aligned with the camera. Guns without a FireLocation (Crossbow) use the far end of the gun meshes' bounds.
fn muzzle_basis(
    gun: &Query<(Entity, &GlobalTransform, &GunRoot)>,
    joints: &Query<(Entity, &Name, &GlobalTransform)>,
    parents: &Query<&ChildOf>,
    kids: &Query<&Children>,
    meshes: &Query<(&GlobalTransform, &Mesh3d)>,
    assets: &Assets<Mesh>,
    cam: &GlobalTransform,
    aiming: bool,
    sights: &mut HashMap<String, Vec3>,
) -> (Vec3, Vec3) {
    let cam_fwd = cam.forward().as_vec3();
    let Ok((root, rgt, model)) = gun.single() else { return (cam.translation(), cam_fwd) };
    let rot = rgt.rotation();
    let axis = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z]
        .into_iter()
        .max_by(|a, b| (rot * *a).dot(cam_fwd).total_cmp(&(rot * *b).dot(cam_fwd)))
        .unwrap();
    let gun_fwd = rot * axis;
    let (mut far, mut sum, mut n) = (f32::MIN, Vec3::ZERO, 0.0);
    for e in kids.iter_descendants(root) {
        let Ok((gt, m)) = meshes.get(e) else { continue };
        let Some(aabb) = assets.get(&m.0).and_then(|m| m.get_aabb()) else { continue };
        for i in 0..8u8 {
            let sgn = Vec3::new(if i & 1 == 0 { -1.0 } else { 1.0 }, if i & 2 == 0 { -1.0 } else { 1.0 }, if i & 4 == 0 { -1.0 } else { 1.0 });
            let w = gt.transform_point(Vec3::from(aabb.center) + Vec3::from(aabb.half_extents) * sgn);
            far = far.max(w.dot(gun_fwd));
            sum += w;
            n += 1.0;
        }
    }
    let centre = if n > 0.0 { sum / n } else { rgt.translation() };
    let socket = FIRE_SOCKETS.with(|m| m.get(model.0).cloned());
    let fire_loc = socket.and_then(|(bone, loc)| {
        let (_, _, jt) = joints.iter().find(|(e, n, _)| n.as_str().eq_ignore_ascii_case(&bone) && crate::viewmodel::root_of_entity(*e, root, parents))?;
        Some(jt.transform_point(Vec3::new(loc.x, -loc.y, loc.z) * 0.01)) // UE cm -> joint frame (x, -y, z) m
    });
    let origin = fire_loc.unwrap_or_else(|| centre + gun_fwd * (if n > 0.0 { far } else { centre.dot(gun_fwd) } - centre.dot(gun_fwd)));
    // Aimed: UE fires along the bore (BulletSpawnOrigin pitch), but on our models the eye (AimPoint, on the camera when
    // aimed) sits ~1-3 deg above the bore-parallel line through the front sight, so shots landed above the post. Fire
    // with the sight line's pitch instead: eye -> front sight tip (highest vertex on the gun's front half, cached per gun in
    // root space). It moves with the gun, so recoil/sway still steer the shot.
    // ponytail: no zeroing; the bullet starts at the muzzle, so holes sit ~sight-height (5-7 cm) under the post.
    // Pitch only: yaw stays the bore's (UE rule), and the highest vertex can be a front-sight wing beside the post.
    let mut sight_pitch = None;
    if aiming && n > 0.0 {
        let tip = match sights.get(model.0) {
            Some(t) => Some(*t),
            None => {
                let up = cam.up().as_vec3();
                let mut best: Option<(f32, Vec3)> = None;
                for e in kids.iter_descendants(root) {
                    let Ok((gt, m)) = meshes.get(e) else { continue };
                    let Some(pos) = assets.get(&m.0).and_then(|m| m.attribute(Mesh::ATTRIBUTE_POSITION)).and_then(|a| a.as_float3()) else { continue };
                    for v in pos {
                        let w = gt.transform_point(Vec3::from(*v));
                        if (w - centre).dot(gun_fwd) > 0.0 && best.is_none_or(|b| w.dot(up) > b.0) {
                            best = Some((w.dot(up), w));
                        }
                    }
                }
                best.map(|(_, w)| {
                    let t = rgt.affine().inverse().transform_point3(w);
                    sights.insert(model.0.to_string(), t);
                    t
                })
            }
        };
        sight_pitch = tip.map(|t| (rgt.transform_point(t) - cam.translation()).normalize().y.clamp(-1.0, 1.0).asin());
    }
    let yaw = (-gun_fwd.x).atan2(-gun_fwd.z);
    let pitch = if aiming { sight_pitch.unwrap_or(gun_fwd.y.clamp(-1.0, 1.0).asin()) } else { cam_fwd.y.clamp(-1.0, 1.0).asin() };
    (origin, Vec3::new(-yaw.sin() * pitch.cos(), pitch.sin(), -yaw.cos() * pitch.cos()))
}

fn fire(
    mut cmd: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut q: Single<(&mut Inventory, &mut Player)>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    mut ray: MeshRayCast,
    mut targets: Query<(Option<&Name>, Option<&SurfaceMat>, Option<&mut Health>, Has<crate::player::IgnoreTraces>)>,
    fx: Res<FireFx>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut soft: ResMut<Assets<crate::soft::SoftMat>>,
    mut fired: MessageWriter<Fired>,
    mut dry: MessageWriter<DryFire>,
    mut regrip: MessageWriter<Regrip>,
    (gun, kids, gun_meshes, mesh_assets): (Query<(Entity, &GlobalTransform, &GunRoot)>, Query<&Children>, Query<(&GlobalTransform, &Mesh3d)>, Res<Assets<Mesh>>),
    (joints, parents, mut sights, cfg): (Query<(Entity, &Name, &GlobalTransform)>, Query<&ChildOf>, Local<HashMap<String, Vec3>>, Option<Res<crate::ui::settings::Settings>>),
) {
    let (inv, p) = &mut *q;
    let suppressed = cfg.as_ref().is_some_and(|c| c.has_att(&inv.slots[inv.current].name, "suppressor"));
    inv.cooldown -= time.delta_secs();
    let now = time.elapsed_secs();
    // GA_Fire_Base ability end (trigger released): TryPlayRegripAnimation, then LastFireTime = FireStartTime.
    if let Some(start) = inv.fire_start
        && !mouse.pressed(MouseButton::Left)
    {
        let w = &inv.slots[inv.current];
        let instability = w.stats.instability.unwrap_or(0.0);
        let fr = w.stats.fire_rate.unwrap_or(0.1);
        let weight = if !w.full_auto {
            // SemiAuto: gap between this pull's start and the previous one's, mapped (FireRate+0.1 .. 2*FireRate -> 0.02..1).
            crate::status::map_range_clamped(start - inv.last_fire_start, fr + 0.1, fr * 2.0, 0.02, 1.0) * instability
        } else {
            // Auto: RegripProbabilityCurve(held time 0.1..MaxFireTimeForRegrip(0.8) -> 0..1) * Instability.
            crate::curve::eval(&REGRIP_CURVE, crate::status::map_range_clamped(now - start, 0.1, 0.8, 0.0, 1.0)) * instability
        };
        if rand::rng().random::<f32>() < weight {
            regrip.write(Regrip { delay: rand::rng().random_range(0.1..0.5) });
        }
        inv.last_fire_start = start;
        inv.fire_start = None;
    }
    if p.aiming || mouse.just_pressed(MouseButton::Left) {
        inv.cancel_bolt(); // GA_Character_Aim / GA_Fire_Base CancelAbilitiesWithTag Ability.Weapon.Reload.Bolt
    }
    let w = &inv.slots[inv.current];
    let trigger = if w.full_auto { mouse.pressed(MouseButton::Left) } else { mouse.just_pressed(MouseButton::Left) };
    if trigger && w.ammo == 0 && mouse.just_pressed(MouseButton::Left) && inv.cooldown <= 0.0 && !inv.busy() && !p.sprinting {
        dry.write(DryFire { row: w.name.clone() });
    }
    if !trigger || inv.cooldown > 0.0 || inv.busy() || p.sprinting || p.dead || w.ammo == 0 {
        return;
    }
    let s = w.stats.clone();
    let cur = inv.current;
    inv.slots[cur].ammo -= 1;
    inv.fire_start.get_or_insert(now);
    // ponytail: AC.PlayHandFatigue has no BP caller and HandFatigue stays 0 through every recorded firefight, so shooting adds none.
    inv.cooldown = s.fire_rate.unwrap_or(0.1);

    let gt = *cam;
    let (origin, aim_dir) = muzzle_basis(&gun, &joints, &parents, &kids, &gun_meshes, &mesh_assets, gt, p.aiming, &mut sights);
    debug!("shot dir vs camera: yaw {:.2} pitch {:.2} deg", (aim_dir.cross(*gt.forward()).y).asin().to_degrees(), (aim_dir.y.asin() - gt.forward().y.asin()).to_degrees());
    // SpreadX = max yaw, SpreadY = max pitch (GA_Fire_Base), full angles.
    let (sx, sy) = if p.aiming { (s.spread_aim_x, s.spread_aim_y) } else { (s.spread_hip_x, s.spread_hip_y) };
    let (sx, sy) = (sx.unwrap_or(0.0), sy.unwrap_or(0.0));
    let mut rng = rand::rng();
    for _ in 0..s.pellet_count.unwrap_or(1.0) as u32 {
        let dir = random_in_elliptical_cone(aim_dir, sx, sy, &mut rng);
        let hits: Vec<(Entity, Vec3, Vec3, f32)> = {
            let filter = |e: Entity| targets.get(e).is_ok_and(|t| !t.3);
            let settings = MeshRayCastSettings::default().with_filter(&filter).with_early_exit_test(&|_| false);
            ray.cast_ray(Ray3d::new(origin, Dir3::new(dir).unwrap()), &settings)
                .iter()
                .filter(|(_, h)| h.distance <= FIRE_RANGE)
                .map(|(e, h)| {
                    let t = targets.get(*e).unwrap();
                    // Damageable targets stop the bullet; marked surfaces use their table row.
                    let absorb = match (&t.1, &t.2) {
                        (Some(m), _) => surface_absorbance(m.0),
                        (None, Some(_)) => 0.0,
                        _ => DEFAULT_ABSORBANCE,
                    };
                    (*e, h.point, h.normal.normalize_or_zero(), absorb)
                })
                .collect()
        };
        let events = penetrate(&hits.iter().map(|h| (h.0, h.3)).collect::<Vec<_>>());
        let mut end = origin + dir * 200.0;
        for (i, mult) in &events {
            let (e, pt, n, absorb) = hits[*i];
            if let Ok((name, _, Some(mut hp), _)) = targets.get_mut(e) {
                hp.0 -= s.damage.unwrap_or(30.0) * mult * name.map_or(1.0, |n| bone_multiplier(n.as_str()));
                if hp.0 <= 0.0 {
                    cmd.entity(e).despawn();
                }
            }
            let surf = Surf::of(targets.get(e).ok().and_then(|t| t.1.map(|m| m.0)));
            impact_fx(&mut cmd, &fx, &mut mats, &mut soft, surf, pt, n);
            if absorb * mult <= MIN_BULLET_MULT {
                end = pt; // the trail stops where the bullet does
            }
        }
        // No trail: GA_Fire_Base.SpawnTrailEffect -> GCN_FireTrail -> WEP.SpawnTrailEffect, whose body is empty in the shipped game.
        let _ = end;
    }
    // WEP.PointLight: orange (1800 K), visible for 1.5 frames (RetriggerableDelay(1.5 * WorldDeltaSeconds)) on SpawnFireParticles.
    // UE places it 50 cm ahead of the gun; Intensity 40 cd there is not comparable to Bevy lumens (ponytail: 40k lm at 30 cm, tuned by eye: 200k at 50 cm blew the exposure out).
    cmd.spawn((PointLight { intensity: 40_000.0, range: 6.0, color: Color::srgb(1.0, 0.55, 0.2), ..default() }, Transform::from_translation(origin + aim_dir * 0.3), Fading(time.delta_secs() * 1.5)));
    // SpawnFireParticles: MuzzleSmoke ParticleSystem at BulletSpawnOrigin, scale = MapRange(Damage, 0..100 -> 0.1..2.0); skipped while aiming.
    // ponytail: the system asset (cosmetic tag, default per skin) is not in the dump; two drifting smoke sprites stand in.
    if !p.aiming && !suppressed {
        let scale = crate::status::map_range_clamped(s.damage.unwrap_or(30.0), 0.0, 100.0, 0.1, 2.0);
        for _ in 0..2 {
            let v = aim_dir * rng.random_range(0.3..0.8) + Vec3::new(rng.random_range(-0.1..0.1), rng.random_range(0.2..0.5), rng.random_range(-0.1..0.1));
            smoke(&mut cmd, &fx, &mut soft, origin + aim_dir * 0.05, v, 0.05 + 0.06 * scale, 1.0, 0.18, Color::srgb(0.4, 0.4, 0.4));
        }
    }
    // WEP.PullSliderLogic: on the shot, or on the Fire montage's PullSlider notify when Settings.bUsePullSliderNotify.
    let w = &inv.slots[cur];
    if w.anims["bUsePullSliderNotify"].as_bool().unwrap_or(false) {
        let notifies = w.one_of("Fire").map(|m| m.notifies).unwrap_or_default();
        inv.pull_slider.extend(notifies.iter().filter(|n| n.1 == "PullSlider_C").map(|n| n.0));
    } else {
        inv.pull_slider.push(0.0);
    }
    let w = &inv.slots[cur];
    fired.write(Fired { row: w.name.clone(), controller: ControllerRecoil::from(&w.anims), aiming: p.aiming, fire_rate: s.fire_rate.unwrap_or(0.1), suppressed });
}

/// Camera-facing smoke sprite (whisp_One sheet): grows 1.5x its size per second, alpha fades from `alpha` over `life`.
#[allow(clippy::too_many_arguments)]
fn smoke(cmd: &mut Commands, fx: &FireFx, mats: &mut Assets<crate::soft::SoftMat>, at: Vec3, vel: Vec3, size: f32, life: f32, alpha: f32, tint: Color) {
    let base = StandardMaterial { base_color_texture: Some(fx.smoke.clone()), base_color: tint.with_alpha(alpha), perceptual_roughness: 1.0, reflectance: 0.0, alpha_mode: AlphaMode::Blend, cull_mode: None, ..default() };
    // soft particle: fade over ~8 cm (or a third of the sprite) from surfaces behind it
    let mat = mats.add(crate::soft::SoftMat { base, extension: crate::soft::SoftExt { dist: std::env::var("BODYCAM_SOFT").ok().and_then(|v| v.parse().ok()).unwrap_or((size * 0.5).clamp(0.05, 0.3)) } });
    cmd.spawn((
        Mesh3d(fx.quad.clone()),
        MeshMaterial3d(mat.clone()),
        Transform::from_translation(at).with_scale(Vec3::splat(size)),
        Particle { vel, gravity: 0.0, grow: size * 1.5, age: 0.0, life, alpha, mat: Some(mat), billboard: true },
        Fading(life),
        crate::player::IgnoreTraces,
    ));
}

/// SpawnImpactEffect: Cubit_ImpactFX_Spawner picks the FXMaterialDataTable row by surface: decal (10 cm, random hole + roll) +
/// the row's Concrete/Wood/Metal_impact_Dyn Cascade system. Its emitters (counts/velocities are in unparsed distributions, so
/// the numbers below are ponytail: guesses): Dust x2 (whisp sprites), debris bits (gravity -1000 cm/s2 for concrete,
/// Spattering -690), Metal adds Sparks (gravity -960 cm/s2, enabled only in the metal row).
fn impact_fx(cmd: &mut Commands, fx: &FireFx, mats: &mut Assets<StandardMaterial>, soft: &mut Assets<crate::soft::SoftMat>, surf: Surf, pt: Vec3, n: Vec3) {
    if n == Vec3::ZERO {
        return;
    }
    let mut rng = rand::rng();
    let (i, dust, bit_col) = match surf {
        Surf::Concrete => (0, Color::srgb(0.4, 0.38, 0.36), Color::srgb(0.5, 0.48, 0.45)),
        Surf::Metal => (1, Color::srgb(0.35, 0.35, 0.35), Color::srgb(0.9, 0.85, 0.7)),
        Surf::Wood => (2, Color::srgb(0.55, 0.42, 0.28), Color::srgb(0.45, 0.3, 0.15)),
    };
    let (quads, dmat) = &fx.decals[i];
    cmd.spawn((
        Mesh3d(quads[rng.random_range(0..4)].clone()),
        MeshMaterial3d(dmat.clone()),
        // ponytail: decal life not in the dump (Cubit spawner lifespan unparsed); 30 s.
        Transform::from_translation(pt + n * 0.003).with_rotation(Quat::from_rotation_arc(Vec3::Z, n) * Quat::from_rotation_z(rng.random_range(0.0..std::f32::consts::TAU))),
        Fading(30.0),
        crate::player::IgnoreTraces,
    ));
    let jitter = |rng: &mut rand::rngs::ThreadRng| Vec3::new(rng.random_range(-1.0..1.0), rng.random_range(-1.0..1.0), rng.random_range(-1.0..1.0));
    for _ in 0..3 {
        smoke(cmd, fx, soft, pt + n * 0.02, n * rng.random_range(0.2..0.6) + jitter(&mut rng) * 0.2, 0.06, 0.6, 0.45, dust);
    }
    let (count, speed, g) = match surf {
        Surf::Metal => (8, 4.0, 9.6),
        _ => (6, 2.5, 10.0),
    };
    for _ in 0..count {
        let mat = mats.add(StandardMaterial { base_color: bit_col, emissive: if matches!(surf, Surf::Metal) { LinearRgba::rgb(6.0, 4.0, 1.5) } else { LinearRgba::BLACK }, unlit: matches!(surf, Surf::Metal), ..default() });
        let life = rng.random_range(0.3..0.7);
        cmd.spawn((
            Mesh3d(fx.bit.clone()),
            MeshMaterial3d(mat),
            Transform::from_translation(pt + n * 0.01),
            Particle { vel: (n + jitter(&mut rng) * 0.7).normalize_or_zero() * speed * rng.random_range(0.4..1.0), gravity: g, grow: 0.0, age: 0.0, life, alpha: 1.0, mat: None, billboard: false },
            Fading(life),
            crate::player::IgnoreTraces,
        ));
    }
}

fn particles(time: Res<Time>, cam: Single<&GlobalTransform, With<BodycamCamera>>, mut mats: ResMut<Assets<StandardMaterial>>, mut soft: ResMut<Assets<crate::soft::SoftMat>>, mut q: Query<(&mut Transform, &mut Particle)>) {
    let dt = time.delta_secs();
    for (mut t, mut p) in &mut q {
        p.vel.y -= p.gravity * dt;
        t.translation += p.vel * dt;
        t.scale += Vec3::splat(p.grow * dt);
        p.age += dt;
        if p.billboard {
            t.rotation = cam.rotation();
        }
        if let Some(mut m) = p.mat.as_ref().and_then(|h| soft.get_mut(h)) {
            m.base.base_color.set_alpha(p.alpha * (1.0 - p.age / p.life).clamp(0.0, 1.0));
        }
    }
}

/// Pistol_Eject Niagara shell (DT_Shell row Pistol, every gun): size 1.5, life 60 s, gravity -980 cm/s2.
#[derive(Component)]
struct Shell {
    vel: Vec3,
    spin: Vec3,
    age: f32,
    landed: bool,
}
const SHELL_LIFE: f32 = 60.0;
const SHELL_CAP: usize = 64;
const EJECT_SOCKETS: &str = include_str!("../../assets/eject_sockets.json");

thread_local! {
    /// gun -> (bone, FireLocation UE cm), baked by tools/fire_sockets.py.
    static FIRE_SOCKETS: HashMap<String, (String, Vec3)> = {
        let raw: HashMap<String, (String, f32, f32, f32)> = serde_json::from_str(include_str!("../../assets/fire_sockets.json")).unwrap_or_default();
        raw.into_iter().map(|(k, (b, x, y, z))| (k, (b, Vec3::new(x, y, z)))).collect()
    };
}

/// WEP.PullSliderLogic -> SpawnShell: at the gun's Eject socket with Pistol_Eject velocity (0, 50..75, 100..125) cm/s in socket space;
/// on the last round it also plays the gun-only FireOut montage.
// ponytail: spin read as rad/s.
fn eject_shells(
    mut cmd: Commands,
    time: Res<Time>,
    mut inv: Single<&mut Inventory>,
    fx: Res<FireFx>,
    guns: Query<(Entity, &GunRoot)>,
    joints: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    old: Query<(Entity, &Shell)>,
    mut out: MessageWriter<MontageStarted>,
) {
    let dt = time.delta_secs();
    for t in &mut inv.pull_slider {
        *t -= dt;
    }
    let due = inv.pull_slider.iter().filter(|t| **t <= 0.0).count();
    inv.pull_slider.retain(|t| *t > 0.0);
    if due == 0 {
        return;
    }
    let w = &inv.slots[inv.current];
    if w.ammo == 0
        && let Some(m) = w.one("FireOut")
    {
        out.write(MontageStarted { row: w.name.clone(), kind: ActionKind::FireOut, montage: m });
    }
    let Some((gun, g)) = guns.iter().next() else { return };
    let sockets: HashMap<String, [f32; 3]> = serde_json::from_str(EJECT_SOCKETS).unwrap_or_default();
    // Socket on the Root bone; guns without one (Glock17) carry it as a SOCKET_Eject bone.
    let (bone, loc) = match sockets.get(g.0) {
        Some(l) => ("root", Vec3::from(*l)),
        None => ("socket_eject", Vec3::ZERO),
    };
    let Some(j) = joints.iter().find(|(e, n)| n.as_str().eq_ignore_ascii_case(bone) && crate::viewmodel::root_of_entity(*e, gun, &parents)) else { return };
    let Ok(jt) = globals.get(j.0) else { return };
    let mut rng = rand::rng();
    let mut n = old.iter().count();
    // UE cm (x, y, z) -> joint frame (x, -y, z) m.
    let to_joint = |v: Vec3| Vec3::new(v.x, -v.y, v.z) * 0.01;
    for _ in 0..due {
        // WEP.SpawnShell spawns with the BulletSpawnOrigin rotation (X = bullet dir, Y = right, Z = up), so the Niagara
        // velocity (0, 50..75, 100..125) cm/s is right + up. Gun joint frame: right = -X, up = +Z.
        let vel = Vec3::new(-rng.random_range(50.0..75.0), 0.0, rng.random_range(100.0..125.0)) * 0.01;
        let spin = Vec3::new(rng.random_range(0.0..2.0), rng.random_range(0.0..2.0), rng.random_range(1.0..30.0));
        let mut t = jt.compute_transform();
        t.translation = jt.transform_point(to_joint(loc));
        t.scale = Vec3::splat(1.5);
        cmd.spawn((
            Mesh3d(fx.shell.0.clone()),
            MeshMaterial3d(fx.shell.1.clone()),
            t,
            Shell { vel: jt.rotation() * vel, spin, age: 0.0, landed: false },
            crate::player::IgnoreTraces,
        ));
        n += 1;
    }
    // Cap: drop the oldest.
    if n > SHELL_CAP {
        let mut v: Vec<_> = old.iter().collect();
        v.sort_by(|a, b| b.1.age.total_cmp(&a.1.age));
        for (e, _) in v.into_iter().take(n - SHELL_CAP) {
            cmd.entity(e).despawn();
        }
    }
}

/// Pistol_Eject collision -> DT_Shell row Pistol sound (Shell_Pistol_Cue), once per shell on its first floor hit.
#[derive(Message)]
pub struct ShellLanded;

/// Shell flight: gravity -980 cm/s2, spin, bounce on the floor (y = 0), despawn after SHELL_LIFE.
fn shells(mut cmd: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Transform, &mut Shell)>, mut landed: MessageWriter<ShellLanded>) {
    let dt = time.delta_secs();
    for (e, mut t, mut s) in &mut q {
        s.age += dt;
        if s.age > SHELL_LIFE {
            cmd.entity(e).despawn();
            continue;
        }
        if s.vel == Vec3::ZERO {
            continue; // settled
        }
        s.vel.y -= 9.8 * dt;
        t.translation += s.vel * dt;
        let spin = s.spin;
        t.rotate_local(Quat::from_scaled_axis(spin * dt));
        if t.translation.y < 0.005 {
            if !s.landed {
                s.landed = true;
                landed.write(ShellLanded);
            }
            t.translation.y = 0.005;
            s.vel.y = -s.vel.y * 0.35;
            s.vel.x *= 0.6;
            s.vel.z *= 0.6;
            s.spin *= 0.5;
            if s.vel.length() < 0.15 {
                s.vel = Vec3::ZERO;
            }
        }
    }
}

fn log_montages(mut ev: MessageReader<MontageStarted>) {
    for m in ev.read() {
        info!("{} {:?}: {} ({:.2}s)", m.row, m.kind, m.montage.path, m.montage.length);
    }
}

fn fade(mut cmd: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Fading)>) {
    for (e, mut f) in &mut q {
        f.0 -= time.delta_secs();
        if f.0 <= 0.0 {
            cmd.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::math::Vec3;
    #[test]
    fn weapons_have_anim_data() {
        let w = super::load_weapons();
        let m4 = w.iter().find(|w| w.name == "M4A1").expect("M4A1 row");
        let mc = m4.one("MagCheckFull").unwrap();
        assert!((mc.length - 4.4).abs() < 0.01);
        let (show, hide) = mc.mag_ui_window().unwrap();
        assert!(show > 0.5 && hide > 2.0 && hide < mc.length);
        assert!(m4.pick("ReloadEmptyCalm").unwrap().length > 3.0);
    }

    #[test]
    fn stats_inherit_default() {
        let s = super::load_stats();
        assert_eq!(s["M4A1"].damage, Some(35.0));
        assert_eq!(s["M4A1"].ammo_capacity, Some(30.0)); // inherited from GE_WeaponStat_Default
        assert_eq!(s["Mossberg"].pellet_count, Some(10.0));
        assert!(!s.contains_key("Default"));
    }

    #[test]
    fn elliptical_spread_stays_in_ellipse_and_is_wider_in_yaw() {
        let mut rng = rand::rng();
        let (mut sh, mut sv) = (0.0f32, 0.0f32);
        for _ in 0..20000 {
            let d = super::random_in_elliptical_cone(Vec3::NEG_Z, 6.0, 2.0, &mut rng);
            let (h, v) = (d.x.asin().to_degrees(), d.y.asin().to_degrees());
            assert!((h / 6.0).powi(2) + (v / 2.0).powi(2) < 1.05, "{h} {v}");
            sh += h * h;
            sv += v * v;
        }
        // UE VRandCone: the half angle only nears the yaw limit close to theta = 90 deg, so the variance ratio is ~3, not 9.
        assert!(sh > sv * 2.0, "yaw spread {sh} vs pitch {sv}");
        assert_eq!(super::random_in_elliptical_cone(Vec3::NEG_Z, 0.0, 0.0, &mut rng), Vec3::NEG_Z);
    }

    #[test]
    fn tactical_reload_returns_leftover_rounds() {
        let mut m4 = super::load_weapons().into_iter().find(|w| w.name == "M4A1").unwrap();
        m4.stats.tags.clear(); // every shipped gun is InfiniteMag; test the finite path
        m4.ammo = 10;
        m4.mags = vec![30, 30];
        m4.reload_done();
        assert_eq!((m4.ammo, m4.mags.clone()), (30, vec![30, 10]));
        m4.ammo = 0; // empty reload discards nothing
        m4.reload_done();
        assert_eq!((m4.ammo, m4.mags.clone()), (30, vec![10]));
        let mut inf = super::load_weapons().into_iter().find(|w| w.infinite_mags()).expect("an InfiniteMag gun");
        let n = inf.mags.len();
        inf.ammo = 0;
        inf.reload_done();
        assert_eq!(inf.mags.len(), n);
    }

    #[test]
    fn penetration_and_bone_multipliers() {
        // wood (0.6) x2: 1.0, 0.6, then 0.36 > 0.25 still hits, 0.216 stops.
        let m: Vec<f32> = super::penetrate(&[(1, 0.6), (1, 0.6), (2, 0.6), (3, 0.6), (4, 0.6)]).iter().map(|h| h.1).collect();
        assert_eq!(m.len(), 3);
        assert!((m[1] - 0.6).abs() < 1e-6 && (m[2] - 0.36).abs() < 1e-6);
        assert_eq!(super::penetrate(&[(1, 1.0), (2, 0.0)]).len(), 2);
        assert_eq!((super::bone_multiplier("head"), super::bone_multiplier("spine_02"), super::bone_multiplier("spine_01"), super::bone_multiplier("hand_r")), (10.0, 4.0, 2.5, 1.0));
    }
}
