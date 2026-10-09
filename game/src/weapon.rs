//! Port of GA_Fire_Base / GA_Reload_Base / GA_Character_Aim / GA_Character_SwapItem,
//! driven by WeaponAttributeSet values pulled from GE_WeaponStat_* (assets/weapon_stats.json).
use crate::player::{BodycamCamera, Player};
use bevy::input::mouse::MouseWheel;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use rand::Rng;
use serde::Deserialize;
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

pub struct Weapon {
    pub name: String,
    pub stats: Stats,
    pub ammo: u32,
    pub mags: u32,
    pub full_auto: bool,
}

impl Weapon {
    pub fn new(name: &str, stats: &HashMap<String, Stats>) -> Self {
        let s = stats[name].clone();
        Weapon {
            name: name.into(),
            ammo: s.ammo_capacity.unwrap_or(30.0) as u32,
            mags: s.mag_amount.unwrap_or(4.0) as u32,
            full_auto: s.fire_modes.iter().any(|m| m == "FullAuto"),
            stats: s,
        }
    }
    fn cap(&self) -> u32 {
        self.stats.ammo_capacity.unwrap_or(30.0) as u32
    }
}

#[derive(Component)]
pub struct Inventory {
    pub slots: Vec<Weapon>,
    pub current: usize,
    pub cooldown: f32,
    /// Remaining reload time. ponytail: flat 2.2s until montage lengths are pulled from W_FP_*_Reload*.
    pub reloading: f32,
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
        app.add_systems(Startup, setup_fx).add_systems(PostStartup, give_loadout).add_systems(Update, (swap, aim, fire, reload, fade));
    }
}

fn give_loadout(mut cmd: Commands, player: Single<Entity, With<Player>>) {
    let stats = load_stats();
    // Default loadout: primary + sidearm, then every other gun for testing.
    let mut names: Vec<String> = vec!["M4A1".into(), "Glock17".into()];
    let mut rest: Vec<String> = stats.keys().filter(|k| !names.contains(k)).cloned().collect();
    rest.sort();
    names.extend(rest);
    let slots = names.iter().map(|n| Weapon::new(n, &stats)).collect();
    cmd.entity(*player).insert(Inventory { slots, current: 0, cooldown: 0.0, reloading: 0.0 });
}

fn swap(keys: Res<ButtonInput<KeyCode>>, mut inv: Single<&mut Inventory>, mut wheel: MessageReader<MouseWheel>) {
    let n = inv.slots.len();
    let mut next = inv.current;
    for (i, k) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3, KeyCode::Digit4, KeyCode::Digit5].iter().enumerate() {
        if keys.just_pressed(*k) && i < n {
            next = i;
        }
    }
    for w in wheel.read() {
        next = if w.y < 0.0 { (next + 1) % n } else { (next + n - 1) % n };
    }
    if next != inv.current {
        inv.current = next;
        inv.reloading = 0.0;
        inv.cooldown = 0.5; // GE_State_Swapping
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
) {
    let (inv, p) = &mut *q;
    inv.cooldown -= time.delta_secs();
    let w = &inv.slots[inv.current];
    let trigger = if w.full_auto { mouse.pressed(MouseButton::Left) } else { mouse.just_pressed(MouseButton::Left) };
    if !trigger || inv.cooldown > 0.0 || inv.reloading > 0.0 || p.sprinting || w.ammo == 0 {
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
        ));
    }
    // Muzzle flash
    cmd.spawn((
        PointLight { intensity: 200_000.0, color: Color::srgb(1.0, 0.8, 0.5), ..default() },
        Transform::from_translation(origin + gt.forward() * 0.6),
        Fading(0.05),
    ));
    // Instability -> camera kick. ponytail: linear scale, real curves live in W_FP_*_Fire anims.
    let kick = s.instability.unwrap_or(0.5) * 0.02;
    p.recoil += Vec2::new(rng.random_range(-kick..=kick) * 0.3, kick);
}

fn reload(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut inv: Single<&mut Inventory>) {
    let cur = inv.current;
    if inv.reloading > 0.0 {
        inv.reloading -= time.delta_secs();
        if inv.reloading <= 0.0 {
            let w = &mut inv.slots[cur];
            w.ammo = w.cap();
            w.mags = w.mags.saturating_sub(1);
        }
        return;
    }
    let w = &inv.slots[cur];
    if keys.just_pressed(KeyCode::KeyR) && w.ammo < w.cap() && w.mags > 0 {
        inv.reloading = 2.2;
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
    fn stats_inherit_default() {
        let s = super::load_stats();
        assert_eq!(s["M4A1"].damage, Some(35.0));
        assert_eq!(s["M4A1"].ammo_capacity, Some(30.0)); // inherited from GE_WeaponStat_Default
        assert_eq!(s["Mossberg"].pellet_count, Some(10.0));
        assert!(!s.contains_key("Default"));
    }
}
