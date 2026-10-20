//! Grenade explosion VFX: the Cascade systems Explosion_Frag_small (G67_C) / Explosion_Flash_small (Flash_C), spawned by
//! SpawnEmitterAtLocation at scale 0.5, plus G67_C's 51 upward line traces that spawn Cubit_ImpactFX_Spawner (FXSize 2) and Flash_C's
//! 0.1 s PointLight (Intensity 30000, Temperature 1800 K). Emitter data is baked by tools/vfx_export.py from the uasset dump into
//! assets/vfx/*.json (RawDistribution lookup tables: op 1 = time lookup, op 2 = uniform min/max pairs; or editor curves).
//! Active emitters: Spark burst (M_Flare star, 5 x 0.1 s), Sparks (M_Radial_Gradient, 150 velocity-aligned streaks), Smoke_expand (Whisps_trans
//! 8x8 flipbook, 10 puffs). The Dust / second Sparks emitters are event-receivers of a disabled collision emitter: they never spawn.
use crate::player::IgnoreTraces;
use crate::soft::{SoftExt, SoftMat};
use bevy::asset::RenderAssetUsages;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use rand::Rng;
use serde::Deserialize;

/// SpawnEmitterAtLocation scale (0.5, 0.5, 0.5): sizes and Velocity-module speeds.
const SCALE: f32 = 0.5;
/// Cascade ColorOverLife is HDR (up to 200) and M_Flare / M_Radial_Gradient's graphs are not in the dump: scale to the game's exposure.
// ponytail: tuned by eye (no white frames at point-blank), not derived from the material graphs.
const HDR: f32 = 0.1;
/// Whisps_trans is lit (directional master); the game's exposure washes grey smoke out against the bright map: darken to match.
const SMOKE_TONE: f32 = 0.6;

/// An explosion's visuals at `at` (world, m).
#[derive(Message)]
pub struct ExplosionFx {
    pub at: Vec3,
    pub flash: bool,
}

#[derive(Deserialize, Clone, Default)]
struct Dist {
    #[serde(default)]
    op: u32,
    #[serde(default)]
    n: usize,
    #[serde(default)]
    stride: usize,
    #[serde(default)]
    sub: usize,
    #[serde(default)]
    ts: f32,
    #[serde(default)]
    tb: f32,
    #[serde(default)]
    v: Vec<f32>,
    /// Editor curve points [t, x(, y, z)], linear.
    curve: Option<Vec<Vec<f32>>>,
}

impl Dist {
    /// Evaluate at life fraction `t` into up to 3 components; uniform distributions draw each component from `rng`.
    fn eval(&self, t: f32, rng: &mut impl Rng) -> [f32; 3] {
        let mut o = [0.0; 3];
        if let Some(c) = &self.curve {
            let k = c.first().map_or(0, |p| p.len() - 1);
            let i = c.iter().position(|p| p[0] > t).unwrap_or(c.len());
            for j in 0..k.min(3) {
                o[j] = if i == 0 {
                    c[0][1 + j]
                } else if i == c.len() {
                    c[i - 1][1 + j]
                } else {
                    let (a, b) = (&c[i - 1], &c[i]);
                    a[1 + j] + (b[1 + j] - a[1 + j]) * (t - a[0]) / (b[0] - a[0]).max(1e-6)
                };
            }
            return o;
        }
        let stride = self.stride.max(1);
        let f = if self.n > 1 { ((t - self.tb) * self.ts).clamp(0.0, (self.n - 1) as f32) } else { 0.0 };
        let (i0, fr) = (f.floor() as usize, f.fract());
        let i1 = (i0 + 1).min(self.n.max(1) - 1);
        let at = |i: usize, j: usize| self.v.get(i * stride + j).copied().unwrap_or(0.0);
        let lerp = |j: usize| at(i0, j) + (at(i1, j) - at(i0, j)) * fr;
        if self.op == 2 {
            for j in 0..self.sub.min(3) {
                o[j] = lerp(j) + (lerp(self.sub + j) - lerp(j)) * rng.random::<f32>();
            }
        } else {
            for j in 0..stride.min(3) {
                o[j] = lerp(j);
            }
        }
        o
    }
    fn scalar(&self, t: f32, rng: &mut impl Rng) -> f32 {
        self.eval(t, rng)[0]
    }
}

#[derive(Deserialize, Default)]
struct Emitter {
    material: String,
    align: String,
    sub: [u32; 2],
    bursts: Vec<(u32, f32)>,
    #[serde(default)]
    event_only: bool,
    lifetime: Option<Dist>,
    size: Option<Dist>,
    size_life: Option<Dist>,
    velocity: Option<Dist>,
    color: Option<Dist>,
    alpha: Option<Dist>,
    rotation: Option<Dist>,
    subuv: Option<Dist>,
    accel_const: Option<[f32; 3]>,
    accel: Option<Dist>,
    drag: Option<Dist>,
}

#[derive(Deserialize)]
struct System {
    emitters: Vec<Emitter>,
}

enum Mat {
    Std(Handle<StandardMaterial>),
    Soft(Handle<SoftMat>),
}

#[derive(Component)]
struct Particle {
    sys: usize,
    em: usize,
    age: f32,
    life: f32,
    vel: Vec3,
    acc: Vec3,
    drag: f32,
    size0: Vec3,
    rot: f32,
    mat: Mat,
}

#[derive(Resource)]
struct Fx {
    quad: Handle<Mesh>,
    flare: Handle<Image>,
    smoke: Handle<Image>,
    grad: Handle<Image>,
    systems: [System; 2],
}

/// UE (X fwd, Y right, Z up; cm) -> Bevy world (m).
fn ue(v: [f32; 3]) -> Vec3 {
    Vec3::new(v[1], v[2], -v[0]) * 0.01
}

fn setup(mut cmd: Commands, assets: Res<AssetServer>, mut meshes: ResMut<Assets<Mesh>>, mut images: ResMut<Assets<Image>>) {
    // M_Radial_Gradient: white, alpha falling off radially (procedural in the game).
    let n = 64usize;
    let px: Vec<u8> = (0..n * n)
        .flat_map(|i| {
            let (x, y) = ((i % n) as f32 / (n - 1) as f32 * 2.0 - 1.0, (i / n) as f32 / (n - 1) as f32 * 2.0 - 1.0);
            let a = (1.0 - (x * x + y * y).sqrt()).clamp(0.0, 1.0);
            [255, 255, 255, (a * a * 255.0) as u8]
        })
        .collect();
    let grad = images.add(Image::new(Extent3d { width: n as u32, height: n as u32, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD));
    let load = |s: &str| serde_json::from_str::<System>(s).expect("assets/vfx json");
    cmd.insert_resource(Fx {
        quad: meshes.add(Rectangle::new(1.0, 1.0)),
        flare: assets.load("textures/fx/flare_star.png"),
        smoke: assets.load("textures/fx/whisp_one_8x8.png"),
        grad,
        systems: [load(include_str!("../../assets/vfx/Explosion_Frag_small.json")), load(include_str!("../../assets/vfx/Explosion_Flash_small.json"))],
    });
}

#[allow(clippy::too_many_arguments)]
fn spawn(
    mut cmd: Commands,
    mut ev: MessageReader<ExplosionFx>,
    fx: Res<Fx>,
    mut std: ResMut<Assets<StandardMaterial>>,
    mut soft: ResMut<Assets<SoftMat>>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    surf: Query<Option<&crate::weapon::SurfaceMat>>,
    weapon_fx: Res<crate::weapon::FireFx>,
) {
    let mut rng = rand::rng();
    // dev: BODYCAM_FX_LIFE=N stretches particle lifetimes N times (to inspect single frames)
    let life_mul = std::env::var("BODYCAM_FX_LIFE").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(1.0);
    for e in ev.read() {
        let sys = e.flash as usize;
        for (ei, em) in fx.systems[sys].emitters.iter().enumerate() {
            if em.event_only {
                continue;
            }
            let count: u32 = em.bursts.iter().filter(|b| b.1 <= 0.0).map(|b| b.0).sum(); // ponytail: only time-0 bursts exist in these systems
            for _ in 0..count {
                let life = em.lifetime.as_ref().map_or(1.0, |d| d.scalar(0.0, &mut rng)).max(0.01);
                let life = life * life_mul;
                let s = em.size.as_ref().map_or([100.0; 3], |d| d.eval(0.0, &mut rng));
                let v = em.velocity.as_ref().map_or([0.0; 3], |d| d.eval(0.0, &mut rng));
                let a = em.accel.as_ref().map_or([0.0; 3], |d| d.eval(0.0, &mut rng));
                let mat = if em.material.starts_with("Whisps") {
                    let base = StandardMaterial { base_color_texture: Some(fx.smoke.clone()), perceptual_roughness: 1.0, reflectance: 0.0, alpha_mode: AlphaMode::Blend, cull_mode: None, ..default() };
                    Mat::Soft(soft.add(SoftMat { base, extension: SoftExt { dist: (s[0] * SCALE * 0.005).clamp(0.05, 0.3) } }))
                } else if em.material == "M_Flare" {
                    Mat::Std(std.add(StandardMaterial { base_color_texture: Some(fx.flare.clone()), unlit: true, alpha_mode: AlphaMode::Add, cull_mode: None, ..default() }))
                } else {
                    Mat::Std(std.add(StandardMaterial { base_color_texture: Some(fx.grad.clone()), unlit: true, alpha_mode: AlphaMode::Blend, cull_mode: None, ..default() }))
                };
                let handle: (Option<Handle<StandardMaterial>>, Option<Handle<SoftMat>>) = match &mat {
                    Mat::Std(h) => (Some(h.clone()), None),
                    Mat::Soft(h) => (None, Some(h.clone())),
                };
                let mut c = cmd.spawn((
                    Mesh3d(fx.quad.clone()),
                    Transform::from_translation(e.at).with_scale(Vec3::ZERO),
                    Particle {
                        sys,
                        em: ei,
                        age: 0.0,
                        life,
                        vel: ue(v) * SCALE,
                        acc: ue(a) + em.accel_const.map_or(Vec3::ZERO, ue),
                        drag: em.drag.as_ref().map_or(0.0, |d| d.scalar(0.0, &mut rng)),
                        size0: Vec3::from(s) * SCALE * 0.01,
                        rot: em.rotation.as_ref().map_or(0.0, |d| d.scalar(0.0, &mut rng)) * std::f32::consts::TAU,
                        mat,
                    },
                    NotShadowCaster,
                    IgnoreTraces,
                ));
                if let Some(h) = handle.0 {
                    c.insert(MeshMaterial3d(h));
                }
                if let Some(h) = handle.1 {
                    c.insert(MeshMaterial3d(h));
                }
            }
        }
        if e.flash {
            // Flash_C PointLight: Intensity 30000 cd (= 377k lm), 1800 K, visible 0.1 s.
            cmd.spawn((PointLight { intensity: 377_000.0, range: 10.0, color: Color::srgb(1.0, 0.56, 0.17), ..default() }, Transform::from_translation(e.at), Life(0.1)));
        } else {
            // G67_C: 51 traces from (loc + 15 cm up) in an 85 deg cone around up, 5000 cm long; each hit spawns Cubit_ImpactFX_Spawner (FXSize 2).
            let o = e.at + Vec3::Y * 0.15;
            for _ in 0..=50 {
                let (az, el) = (rng.random_range(0.0..std::f32::consts::TAU), (85.0f32.to_radians() * rng.random::<f32>().sqrt()).min(85.0f32.to_radians()));
                let Ok(dir) = Dir3::new(Vec3::new(el.sin() * az.cos(), el.cos(), el.sin() * az.sin())) else { continue };
                let filter = |x: Entity| !ignore.contains(x);
                let hit = ray.cast_ray(Ray3d::new(o, dir), &MeshRayCastSettings::default().with_filter(&filter)).first().map(|(x, h)| (*x, h.point, h.normal.normalize_or_zero(), h.distance));
                if let Some((x, pt, n, d)) = hit && d <= 50.0 {
                    crate::weapon::impact_fx(&mut cmd, &weapon_fx, &mut std, &mut soft, crate::weapon::Surf::of(surf.get(x).ok().flatten().map(|m| m.0)), pt, n);
                }
            }
        }
    }
}

#[derive(Component)]
struct Life(f32);

fn tick_lights(mut cmd: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Life)>) {
    for (e, mut l) in &mut q {
        l.0 -= time.delta_secs();
        if l.0 <= 0.0 {
            cmd.entity(e).despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn simulate(
    mut cmd: Commands,
    time: Res<Time>,
    fx: Res<Fx>,
    cam: Single<&GlobalTransform, With<crate::player::BodycamCamera>>,
    mut std: ResMut<Assets<StandardMaterial>>,
    mut soft: ResMut<Assets<SoftMat>>,
    mut q: Query<(Entity, &mut Transform, &mut Particle)>,
) {
    let dt = time.delta_secs();
    let mut rng = rand::rng();
    let cp = cam.translation();
    for (e, mut tf, mut p) in &mut q {
        p.age += dt;
        let t = p.age / p.life;
        if t >= 1.0 {
            cmd.entity(e).despawn();
            continue;
        }
        let em = &fx.systems[p.sys].emitters[p.em];
        let acc = p.acc;
        p.vel += acc * dt;
        let k = (1.0 - p.drag * dt).max(0.0);
        p.vel *= k;
        tf.translation += p.vel * dt;
        // size: StartSize x SizeMultiplyLife per axis; PSA_Square sprites are square (Size.Y = Size.X).
        let m = em.size_life.as_ref().map_or([1.0; 3], |d| d.eval(t, &mut rng));
        let (w, h) = (p.size0.x * m[0], if em.align == "PSA_Velocity" { p.size0.y * m[1] } else { p.size0.x * m[0] });
        let up = if em.align == "PSA_Velocity" && p.vel.length_squared() > 1e-6 { p.vel.normalize() } else { Vec3::ZERO };
        tf.rotation = if up == Vec3::ZERO {
            cam.rotation() * Quat::from_rotation_z(p.rot)
        } else {
            let right = up.cross(cp - tf.translation).normalize_or_zero();
            Quat::from_mat3(&Mat3::from_cols(right, up, right.cross(up)))
        };
        tf.scale = Vec3::new(w, h, 1.0);
        let c = em.color.as_ref().map_or([1.0; 3], |d| d.eval(t, &mut rng));
        let mut a = em.alpha.as_ref().map_or(1.0, |d| d.scalar(t, &mut rng));
        if em.material == "M_Flare" {
            // soft-particle fade: the flare must not whiteout the screen when the camera is inside it
            a *= ((cp - tf.translation).length() / (w * 0.5).max(0.01)).clamp(0.0, 1.0).powi(2);
        }
        match &p.mat {
            Mat::Std(h) => {
                if let Some(mut mm) = std.get_mut(h) {
                    mm.base_color = Color::LinearRgba(LinearRgba::new(c[0] * HDR, c[1] * HDR, c[2] * HDR, a.min(2.0)));
                }
            }
            Mat::Soft(h) => {
                if let Some(mut mm) = soft.get_mut(h) {
                    // SubUV: SubImageIndex over life picks the flipbook frame (Linear_Blend between frames not done: nearest).
                    let (cols, rows) = (em.sub[0].max(1), em.sub[1].max(1));
                    let f = (em.subuv.as_ref().map_or(0.0, |d| d.scalar(t, &mut rng)).max(0.0) as u32).min(cols * rows - 1);
                    let (cw, ch) = (1.0 / cols as f32, 1.0 / rows as f32);
                    mm.base.uv_transform = bevy::math::Affine2::from_scale_angle_translation(Vec2::new(cw, ch), 0.0, Vec2::new((f % cols) as f32 * cw, (f / cols) as f32 * ch));
                    mm.base.base_color = Color::LinearRgba(LinearRgba::new(c[0] * SMOKE_TONE, c[1] * SMOKE_TONE, c[2] * SMOKE_TONE, a.clamp(0.0, 1.0)));
                }
            }
        }
    }
}

pub struct ExplosionPlugin;
impl Plugin for ExplosionPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<ExplosionFx>().add_systems(Startup, setup).add_systems(Update, (spawn, simulate, tick_lights).chain());
    }
}
