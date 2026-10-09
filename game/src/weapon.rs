//! Port of GA_Fire_Base / GA_Reload_Base / GA_MagCheck / GA_Inspect / GA_Character_Aim / GA_Character_SwapItem.
//! Stats: WeaponAttributeSet from GE_WeaponStat_* (assets/weapon_stats.json). Weapons, action montages and
//! notify times: DT_WeaponItems rows + BAS_* animation sets (assets/weapons/<Row>.json, tools/weapon_data.py).
//! Inputs follow IMC_KBM_CharacterOnFoot: R tap (<0.25 s, IA_Reload) reloads, R hold 0.25 s (IA_InspectAmmos)
//! mag-checks, J inspects, 1-6 / wheel swap, LMB fire, RMB aim.
use crate::player::{BodycamCamera, Player};
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
}

impl Montage {
    fn from(v: &Value) -> Option<Self> {
        let c = v.get("Character").unwrap_or(v);
        Some(Montage {
            path: c.get("path")?.as_str()?.into(),
            length: c.get("length")?.as_f64()? as f32,
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
    pub mags: u32,
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
}

#[derive(Clone, Debug)]
pub struct Action {
    pub kind: ActionKind,
    pub montage: Montage,
    pub t: f32,
    /// Slot to draw after a holster.
    next_slot: usize,
}

/// PRAS RecoilData.ControllerRecoil: per-shot (min, max) steps in degrees and the RecoilSmoothing interp speed.
#[derive(Clone, Debug)]
pub struct ControllerRecoil {
    pub horizontal: (f32, f32),
    pub vertical: (f32, f32),
    pub smoothing: f32,
}

impl ControllerRecoil {
    fn from(anims: &Value) -> Self {
        let c = &anims["RecoilData"]["ControllerRecoil"];
        let r = |k: &str| (c[k]["x"].as_f64().unwrap_or(0.0) as f32, c[k]["y"].as_f64().unwrap_or(0.0) as f32);
        let (lo, hi) = r("HorizontalRecoilStep");
        let (vlo, vhi) = r("VerticalRecoilStep");
        ControllerRecoil { horizontal: (lo.min(hi), lo.max(hi)), vertical: (vlo.min(vhi), vlo.max(vhi)), smoothing: r("RecoilSmoothing").1.max(1.0) }
    }
}

/// One shot fired by the player (camera recoil/shake, audio, viewmodel recoil).
#[derive(Message, Clone, Debug)]
pub struct Fired {
    pub row: String,
    pub controller: ControllerRecoil,
    pub aiming: bool,
}

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
}

impl Inventory {
    pub fn busy(&self) -> bool {
        self.action.is_some()
    }
    fn start(&mut self, kind: ActionKind, montage: Option<Montage>, next_slot: usize, out: &mut MessageWriter<MontageStarted>) {
        let Some(montage) = montage else { return };
        out.write(MontageStarted { row: self.slots[self.current].name.clone(), kind, montage: montage.clone() });
        self.action = Some(Action { kind, montage, t: 0.0, next_slot });
    }
}

#[derive(Component)]
pub struct Health(pub f32);

#[derive(Component)]
struct Fading(f32);

#[derive(Resource)]
struct TracerFx(Handle<Mesh>, Handle<StandardMaterial>);

fn setup_fx(mut cmd: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let mat = StandardMaterial { base_color: Color::srgb(1.0, 0.9, 0.5), emissive: LinearRgba::rgb(8.0, 6.0, 2.0), unlit: true, ..default() };
    cmd.insert_resource(TracerFx(meshes.add(Cuboid::new(0.01, 0.01, 1.0)), mats.add(mat)));
}

pub struct WeaponPlugin;
impl Plugin for WeaponPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<MontageStarted>()
            .add_message::<Fired>()
            .add_message::<DryFire>()
            .add_systems(Startup, setup_fx)
            .add_systems(PostStartup, give_loadout)
            .add_systems(Update, ((actions, swap, aim, fire, fade).chain(), log_montages));
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
            mags: s.mag_amount.unwrap_or(4.0) as u32,
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
    slots.sort_by_key(|w| (w.name != "M4A1", w.name != "Glock17", w.name.clone()));
    let mut inv = Inventory { slots, current: 0, cooldown: 0.0, action: None, r_held: 0.0 };
    let first = inv.slots[0].one("FirstEquip");
    inv.slots[0].equipped_once = true;
    inv.start(ActionKind::Draw, first, 0, &mut out);
    cmd.entity(*player).insert(inv);
}

/// Advances the current action; handles R tap/hold and J.
fn actions(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut inv: Single<&mut Inventory>, mut out: MessageWriter<MontageStarted>) {
    let dt = time.delta_secs();
    if let Some(a) = &mut inv.action {
        a.t += dt;
        if a.t >= a.montage.length {
            let (kind, next) = (a.kind, a.next_slot);
            inv.action = None;
            let cur = inv.current;
            match kind {
                // GE_AddAmmo on completion. ponytail: no chambered-round bonus on tactical reloads.
                ActionKind::Reload => {
                    let w = &mut inv.slots[cur];
                    w.ammo = w.cap();
                    w.mags = w.mags.saturating_sub(1);
                }
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
        if tapped && !inv.busy() && w.ammo < w.cap() && w.mags > 0 {
            let m = if w.ammo == 0 { w.pick("ReloadEmptyCalm") } else { w.pick("ReloadTacCalm") };
            let m = m.or_else(|| w.pick("ReloadEmptyCalm"));
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
fn aim(mouse: Res<ButtonInput<MouseButton>>, mut p: Single<&mut Player>) {
    p.aiming = mouse.pressed(MouseButton::Right);
}

fn fire(
    mut cmd: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut q: Single<(&mut Inventory, &mut Player)>,
    cam: Single<(Entity, &GlobalTransform), With<BodycamCamera>>,
    mut ray: MeshRayCast,
    mut targets: Query<&mut Health>,
    fx: Res<TracerFx>,
    mut fired: MessageWriter<Fired>,
    mut dry: MessageWriter<DryFire>,
) {
    let (inv, p) = &mut *q;
    inv.cooldown -= time.delta_secs();
    let w = &inv.slots[inv.current];
    let trigger = if w.full_auto { mouse.pressed(MouseButton::Left) } else { mouse.just_pressed(MouseButton::Left) };
    if trigger && w.ammo == 0 && mouse.just_pressed(MouseButton::Left) && inv.cooldown <= 0.0 && !inv.busy() && !p.sprinting {
        dry.write(DryFire { row: w.name.clone() });
    }
    if !trigger || inv.cooldown > 0.0 || inv.busy() || p.sprinting || w.ammo == 0 {
        return;
    }
    let s = w.stats.clone();
    let cur = inv.current;
    inv.slots[cur].ammo -= 1;
    inv.cooldown = s.fire_rate.unwrap_or(0.1);

    let (cam_e, gt) = *cam;
    let (sx, sy) = if p.aiming { (s.spread_aim_x, s.spread_aim_y) } else { (s.spread_hip_x, s.spread_hip_y) };
    let (sx, sy) = (sx.unwrap_or(0.0).to_radians() * 0.5, sy.unwrap_or(0.0).to_radians() * 0.5);
    let mut rng = rand::rng();
    let origin = gt.translation();
    for _ in 0..s.pellet_count.unwrap_or(1.0) as u32 {
        let jitter = Quat::from_euler(EulerRot::YXZ, rng.random_range(-sx..=sx), rng.random_range(-sy..=sy), 0.0);
        let dir = gt.rotation() * jitter * Vec3::NEG_Z;
        let filter = |e: Entity| e != cam_e;
        let settings = MeshRayCastSettings::default().with_filter(&filter);
        let hit = ray.cast_ray(Ray3d::new(origin, Dir3::new(dir).unwrap()), &settings).first().map(|(e, h)| (*e, h.point));
        let end = hit.map(|(_, pt)| pt).unwrap_or(origin + dir * 200.0);
        if let Some((e, _)) = hit
            && let Ok(mut h) = targets.get_mut(e)
        {
            h.0 -= s.damage.unwrap_or(30.0);
            if h.0 <= 0.0 {
                cmd.entity(e).despawn();
            }
        }
        let len = origin.distance(end);
        cmd.spawn((
            Mesh3d(fx.0.clone()),
            MeshMaterial3d(fx.1.clone()),
            Transform::from_translation((origin + end) / 2.0).looking_at(end, Vec3::Y).with_scale(Vec3::new(1.0, 1.0, len)),
            Fading(0.04),
            crate::player::IgnoreTraces,
        ));
    }
    // Muzzle flash
    cmd.spawn((
        PointLight { intensity: 200_000.0, color: Color::srgb(1.0, 0.8, 0.5), ..default() },
        Transform::from_translation(origin + gt.forward() * 0.6),
        Fading(0.05),
    ));
    let w = &inv.slots[cur];
    fired.write(Fired { row: w.name.clone(), controller: ControllerRecoil::from(&w.anims), aiming: p.aiming });
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
}
