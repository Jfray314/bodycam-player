//! Blood, ported from the real game's assets (sizes in UE cm converted to m, UE Z-up -> Bevy Y-up).
//! - Flesh hit = FXMaterialDataTable_GenericBullets SurfaceType8, spawned by Cubit_ImpactFX_Spawner (bp2txt of the BP):
//!   Decal_Blood_Splatter_Main (Splasher_1 2x2 atlas, random Frame) of DecalSize (3,8,8) cm x rand(0.3..0.4) on the hit component,
//!   BloodCloud_Droplets_A Cascade system (values below are the exported modules), Blood_Dripping SoundCue, and after 0.2 s the
//!   BackSplatter: 1.2 m trace in a 10 deg cone along the shot, RearSpatter decal (3,10,10) cm x lerp(rand(2,3), rand(3,3.5), dist 50..150 cm).
//! - BloodCloud_Droplets_A LOD0: Dust (Whisps_trans2) burst 5, life 0.25-0.75, vel local (+-50,+-50,0..1200) cm/s, drag 2-7, accel world
//!   (+-60, +-60, -250..-100), colour (0.0529,0,0); Source (invisible) burst 5, life 0.75-1.0, vel (+-50,+-50,100..350), gravity 960, collides
//!   with world (freeze), CollisionEvent -> SpawnParticleDecals: Decal_Blood_Splatter_Main half-extent 5 cm x rand(1.5..3.5), random Frame/roll.
//! - WW_RagdollSystem (bp2txt): AddImpulse -> NS_BloodSquirt_Big (Pouring_Life 0, Splatter_SpriteSize rand 0.1-0.2, Dripping_SpriteScale 0.3,
//!   Dripping_Velocity = MapRange(5 m trace, 50..100 cm -> 10..25)) + NS_Dripping_Slow bleeding (rate 50/s at health<=20, SpriteSize 0.6);
//!   BloodPool (on death): re-check every 5 s until RagdollVelocityLength <= 15, trace 5 m down from spine_03/head, NS_BloodPuddle2/3/4
//!   (Puddle_Life 100, Puddle_Size rand 75..135).
//! ponytail (not recoverable from the cooked assets): the Niagara emitter internals (spawn counts, curves live in compiled VM bytecode),
//! the puff opacity (Whisps_trans2 material), the Dust size axes (X/Y differ; a square of their geometric mean is used), the DustTrail emitter
//! (decodes to 2.5-6 m sprites growing x6.5; skipped), the ribbon trails (streaks stand in), puddle growth timing, and whisp frame (smoke.png).
use crate::audio::PlayCue;
use crate::player::{BodycamCamera, IgnoreTraces};
use crate::ragdoll::Ragdoll;
use crate::soft::{SoftExt, SoftMat};
use crate::weapon::{Fading, FireFx, Health};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use bevy_rapier3d::prelude::Velocity;
use rand::Rng;
use std::f32::consts::TAU;

const BLOOD_CUE: &str = "/Game/BallisticsVFX/SFX/Impacts/Blood_Dripping";
const DECAL_LIFE: f32 = 100.0; // Cubit_ImpactFX_Spawner 'Decal Life'
const GRAVITY: f32 = 9.6; // Source emitter acceleration 960 cm/s2
/// Puff opacity scale. ponytail: Whisps_trans2's opacity is unknown (the alpha curve peaks at 2.9).
const PUFF_OPACITY: f32 = 0.5;
/// ColorOverLife is (0.0529, 0, 0) but that renders black-brown here; the material's colour scale is unknown, so a visible dark red.
const PUFF_RED: f32 = 0.3;
/// ParticleModuleColorOverLife_3 alpha (16 samples over the life).
const PUFF_ALPHA: [f32; 16] = [2.93, 2.46, 1.91, 1.37, 0.90, 0.59, 0.51, 0.50, 0.46, 0.39, 0.31, 0.22, 0.14, 0.07, 0.02, 0.0];
const BLEED_RATE: f32 = 12.0; // drops/s. ponytail: Dripping_spawnRate is 50/s; capped for the per-drop ray casts
const BLEED_SECS: f32 = 10.0;
const POOL_LIFE: f32 = 100.0;

#[derive(Resource)]
pub struct BloodFx {
    frames: Vec<Handle<Mesh>>,
    splat: Vec<Handle<Mesh>>,
    decal: Handle<StandardMaterial>,
    splat_mat: Handle<StandardMaterial>,
    bit: Handle<Mesh>,
    bit_mat: Handle<StandardMaterial>,
    unit: Handle<Mesh>,
    pools: Vec<Handle<StandardMaterial>>,
}

/// Spawn an entity with this from the fire / remote-shot code; `flesh_hits` turns it into the whole effect.
#[derive(Component)]
pub struct FleshHit {
    pub pt: Vec3,
    pub n: Vec3,
    pub dir: Vec3,
    pub hit: Entity,
}

#[derive(Component)]
struct Backsplat {
    t: f32,
    pt: Vec3,
    dir: Vec3,
    skip: Entity,
}
#[derive(Component)]
struct Puff {
    vel: Vec3,
    drag: f32,
    accel: Vec3,
    age: f32,
    life: f32,
    size: f32,
    spin: f32,
    spin_rate: f32,
}
/// Falling blood: lands on the first solid surface, leaving a decal (`splat` = flipbook splatter of that side, else a Splasher decal).
#[derive(Component)]
struct Drop {
    vel: Vec3,
    age: f32,
    life: f32,
    side: f32,
    splat: bool,
}
#[derive(Component)]
struct PoolReq {
    from: Vec3,
    t: f32,
}
#[derive(Component)]
struct Pool {
    age: f32,
    size: f32,
}
#[derive(Component)]
struct Bleed {
    t: f32,
    acc: f32,
    next_pool: f32,
    pooled: bool,
    last: Vec3,
}

pub struct BloodPlugin;
impl Plugin for BloodPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, (flesh_hits, backsplat, puffs, drops, pool_reqs, pools, ragdoll_bleed));
    }
}

/// `n` x `n` flipbook quad (unit size) showing one cell, same vertex order / uv as weapon::atlas_quad.
fn grid_quad(meshes: &mut Assets<Mesh>, n: u32, ix: u32, iy: u32) -> Handle<Mesh> {
    let mut m = Rectangle::new(1.0, 1.0).mesh().build();
    let k = n as f32;
    let uv: Vec<[f32; 2]> = [[1.0, 0.0], [0.0, 0.0], [0.0, 1.0], [1.0, 1.0]].iter().map(|u| [(ix as f32 + u[0]) / k, (iy as f32 + u[1]) / k]).collect();
    m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    meshes.add(m)
}

fn wet(tex: Handle<Image>, color: Color) -> StandardMaterial {
    StandardMaterial { base_color_texture: Some(tex), base_color: color, perceptual_roughness: 0.25, reflectance: 0.5, alpha_mode: AlphaMode::Blend, cull_mode: None, ..default() }
}

fn setup(mut cmd: Commands, assets: Res<AssetServer>, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let red = Color::srgb(0.3, 0.012, 0.01);
    cmd.insert_resource(BloodFx {
        frames: (0..4).map(|i| grid_quad(&mut meshes, 2, i % 2, i / 2)).collect(),
        splat: (0..9).map(|i| grid_quad(&mut meshes, 3, i % 3, i / 3)).collect(),
        decal: mats.add(wet(assets.load("textures/fx/decal_blood.png"), red)),
        splat_mat: mats.add(wet(assets.load("textures/fx/blood_splat_3x3.png"), red)),
        bit: meshes.add(Cuboid::new(0.006, 0.006, 0.05)),
        bit_mat: mats.add(StandardMaterial { base_color: Color::srgb(0.25, 0.008, 0.008), perceptual_roughness: 0.2, ..default() }),
        unit: meshes.add(Rectangle::new(1.0, 1.0)),
        pools: (2..=4).map(|i| mats.add(wet(assets.load(format!("textures/fx/blood_pool_{i}.png")), Color::srgb(0.2, 0.006, 0.005)))).collect(),
    });
}

fn decal_tf(pt: Vec3, n: Vec3, side: f32, rng: &mut impl Rng) -> Transform {
    Transform::from_translation(pt + n * 0.002).with_rotation(Quat::from_rotation_arc(Vec3::Z, n) * Quat::from_rotation_z(rng.random_range(0.0..TAU))).with_scale(Vec3::new(side, side, 1.0))
}

fn spawn_decal(cmd: &mut Commands, bf: &BloodFx, pt: Vec3, n: Vec3, side: f32, parent: Option<&GlobalTransform>, hit: Entity, rng: &mut impl Rng) {
    let tf = decal_tf(pt, n, side, rng);
    let bundle = (Mesh3d(bf.frames[rng.random_range(0..4)].clone()), MeshMaterial3d(bf.decal.clone()), Fading(DECAL_LIFE), IgnoreTraces);
    // SpawnDecalAttached: rides the hit component (a dead bot takes its decal with it).
    if let Some(p) = parent {
        let id = cmd.spawn((bundle, GlobalTransform::from(tf).reparented_to(p))).id();
        cmd.entity(hit).add_child(id);
    } else {
        cmd.spawn((bundle, tf));
    }
}

fn spawn_splat(cmd: &mut Commands, bf: &BloodFx, pt: Vec3, n: Vec3, side: f32, rng: &mut impl Rng) {
    cmd.spawn((Mesh3d(bf.splat[rng.random_range(0..9)].clone()), MeshMaterial3d(bf.splat_mat.clone()), decal_tf(pt, n, side, rng), Fading(DECAL_LIFE), IgnoreTraces));
}

fn flesh_hits(
    mut cmd: Commands,
    hits: Query<(Entity, &FleshHit)>,
    bf: Res<BloodFx>,
    fx: Res<FireFx>,
    mut soft: ResMut<Assets<SoftMat>>,
    mut cue: MessageWriter<PlayCue>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    gts: Query<&GlobalTransform>,
    mut ray: MeshRayCast,
    solid: Query<(), (Without<IgnoreTraces>, Without<Health>)>,
) {
    let mut rng = rand::rng();
    for (e, h) in &hits {
        cmd.entity(e).despawn();
        let q = Quat::from_rotation_arc(Vec3::Y, h.n);
        // Decal_Blood_Splatter_Main: DecalSize.y 8 cm x rand(0.3, 0.4), half extent -> full side.
        spawn_decal(&mut cmd, &bf, h.pt, h.n, 2.0 * 0.08 * rng.random_range(0.3..0.4), gts.get(h.hit).ok(), h.hit, &mut rng);
        cue.write(PlayCue { cue: BLOOD_CUE.into(), dist_cm: cam.translation().distance(h.pt) * 100.0, bools: vec![] });
        // Dust x5
        for _ in 0..5 {
            let w = rng.random_range(0.395..0.9f32);
            let hgt = rng.random_range(1.25..3.0f32);
            cmd.spawn((
                Mesh3d(fx.quad.clone()),
                MeshMaterial3d(soft.add(SoftMat { base: puff_base(&fx), extension: SoftExt { dist: 0.1 } })),
                Transform::from_translation(h.pt + h.n * 0.02).with_scale(Vec3::ZERO),
                Puff {
                    vel: q * Vec3::new(rng.random_range(-0.5..0.5), rng.random_range(0.0..12.0), rng.random_range(-0.5..0.5)),
                    drag: rng.random_range(2.0..7.0),
                    accel: Vec3::new(rng.random_range(-0.6..0.6), rng.random_range(-2.5..-1.0), rng.random_range(-0.6..0.6)),
                    age: 0.0,
                    life: rng.random_range(0.25..0.75),
                    size: 0.5 * (w * hgt).sqrt(),
                    spin: rng.random_range(0.0..TAU),
                    spin_rate: rng.random_range(-0.2..0.2) * TAU,
                },
                IgnoreTraces,
            ));
        }
        // Source droplets x5 (their ribbon trails drawn as streaks); landing -> 5 cm x rand(1.5, 3.5) splash decal
        for _ in 0..5 {
            let v = q * Vec3::new(rng.random_range(-0.5..0.5), rng.random_range(1.0..3.5), rng.random_range(-0.5..0.5));
            spawn_drop(&mut cmd, &bf, h.pt + h.n * 0.01, v, rng.random_range(0.75..1.0), 0.0, false);
        }
        // NS_BloodSquirt_Big drips: Dripping_Velocity from a 5 m trace along the shot.
        let d_cm = ray
            .cast_ray(Ray3d::new(h.pt, Dir3::new(h.dir).unwrap_or(Dir3::Y)), &MeshRayCastSettings::default().with_filter(&|e| solid.contains(e)))
            .first()
            .map_or(500.0, |(_, hit)| hit.distance * 100.0)
            .min(500.0);
        let dv = crate::status::map_range_clamped(d_cm, 50.0, 100.0, 10.0, 25.0) * 0.01;
        for _ in 0..4 {
            let v = h.dir * dv + Vec3::new(rng.random_range(-0.05..0.05), rng.random_range(-0.05..0.0), rng.random_range(-0.05..0.05));
            spawn_drop(&mut cmd, &bf, h.pt + h.n * 0.01, v, 3.0, rng.random_range(0.1..0.2) * 0.3, true);
        }
        cmd.spawn(Backsplat { t: 0.2, pt: h.pt, dir: h.dir, skip: h.hit });
    }
}

fn puff_base(fx: &FireFx) -> StandardMaterial {
    StandardMaterial { base_color_texture: Some(fx.smoke.clone()), base_color: Color::linear_rgba(PUFF_RED, 0.01, 0.01, 0.0), perceptual_roughness: 1.0, reflectance: 0.0, alpha_mode: AlphaMode::Blend, cull_mode: None, ..default() }
}

fn spawn_drop(cmd: &mut Commands, bf: &BloodFx, pos: Vec3, vel: Vec3, life: f32, side: f32, splat: bool) {
    cmd.spawn((Mesh3d(bf.bit.clone()), MeshMaterial3d(bf.bit_mat.clone()), Transform::from_translation(pos), Drop { vel, age: 0.0, life, side, splat }, IgnoreTraces));
}

fn backsplat(
    mut cmd: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Backsplat)>,
    bf: Res<BloodFx>,
    mut ray: MeshRayCast,
    solid: Query<(), (Without<IgnoreTraces>, Without<Health>)>,
) {
    let mut rng = rand::rng();
    for (e, mut b) in &mut q {
        b.t -= time.delta_secs();
        if b.t > 0.0 {
            continue;
        }
        cmd.entity(e).despawn();
        // RandomUnitVectorInEllipticalConeInDegrees(10, 10) x 120 cm, ignoring the hit actor.
        let dir = Dir3::new(b.dir).unwrap_or(Dir3::NEG_Z);
        let cos = rng.random_range(10f32.to_radians().cos()..1.0);
        let axis = Quat::from_axis_angle(*dir, rng.random_range(0.0..TAU)) * dir.any_orthonormal_vector();
        let d = Quat::from_axis_angle(axis, cos.acos()) * *dir;
        let skip = b.skip;
        let filter = |e: Entity| e != skip && solid.contains(e);
        let Some((_, hit)) = ray.cast_ray(Ray3d::new(b.pt, Dir3::new(d).unwrap_or(dir)), &MeshRayCastSettings::default().with_filter(&filter)).first().cloned() else { continue };
        if hit.distance > 1.2 {
            continue;
        }
        let d_cm = hit.distance * 100.0;
        let s = crate::status::map_range_clamped(d_cm, 50.0, 150.0, rng.random_range(2.0..3.0), rng.random_range(3.0..3.5));
        let n = hit.normal.normalize_or_zero();
        if n == Vec3::ZERO {
            continue;
        }
        let pt = hit.point - d * 0.04 * (d_cm / 200.0).clamp(0.0, 1.0);
        let mut tf = decal_tf(pt, n, 2.0 * 0.10 * s, &mut rng);
        tf.translation = pt + n * 0.002;
        cmd.spawn((Mesh3d(bf.frames[rng.random_range(0..4)].clone()), MeshMaterial3d(bf.decal.clone()), tf, Fading(DECAL_LIFE), IgnoreTraces));
    }
}

fn puffs(mut cmd: Commands, time: Res<Time>, cam: Single<&GlobalTransform, With<BodycamCamera>>, mut soft: ResMut<Assets<SoftMat>>, mut q: Query<(Entity, &mut Transform, &mut Puff, &MeshMaterial3d<SoftMat>)>) {
    let dt = time.delta_secs();
    for (e, mut t, mut p, m) in &mut q {
        p.age += dt;
        if p.age >= p.life {
            cmd.entity(e).despawn();
            continue;
        }
        let a = p.accel;
        p.vel += a * dt;
        let k = (-p.drag * dt).exp();
        p.vel *= k;
        t.translation += p.vel * dt;
        let f = p.age / p.life;
        // ParticleModuleSizeMultiplyLife_1: 0 -> 0.5 over the first ~0.1 of the life
        t.scale = Vec3::splat(p.size * (f / 0.1).min(1.0));
        p.spin += p.spin_rate * dt;
        t.rotation = cam.rotation() * Quat::from_rotation_z(p.spin);
        if let Some(mut mat) = soft.get_mut(&m.0) {
            let x = f * 15.0;
            let i = (x as usize).min(14);
            let al = PUFF_ALPHA[i] + (PUFF_ALPHA[i + 1] - PUFF_ALPHA[i]) * (x - i as f32);
            mat.base.base_color.set_alpha((al * PUFF_OPACITY).clamp(0.0, 1.0));
        }
    }
}

fn drops(
    mut cmd: Commands,
    time: Res<Time>,
    bf: Res<BloodFx>,
    mut q: Query<(Entity, &mut Transform, &mut Drop)>,
    mut ray: MeshRayCast,
    solid: Query<(), (Without<IgnoreTraces>, Without<Health>)>,
) {
    let dt = time.delta_secs();
    let mut rng = rand::rng();
    for (e, mut t, mut d) in &mut q {
        d.age += dt;
        if d.age >= d.life {
            cmd.entity(e).despawn();
            continue;
        }
        d.vel.y -= GRAVITY * dt;
        let step = d.vel * dt;
        let len = step.length();
        if len > 1e-6
            && let Ok(dir) = Dir3::new(step)
            && let Some((_, hit)) = ray.cast_ray(Ray3d::new(t.translation, dir), &MeshRayCastSettings::default().with_filter(&|e| solid.contains(e))).first().cloned()
            && hit.distance <= len
        {
            let n = hit.normal.normalize_or_zero();
            if n != Vec3::ZERO {
                if d.splat {
                    spawn_splat(&mut cmd, &bf, hit.point, n, d.side, &mut rng);
                } else {
                    // ParticleDecalSize (0.01, 5, 5) cm half extents x rand(1.5, 3.5)
                    let side = 2.0 * 0.05 * rng.random_range(1.5..3.5);
                    cmd.spawn((Mesh3d(bf.frames[rng.random_range(0..4)].clone()), MeshMaterial3d(bf.decal.clone()), decal_tf(hit.point, n, side, &mut rng), Fading(DECAL_LIFE), IgnoreTraces));
                }
            }
            cmd.entity(e).despawn();
            continue;
        }
        t.translation += step;
        if let Ok(dir) = Dir3::new(d.vel) {
            t.rotation = Quat::from_rotation_arc(Vec3::Z, *dir);
        }
    }
}

/// BloodPool: trace 5 m down from `from`, spawn a random NS_BloodPuddle2/3/4 after `delay` s.
pub fn request_pool(cmd: &mut Commands, from: Vec3, delay: f32) {
    cmd.spawn(PoolReq { from, t: delay });
}

fn pool_reqs(mut cmd: Commands, time: Res<Time>, mut q: Query<(Entity, &mut PoolReq)>, bf: Res<BloodFx>, mut ray: MeshRayCast, solid: Query<(), (Without<IgnoreTraces>, Without<Health>)>) {
    let mut rng = rand::rng();
    for (e, mut r) in &mut q {
        r.t -= time.delta_secs();
        if r.t > 0.0 {
            continue;
        }
        cmd.entity(e).despawn();
        let Some((_, hit)) = ray.cast_ray(Ray3d::new(r.from, Dir3::NEG_Y), &MeshRayCastSettings::default().with_filter(&|e| solid.contains(e))).first().cloned() else { continue };
        if hit.distance > 5.0 {
            continue;
        }
        let n = hit.normal.normalize_or_zero();
        let n = if n == Vec3::ZERO { Vec3::Y } else { n };
        cmd.spawn((
            Mesh3d(bf.unit.clone()),
            MeshMaterial3d(bf.pools[rng.random_range(0..3)].clone()),
            Transform::from_translation(hit.point + n * 0.003).with_rotation(Quat::from_rotation_arc(Vec3::Z, n) * Quat::from_rotation_z(rng.random_range(0.0..TAU))).with_scale(Vec3::ZERO),
            Pool { age: 0.0, size: rng.random_range(0.75..1.35) },
            Fading(POOL_LIFE),
            IgnoreTraces,
        ));
    }
}

/// ponytail: Puddle_Size is the final size; the growth curve is in compiled Niagara data, so it spreads ease-out over 15 s from 20 %.
fn pools(time: Res<Time>, mut q: Query<(&mut Transform, &mut Pool)>) {
    for (mut t, mut p) in &mut q {
        p.age += time.delta_secs();
        let k = (p.age / 15.0).min(1.0);
        t.scale = Vec3::new(p.size, p.size, 1.0) * (0.2 + 0.8 * (1.0 - (1.0 - k) * (1.0 - k)));
    }
}

/// Dead ragdoll: the AddImpulse squirt at the knock, NS_Dripping_Slow bleeding from spine_03, then BloodPool once it settles.
fn ragdoll_bleed(
    mut cmd: Commands,
    time: Res<Time>,
    bf: Res<BloodFx>,
    mut q: Query<(Entity, &Ragdoll, Option<&mut Bleed>)>,
    stale: Query<(Entity, &Bleed), Without<Ragdoll>>,
    parts: Query<(&Transform, &Velocity)>,
) {
    for (e, b) in &stale {
        // The corpse is gone (respawn) before it settled: UE's body would stay and pool where it lay.
        if !b.pooled {
            request_pool(&mut cmd, b.last, 0.0);
        }
        cmd.entity(e).remove::<Bleed>();
    }
    let dt = time.delta_secs();
    let mut rng = rand::rng();
    for (e, rag, bleed) in &mut q {
        let Some(&spine) = rag.bodies.get("spine_03") else { continue };
        let Ok((tf, vel)) = parts.get(spine) else { continue };
        let Some(mut b) = bleed else {
            cmd.entity(e).insert(Bleed { t: 0.0, acc: 0.0, next_pool: 0.0, pooled: false, last: tf.translation });
            continue;
        };
        b.t += dt;
        b.last = tf.translation;
        if b.t < BLEED_SECS {
            b.acc += dt * BLEED_RATE;
            while b.acc >= 1.0 {
                b.acc -= 1.0;
                let v = Vec3::new(rng.random_range(-0.15..0.15), rng.random_range(-0.1..0.1), rng.random_range(-0.15..0.15));
                // Splatter_SpriteSize 0.6 at health <= 0 (x Dripping_SpriteScale 0.3)
                spawn_drop(&mut cmd, &bf, tf.translation + Vec3::new(rng.random_range(-0.1..0.1), 0.0, rng.random_range(-0.1..0.1)), v, 3.0, 0.6 * 0.3, true);
            }
        }
        if !b.pooled && b.t >= b.next_pool {
            if vel.linear.length() * 100.0 > 15.0 {
                b.next_pool += 5.0; // RagdollVelocityLength > 15 -> Delay 5.0
            } else {
                b.pooled = true;
                let from = rag.bodies.get(if rng.random_bool(0.5) { "head" } else { "spine_03" }).and_then(|&h| parts.get(h).ok()).map_or(tf.translation, |p| p.0.translation);
                request_pool(&mut cmd, from, 0.0);
            }
        }
    }
}
