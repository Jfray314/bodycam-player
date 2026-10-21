//! Gunfight test bench (not a port): an AI shooter downrange fires bursts at the player to exercise the stress
//! system (near misses -> BulletFly +0.3, hits -> PlayerDamage) and the messy-shooting layers.
//! F6 toggles it; BODYCAM_GUNFIGHT=1 starts it enabled. The player can shoot it (weapon::Health); it respawns after 3 s.
use crate::player::{BodycamCamera, IgnoreTraces};
use crate::status::{BulletFly, NearMiss, PlayerDamage, SurvivorStatus};
use crate::weapon::{Fading, Health, FireFx};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use rand::Rng;

const SHOOTER_DIST: f32 = 13.0; // m in front of the player when it spawns
const SPREAD_DEG: f32 = 2.5; // half-angle of the aim error cone
const HIT_RADIUS: f32 = 0.3; // m, around the camera
const WHIZ_RADIUS: f32 = 3.0; // m, closest approach that still counts as a near miss
const MAX_WHIZZ_BY_RADIUS: f32 = 4.0; // m, GA_Fire_Base.MaxWhizzByRadius 400 cm: crack + whiz audio
const SHOOTER_WEAPON: &str = "M4A1"; // sound set of the shooter's gun
const DAMAGE: f32 = 8.0;

#[derive(Resource)]
pub struct Gunfight {
    pub enabled: bool,
    respawn_in: f32,
    log_in: f32,
}

#[derive(Component)]
struct Shooter {
    burst_left: u32,
    next_shot: f32,
    next_burst: f32,
}

pub struct GunfightPlugin;
impl Plugin for GunfightPlugin {
    fn build(&self, app: &mut App) {
        let enabled = std::env::var("BODYCAM_GUNFIGHT").is_ok_and(|v| v == "1");
        app.insert_resource(Gunfight { enabled, respawn_in: 0.0, log_in: 1.0 }).add_systems(Update, (toggle, spawn_shooter, shoot).chain());
    }
}

fn toggle(keys: Res<ButtonInput<KeyCode>>, mut g: ResMut<Gunfight>, mut cmd: Commands, shooters: Query<Entity, With<Shooter>>) {
    if keys.just_pressed(KeyCode::F6) {
        g.enabled = !g.enabled;
        info!("gunfight {}", if g.enabled { "on" } else { "off" });
        if !g.enabled {
            shooters.iter().for_each(|e| cmd.entity(e).despawn());
        }
    }
}

/// (Re)spawns the shooter 3 s after it dies (the player's fire() despawns entities whose Health hits 0).
fn spawn_shooter(
    time: Res<Time>,
    mut g: ResMut<Gunfight>,
    mut cmd: Commands,
    shooters: Query<(), With<Shooter>>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    if !g.enabled || !shooters.is_empty() {
        g.respawn_in = 3.0;
        return;
    }
    g.respawn_in -= time.delta_secs();
    if g.respawn_in > 0.0 {
        return;
    }
    cmd.spawn((
        Shooter { burst_left: 0, next_shot: 0.0, next_burst: time.elapsed_secs() + 1.5 },
        Health(100.0),
        Mesh3d(meshes.add(Capsule3d::new(0.25, 1.1))),
        MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::srgb(0.7, 0.12, 0.1), perceptual_roughness: 0.8, ..default() })),
        Transform::from_translation(cam.translation() + (cam.forward().with_y(0.0).normalize_or(Vec3::NEG_Z) * SHOOTER_DIST) - Vec3::Y * 0.8),
    ));
}

fn shoot(
    time: Res<Time>,
    mut g: ResMut<Gunfight>,
    status: Res<SurvivorStatus>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    mut shooters: Query<(Entity, &mut Shooter, &mut Transform)>,
    mut gun: BotGun,
) {
    let now = time.elapsed_secs();
    if g.enabled {
        g.log_in -= time.delta_secs();
        if g.log_in <= 0.0 {
            g.log_in = 1.0;
            info!("gunfight stress={:.2} health={:.0}", status.stress, status.health);
        }
    }
    let target = cam.translation();
    let mut rng = rand::rng();
    for (e, mut s, mut tf) in &mut shooters {
        let muzzle = tf.translation + Vec3::Y * 0.5;
        let y = tf.translation.y;
        tf.look_at(Vec3::new(target.x, y, target.z), Vec3::Y);
        if s.burst_left == 0 {
            if now >= s.next_burst {
                s.burst_left = rng.random_range(3..=6);
                s.next_shot = now;
            }
            continue;
        }
        if now < s.next_shot {
            continue;
        }
        s.next_shot = now + 0.12;
        s.burst_left -= 1;
        if s.burst_left == 0 {
            s.next_burst = now + rng.random_range(2.0..4.0);
        }
        let _ = gun.fire(&[e], muzzle, target, SPREAD_DEG);
    }
}

/// Shared AI gun (gunfight Shooter and grid bots): one shot from `muzzle` at `target` with a `spread_deg` aim-error cone.
/// Near misses -> NearMiss/BulletFly, hits -> PlayerDamage, plus a tracer. `own` entities are skipped by the ray cast.
#[derive(SystemParam)]
pub struct BotGun<'w, 's> {
    cmd: Commands<'w, 's>,
    ignore: Query<'w, 's, (), With<IgnoreTraces>>,
    ray: MeshRayCast<'w, 's>,
    fx: Res<'w, FireFx>,
    dmg: MessageWriter<'w, PlayerDamage>,
    whiz: MessageWriter<'w, BulletFly>,
    near: MessageWriter<'w, NearMiss>,
}

impl BotGun<'_, '_> {
    /// Clear line from `from` to `to` (ignoring `own` and tracers)?
    pub fn sees(&mut self, own: &[Entity], from: Vec3, to: Vec3) -> bool {
        let Ok(dir) = Dir3::new(to - from) else { return true };
        let filter = |x: Entity| !own.contains(&x) && !self.ignore.contains(x);
        let settings = MeshRayCastSettings::default().with_filter(&filter);
        self.ray.cast_ray(Ray3d::new(from, dir), &settings).first().is_none_or(|(_, h)| h.distance > from.distance(to) - 0.3)
    }

    /// Returns whether the shot hit the player (damage written).
    pub fn fire(&mut self, own: &[Entity], muzzle: Vec3, target: Vec3, spread_deg: f32) -> bool {
        let mut rng = rand::rng();
        // WEP.GetInitialFireDirection (bot branch): aim at the target, RandomUnitVectorInEllipticalConeInDegrees(spread, spread).
        let aim = (target - muzzle).normalize();
        let dir = crate::weapon::random_in_elliptical_cone(aim, spread_deg, spread_deg, &mut rng);
        let mut hit = false;
        let filter = |x: Entity| !own.contains(&x) && !self.ignore.contains(x);
        let settings = MeshRayCastSettings::default().with_filter(&filter);
        let wall = self.ray.cast_ray(Ray3d::new(muzzle, Dir3::new(dir).unwrap()), &settings).first().map(|(_, h)| h.distance);
        // Closest approach to the player along the ray.
        let to = target - muzzle;
        let t = to.dot(dir);
        let miss = (to - dir * t).length();
        let reaches = t > 0.0 && wall.is_none_or(|w| w > t - 0.1);
        let len = if reaches { t.max(0.5) } else { wall.unwrap_or(60.0) };
        if reaches {
            if miss <= MAX_WHIZZ_BY_RADIUS {
                self.near.write(NearMiss { row: SHOOTER_WEAPON.into(), origin_dist_cm: to.length() * 100.0, closest_dist_cm: miss * 100.0 });
            }
            if miss < HIT_RADIUS {
                self.dmg.write(PlayerDamage(DAMAGE, Some(crate::status::hit_push(dir, target, DAMAGE, 35.0, 2100.0)))); // M4A1 Damage / RagdollForce
                hit = true;
            } else if miss < WHIZ_RADIUS {
                self.whiz.write(BulletFly { distance_cm: miss * 100.0 });
            }
        }
        let end = muzzle + dir * if reaches { t + 6.0 } else { len };
        self.cmd.spawn((
            Mesh3d(self.fx.tracer.0.clone()),
            MeshMaterial3d(self.fx.tracer.1.clone()),
            Transform::from_translation((muzzle + end) / 2.0).looking_at(end, Vec3::Y).with_scale(Vec3::new(1.0, 1.0, muzzle.distance(end))),
            Fading(0.06),
            IgnoreTraces,
        ));
        hit
    }
}
