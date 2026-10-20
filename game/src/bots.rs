//! Test-grid bots (grid.rs): capsule humanoids that patrol a route, notice the player (range + view cone + line of sight), turn, fire
//! bursts with spread (gunfight::BotGun: near misses, whiz, damage), strafe, die when shot (body 1x, head 10x via weapon::bone_multiplier)
//! and respawn after a delay. F7 hostile/passive, F10 respawn all.
// Navigation: grid A* over grid::Colliders (Colliders::path, walks ramps/stairs by the STEP rule), repathed every 0.7 s; chasing goes to the last seen position.
// ponytail: grid test world only - the real lobby has no bots; its map is a render-mesh ray-cast world (no block list to path over). Behaviour is a
// hand-written patrol/notice/burst state machine, not the game's BT_* (ALS_BT_AICharacter tasks AI_SimpleMoveTo/Temporisation/lookSpot... are not ported).
use crate::grid::Colliders;
use crate::gunfight::BotGun;
use crate::player::BodycamCamera;
use crate::weapon::Health;
use bevy::prelude::*;
use rand::Rng;

const SIGHT: f32 = 70.0; // m
/// AI_Humanoid.UpdateBotAccuracy: CurrentAccuracy = 100 deg with no kills, else MapRangeClamped(kills - trunc(1.5 * deaths), -5..10 -> 100..1.5).
fn bot_accuracy(kills: i32, deaths: i32) -> f32 {
    if kills == 0 { 100.0 } else { crate::status::map_range_clamped((kills - (deaths as f32 * 1.5) as i32) as f32, -5.0, 10.0, 100.0, 1.5) }
}
/// WEP.GetInitialFireDirection bot branch: max yaw = max pitch (deg) of the aim cone = SelectFloat(3, SelectFloat(CurrentAccuracy, 1, TargetIsPlayer), IsAiming).
fn fire_cone(aiming: bool, target_is_player: bool, accuracy: f32) -> f32 {
    if aiming { 3.0 } else if target_is_player { accuracy } else { 1.0 }
}
/// Test-bench stand-in for the match history behind PlayerStats.Kill (a fresh bot would have 100 deg of error and never hit): bots spawn with this many kills.
const SEED_KILLS: i32 = 9;
/// There is no player-death hook: a bot is credited a kill per PLAYER_HP of damage it lands (gunfight::DAMAGE per hit).
const HIT_DMG: f32 = 8.0;
const PLAYER_HP: f32 = 100.0;
const RESPAWN_DELAY: f32 = 6.0; // s after death
const TURN: f32 = 5.0; // rad/s
const REMEMBER: f32 = 3.0; // s a bot keeps facing the player after losing sight
const BLIND_S: f32 = 18.0; // GE_Flashed duration (see effects.rs)

pub struct BotSpec {
    /// Patrol waypoints (feet); the first is also the spawn point.
    pub route: Vec<Vec3>,
}

#[derive(Resource)]
struct Hostile(bool);

struct Slot {
    spec: BotSpec,
    e: Option<Entity>,
    timer: f32,
    kills: i32,
    deaths: i32,
}

type Art = (Handle<Mesh>, Handle<StandardMaterial>);

#[derive(Resource)]
struct Bots {
    slots: Vec<Slot>,
    art: [Art; 3], // body, head, gun
}

#[derive(Component)]
struct Bot {
    route: Vec<Vec3>,
    wp: usize,
    wait: f32,
    yaw: f32,
    parts: [Entity; 3], // body, head, gun
    los: bool,
    los_t: f32,
    aware: f32,
    burst_left: u32,
    next_shot: f32,
    next_burst: f32,
    strafe: f32,
    strafe_t: f32,
    path: Vec<Vec2>,
    goal: Vec2,
    path_t: f32,
    last_seen: Vec2,
    accuracy: f32,
    kills: i32,
    hit_dmg: f32,
    /// Seconds left blinded by a flashbang (no sight, no shooting, no moving).
    blind: f32,
}

pub fn setup(cmd: &mut Commands, meshes: &mut Assets<Mesh>, mats: &mut Assets<StandardMaterial>, specs: Vec<BotSpec>) {
    let mut m = |r: f32, g: f32, b: f32| mats.add(StandardMaterial { base_color: Color::srgb(r, g, b), perceptual_roughness: 0.8, ..default() });
    let art = [
        (meshes.add(Capsule3d::new(0.22, 1.0)), m(0.7, 0.12, 0.1)),
        (meshes.add(Sphere::new(0.13)), m(0.8, 0.6, 0.45)),
        (meshes.add(Cuboid::new(0.06, 0.08, 0.5)), m(0.08, 0.08, 0.08)),
    ];
    cmd.insert_resource(Bots { slots: specs.into_iter().map(|spec| Slot { spec, e: None, timer: 0.0, kills: SEED_KILLS, deaths: 0 }).collect(), art });
    cmd.insert_resource(Hostile(true));
}

pub struct BotsPlugin;
impl Plugin for BotsPlugin {
    fn build(&self, app: &mut App) {
        if crate::grid::wanted() {
            app.add_systems(Update, (keys, lifecycle, think).chain().run_if(resource_exists::<Bots>));
        }
    }
}

fn keys(k: Res<ButtonInput<KeyCode>>, mut hostile: ResMut<Hostile>, mut bots: ResMut<Bots>, mut cmd: Commands) {
    if k.just_pressed(KeyCode::F7) {
        hostile.0 = !hostile.0;
        info!("bots {}", if hostile.0 { "hostile" } else { "passive" });
    }
    if k.just_pressed(KeyCode::F10) {
        info!("bots respawn");
        for s in &mut bots.slots {
            if let Some(e) = s.e.take() {
                cmd.entity(e).despawn();
            }
            s.timer = 0.0;
        }
    }
}

/// A bot dies when its body or head entity is gone (the player's fire() despawns entities whose Health hits 0).
fn lifecycle(time: Res<Time>, mut bots: ResMut<Bots>, mut cmd: Commands, alive: Query<&Bot>, parts: Query<(), With<Mesh3d>>, xf: Query<&GlobalTransform>) {
    let art = bots.art.clone();
    for s in &mut bots.slots {
        if let Some(e) = s.e {
            if alive.get(e).is_ok_and(|b| b.parts[..2].iter().all(|&p| parts.contains(p))) {
                continue;
            }
            if let Ok(b) = alive.get(e) {
                s.kills = b.kills;
            }
            s.deaths += 1;
            // WW_RagdollSystem.BloodPool (death): puddle under the body after 5 s. ponytail: bots have no ragdoll, so no velocity re-check.
            if let Ok(g) = xf.get(e) {
                crate::blood::request_pool(&mut cmd, g.translation() + Vec3::Y, 5.0);
            }
            cmd.entity(e).despawn();
            s.e = None;
            s.timer = RESPAWN_DELAY;
        }
        s.timer -= time.delta_secs();
        if s.timer > 0.0 {
            continue;
        }
        let e = cmd.spawn((Transform::from_translation(s.spec.route[0]), Visibility::default())).id();
        let mut ids = [Entity::PLACEHOLDER; 3];
        cmd.entity(e).with_children(|c| {
            ids[0] = c.spawn((Mesh3d(art[0].0.clone()), MeshMaterial3d(art[0].1.clone()), Health(100.0), Name::new("body"), Transform::from_xyz(0.0, 0.72, 0.0))).id();
            ids[1] = c.spawn((Mesh3d(art[1].0.clone()), MeshMaterial3d(art[1].1.clone()), Health(100.0), Name::new("head"), Transform::from_xyz(0.0, 1.6, 0.0))).id();
            ids[2] = c.spawn((Mesh3d(art[2].0.clone()), MeshMaterial3d(art[2].1.clone()), Transform::from_xyz(0.2, 1.1, -0.35))).id();
        });
        cmd.entity(e).insert(Bot {
            route: s.spec.route.clone(),
            wp: 1 % s.spec.route.len(),
            wait: 0.0,
            yaw: 0.0,
            parts: ids,
            los: false,
            los_t: 0.0,
            aware: 0.0,
            burst_left: 0,
            next_shot: 0.0,
            next_burst: 0.0,
            strafe: 0.0,
            strafe_t: 0.0,
            path: vec![],
            goal: Vec2::splat(f32::MAX),
            path_t: 0.0,
            last_seen: Vec2::new(s.spec.route[0].x, s.spec.route[0].z),
            accuracy: bot_accuracy(s.kills, s.deaths),
            kills: s.kills,
            hit_dmg: 0.0,
            blind: 0.0,
        });
        s.e = Some(e);
        s.timer = RESPAWN_DELAY; // the Bot component shows up next frame; until then `alive` would read it as dead
    }
}

fn think(
    time: Res<Time>,
    hostile: Res<Hostile>,
    col: Res<Colliders>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    mut bots: Query<(Entity, &mut Bot, &mut Transform)>,
    mut gun: BotGun,
    mut flash: MessageReader<crate::effects::Flashbang>,
) {
    let (dt, now) = (time.delta_secs(), time.elapsed_secs());
    let target = cam.translation();
    let mut rng = rand::rng();
    // GameplayCue.Player.Flashed on a bot: same distance falloff as the player (250..1000 cm -> 1..0) x GE_Flashed 18 s, needs a clear line to the burst.
    // ponytail: the bot AI's flash reaction is not in the dump; blinded bots simply stop (no sight, shots or movement).
    let bursts: Vec<Vec3> = flash.read().map(|f| f.at).collect();
    for (e, mut b, mut tf) in &mut bots {
        let own = [e, b.parts[0], b.parts[1], b.parts[2]];
        let eye = tf.translation + Vec3::Y * 1.5;
        for at in &bursts {
            let k = crate::status::map_range_clamped(eye.distance(*at) * 100.0, 250.0, 1000.0, 1.0, 0.0);
            if k > 0.0 && gun.sees(&own, eye, *at) {
                b.blind = b.blind.max(BLIND_S * k);
                info!("bot flashed: {:.1} s", b.blind);
            }
        }
        if b.blind > 0.0 {
            b.blind -= dt;
            b.los = false;
            b.aware = 0.0;
            b.burst_left = 0;
            continue;
        }
        let to = target - eye;
        let flat = Vec2::new(to.x, to.z);
        // Notice: range + view cone (or already aware / very close) + line of sight, re-checked 5x per second.
        b.los_t -= dt;
        if b.los_t <= 0.0 {
            b.los_t = 0.2;
            let in_view = to.length() < 6.0 || b.aware > 0.0 || tf.forward().dot(to.normalize_or_zero()) > 0.2;
            let was = b.los;
            b.los = hostile.0 && to.length() < SIGHT && in_view && gun.sees(&own, eye, target);
            if b.los && !was {
                b.next_burst = now + rng.random_range(0.4..0.9); // reaction time
            }
        }
        if !hostile.0 {
            b.los = false;
        }
        b.aware = if b.los { REMEMBER } else { b.aware - dt };
        let engaged = hostile.0 && b.aware > 0.0;

        // Move + face: engaged = face the player, close/back off to 6..18 m and strafe; otherwise patrol the route.
        // Out of sight or far away: walk the A* path to the last seen position; patrol also follows paths (rooms, doorways, ramps).
        let here = Vec2::new(tf.translation.x, tf.translation.z);
        if b.los {
            b.last_seen = Vec2::new(target.x, target.z);
        }
        let mut dir = Vec2::ZERO;
        let mut face = None;
        let mut goal = None;
        if engaged {
            let d = flat.normalize_or_zero();
            if b.los && flat.length() <= 18.0 {
                face = Some(d);
                b.strafe_t -= dt;
                if b.strafe_t <= 0.0 {
                    b.strafe_t = rng.random_range(1.0..2.5);
                    b.strafe = rng.random_range(-1..=1) as f32;
                }
                let range = if flat.length() < 6.0 { -0.7 } else { 0.0 };
                dir = d * range * 2.0 + Vec2::new(-d.y, d.x) * b.strafe * 1.5;
            } else {
                goal = Some((b.last_seen, 2.0));
            }
        } else if b.wait > 0.0 {
            b.wait -= dt;
        } else {
            let wp = b.route[b.wp];
            let wp2 = Vec2::new(wp.x, wp.z);
            if here.distance(wp2) < 0.3 {
                b.wait = rng.random_range(1.0..3.0);
                b.wp = (b.wp + 1) % b.route.len();
            } else {
                goal = Some((wp2, 1.1));
            }
        }
        if let Some((g, speed)) = goal {
            b.path_t -= dt;
            if b.path_t <= 0.0 || g.distance(b.goal) > 1.0 {
                b.path = col.path(here, tf.translation.y, g, 0.3);
                b.goal = g;
                b.path_t = 0.7;
            }
            while b.path.first().is_some_and(|p| p.distance(here) < 0.35) {
                b.path.remove(0);
            }
            if let Some(p) = b.path.first() {
                let d = (*p - here).normalize_or_zero();
                dir = d * speed;
                face = Some(d);
            } else if !engaged && here.distance(g) > 1.0 {
                b.wp = (b.wp + 1) % b.route.len(); // unreachable waypoint: skip it
            }
        } else {
            b.path.clear();
        }
        if engaged && b.los {
            face = Some(flat.normalize_or_zero());
        }
        let mut pos = tf.translation + Vec3::new(dir.x, 0.0, dir.y) * dt;
        let y = pos.y;
        col.push_out(&mut pos, 0.3, y, None);
        pos.y = col.ground(Vec2::new(pos.x, pos.z), pos.y);
        tf.translation = pos;
        let mut off = 0.0;
        if let Some(f) = face {
            use std::f32::consts::{PI, TAU};
            off = ((-f.x).atan2(-f.y) - b.yaw + PI).rem_euclid(TAU) - PI;
            b.yaw += off.clamp(-TURN * dt, TURN * dt);
            tf.rotation = Quat::from_rotation_y(b.yaw);
        }

        // Fire bursts (3-6 rounds, 0.12 s apart) while the player is in sight and roughly in front.
        if !(b.los && off.abs() < 0.3) {
            continue;
        }
        if b.burst_left == 0 {
            if now >= b.next_burst {
                b.burst_left = rng.random_range(3..=6);
                b.next_shot = now;
            }
            continue;
        }
        if now < b.next_shot {
            continue;
        }
        b.next_shot = now + 0.12;
        b.burst_left -= 1;
        if b.burst_left == 0 {
            b.next_burst = now + rng.random_range(1.5..3.5);
        }
        let muzzle = tf.translation + Vec3::Y * 1.2 + *tf.forward() * 0.7;
        // Aiming = standing still (no strafe, not chasing). Target is always the player here.
        let cone = fire_cone(dir == Vec2::ZERO, true, b.accuracy);
        if gun.fire(&own, muzzle, target, cone) {
            b.hit_dmg += HIT_DMG;
            if b.hit_dmg >= PLAYER_HP {
                b.hit_dmg -= PLAYER_HP;
                b.kills += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accuracy_follows_ue_branches() {
        assert_eq!(bot_accuracy(0, 5), 100.0);
        assert!((bot_accuracy(10, 0) - 1.5).abs() < 1e-4);
        assert!((bot_accuracy(1, 2) - 80.3).abs() < 0.1); // 1 - trunc(3) = -2 -> 100 - 3/15 * 98.5
        assert_eq!(fire_cone(true, true, 40.0), 3.0);
        assert_eq!(fire_cone(false, true, 40.0), 40.0);
        assert_eq!(fire_cone(false, false, 40.0), 1.0);
    }
}
