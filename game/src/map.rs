//! The real Bodycam lobby / training range (assets-local/maps/scene.glb + scene.json, from `ue-export scene`).
//! Falls back to the grey-box range in main.rs when the asset is missing.
//! Collision: ray casts against the map's render meshes (no physics crate in the project).
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastBackfaces, RayCastVisibility};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstanceReady};
use crate::player::Player;

pub struct MapPlugin;
impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        if available() {
            app.add_systems(Startup, setup).add_systems(Update, (settle, shadow_nearest, apply_exposure));
        }
    }
}

fn root() -> String {
    std::env::var("BODYCAM_ASSETS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../assets-local").into())
}
pub fn available() -> bool {
    let r = root();
    std::path::Path::new(&format!("{r}/maps/scene.glb")).exists() && std::path::Path::new(&format!("{r}/maps/scene.json")).exists()
}

/// Present only while the map is the level. `frames` counts down after the scene spawns so transforms/AABBs exist before the player drops in.
#[derive(Resource)]
pub struct MapState {
    pub frames: i32,
}
#[derive(Component)]
pub struct MapGeometry;

#[derive(serde::Deserialize)]
struct SceneInfo {
    player_start: Start,
    #[serde(default)]
    lights: Vec<UeLight>,
    #[serde(default)]
    post: Vec<UePost>,
}
/// One exported UE light component (units: cm, UE axes; see tools/ue-export/Scene.cs).
#[derive(serde::Deserialize)]
struct UeLight {
    kind: String,
    loc_cm: [f32; 3],
    dir_ue: [f32; 3],
    intensity: f32,
    units: String,
    color: [u8; 3],
    atten_cm: f32,
    src_radius: f32,
    rect_w: f32,
    rect_h: f32,
    outer_deg: f32,
    inner_deg: f32,
    temp: f32,
    shadows: bool,
}
#[derive(serde::Deserialize)]
struct UePost {
    ae_min: f32,
    ae_max: f32,
    level: String,
}
/// Auto-exposure range (EV) from the level's PostProcessVolume, applied to the camera by `apply_exposure`.
#[derive(Resource)]
struct MapExposure(f32, f32);
/// Marks lights that may cast shadows when among the nearest `MAX_SHADOW_LIGHTS` to the camera.
#[derive(Component)]
struct ShadowCandidate;
const MAX_SHADOW_LIGHTS: usize = 6;
/// Offset between UE auto-exposure bounds and Bevy's log-luminance range (tuned by eye).
const EXPOSURE_OFFSET: f32 = 0.0;
/// Stand-in fill for Lumen indirect bounce (cd/m2); the level's skylight is mostly occluded indoors.
const AMBIENT: f32 = 20.0;
#[derive(serde::Deserialize)]
struct Start {
    loc_cm: [f32; 3],
    yaw_deg: f32,
}

/// UE (cm, X fwd, Y right, Z up) -> Bevy (m, -Z fwd, X right, Y up), same as the exporter.
fn ue_to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[1], p[2], -p[0]) * 0.01
}

fn setup(mut cmd: Commands, assets: Res<AssetServer>, mut ambient: ResMut<GlobalAmbientLight>) {
    let info: SceneInfo = serde_json::from_str(&std::fs::read_to_string(format!("{}/maps/scene.json", root())).expect("scene.json")).expect("scene.json parse");
    let start = ue_to_bevy(info.player_start.loc_cm);
    let yaw = -info.player_start.yaw_deg.to_radians();
    cmd.insert_resource(MapState { frames: i32::MAX });
    cmd.spawn((WorldAssetRoot(assets.load("maps/scene.glb#Scene0")), Transform::default()))
        .observe(move |ev: On<WorldInstanceReady>, mut cmd: Commands, children: Query<&Children>, meshes: Query<(), With<Mesh3d>>, mut player: Single<(&mut Player, &mut Transform)>, mut st: ResMut<MapState>| {
            for e in children.iter_descendants(ev.entity) {
                if meshes.contains(e) {
                    cmd.entity(e).insert((MapGeometry, RayCastBackfaces));
                }
            }
            // PlayerStart is the capsule centre (88 cm half height): drop from just above and let the ground cast find the floor.
            player.1.translation = start;
            player.0.yaw = yaw;
            player.0.vel = Vec3::ZERO;
            st.frames = 5;
        });
    if info.lights.is_empty() {
        // Fallback when scene.json has no lights: dim shadowed sun + strong ambient fill.
        cmd.spawn((DirectionalLight { illuminance: 10_000.0, shadow_maps_enabled: true, ..default() }, Transform::from_xyz(30.0, 60.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y)));
        ambient.brightness = 1200.0;
    } else {
        ambient.brightness = AMBIENT;
        for l in &info.lights {
            spawn_light(&mut cmd, l);
        }
        if let Some(p) = info.post.iter().find(|p| p.level.ends_with("GlobalPostProcess")) {
            cmd.insert_resource(MapExposure(p.ae_min + EXPOSURE_OFFSET, p.ae_max + EXPOSURE_OFFSET));
        }
    }
}

/// UE intensity -> candela (point/spot/rect) or lux (directional). Unitless: 1/625 cd per unit (UE: default 5000 = 8 cd);
/// Lumens: divided by the emission solid angle (sphere 4pi, spot cone, rect Lambert pi).
fn candela(l: &UeLight) -> f32 {
    use std::f32::consts::PI;
    let outer = l.outer_deg.to_radians().clamp(0.01, 1.55);
    if l.units.contains("Candelas") {
        l.intensity
    } else if l.units.contains("Lumens") {
        l.intensity / match l.kind.as_str() { "spot" => 2.0 * PI * (1.0 - outer.cos()), "rect" => PI, _ => 4.0 * PI }
    } else {
        l.intensity / 625.0
    }
}

/// Kelvin -> RGB tint (Tanner Helland approximation).
fn kelvin(t: f32) -> [f32; 3] {
    let t = t.clamp(1000.0, 15000.0) / 100.0;
    let r = if t <= 66.0 { 255.0 } else { 329.7 * (t - 60.0).powf(-0.1332) };
    let g = if t <= 66.0 { 99.47 * t.ln() - 161.12 } else { 288.1 * (t - 60.0).powf(-0.0755) };
    let b = if t >= 66.0 { 255.0 } else if t <= 19.0 { 0.0 } else { 138.5 * (t - 10.0).ln() - 305.04 };
    [r, g, b].map(|v| (v / 255.0).clamp(0.0, 1.0))
}

fn spawn_light(cmd: &mut Commands, l: &UeLight) {
    use std::f32::consts::PI;
    let pos = ue_to_bevy(l.loc_cm);
    let fwd = ue_to_bevy(l.dir_ue).normalize_or(Vec3::NEG_Z);
    let up = if fwd.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let tf = Transform::from_translation(pos).looking_to(fwd, up);
    let mut c = l.color.map(|v| (v as f32 / 255.0).powf(2.2));
    if l.temp > 0.0 {
        let k = kelvin(l.temp);
        c = [c[0] * k[0].powf(2.2), c[1] * k[1].powf(2.2), c[2] * k[2].powf(2.2)];
    }
    let color = Color::linear_rgb(c[0], c[1], c[2]);
    let cd = candela(l);
    let range = (l.atten_cm * 0.01).max(0.5);
    let radius = l.src_radius * 0.01;
    // Bevy point/spot intensity is lumens = 4pi * candela.
    let lm = cd * 4.0 * PI;
    let e = match l.kind.as_str() {
        "point" if cd > 0.0 => cmd.spawn((PointLight { color, intensity: lm, range, radius, shadow_maps_enabled: false, ..default() }, tf)).id(),
        "spot" if cd > 0.0 => {
            let outer = l.outer_deg.to_radians().clamp(0.02, 1.5);
            cmd.spawn((SpotLight { color, intensity: lm, range, radius, outer_angle: outer, inner_angle: l.inner_deg.to_radians().clamp(0.0, outer * 0.95), shadow_maps_enabled: false, ..default() }, tf)).id()
        }
        "rect" if cd > 0.0 => {
            let (w, h) = (l.rect_w * 0.01, l.rect_h * 0.01);
            cmd.spawn((RectLight { color, intensity: cd * PI * (w * h).max(0.01), range, width: w, height: h }, tf)).id()
        }
        "directional" if l.intensity > 0.0 => {
            cmd.spawn((DirectionalLight { color, illuminance: l.intensity, shadow_maps_enabled: l.shadows, ..default() }, tf));
            return;
        }
        _ => return,
    };
    if l.shadows && matches!(l.kind.as_str(), "point" | "spot") {
        cmd.entity(e).insert(ShadowCandidate);
    }
}

/// Shadow maps only on the nearest few shadow-casting local lights (the shadow maps are the cost, not the lights).
fn shadow_nearest(
    cam: Query<&GlobalTransform, With<Camera3d>>,
    mut points: Query<(Entity, &GlobalTransform, &mut PointLight), With<ShadowCandidate>>,
    mut spots: Query<(Entity, &GlobalTransform, &mut SpotLight), With<ShadowCandidate>>,
) {
    let Some(p) = cam.iter().next().map(|t| t.translation()) else { return };
    let mut d: Vec<(f32, Entity)> = points.iter().map(|(e, t, _)| (t.translation().distance_squared(p), e)).chain(spots.iter().map(|(e, t, _)| (t.translation().distance_squared(p), e))).collect();
    d.sort_by(|a, b| a.0.total_cmp(&b.0));
    let on: Vec<Entity> = d.iter().take(MAX_SHADOW_LIGHTS).map(|x| x.1).collect();
    for (e, _, mut l) in &mut points {
        let v = on.contains(&e);
        if l.shadow_maps_enabled != v {
            l.shadow_maps_enabled = v;
        }
    }
    for (e, _, mut l) in &mut spots {
        let v = on.contains(&e);
        if l.shadow_maps_enabled != v {
            l.shadow_maps_enabled = v;
        }
    }
}

fn apply_exposure(ex: Option<Res<MapExposure>>, mut q: Query<&mut bevy::post_process::auto_exposure::AutoExposure>) {
    let Some(ex) = ex else { return };
    for mut ae in &mut q {
        if *ae.range.start() != ex.0 || *ae.range.end() != ex.1 {
            ae.range = ex.0..=ex.1;
        }
    }
}

fn settle(mut st: ResMut<MapState>) {
    if st.frames > 0 && st.frames != i32::MAX {
        st.frames -= 1;
    }
}
impl MapState {
    pub fn ready(&self) -> bool {
        self.frames == 0
    }
}

const RADIUS: f32 = 0.34;
const HEIGHT: f32 = 1.8;
const STEP: f32 = 0.4;

/// Moves `pos` (feet) from `old` against the map: walls (3 probes + 4 compass push-outs), floor/step snap, ceiling.
/// Returns whether the player stands on ground. ponytail: ray probes, not a swept capsule; thin ledges and sharp slopes can leak.
pub fn collide(ray: &mut MeshRayCast, geo: &Query<(), With<MapGeometry>>, old: Vec3, pos: &mut Vec3, vel: &mut Vec3, was_grounded: bool) -> bool {
    let filter = |e: Entity| geo.contains(e);
    let settings = MeshRayCastSettings::default().with_filter(&filter).with_visibility(RayCastVisibility::Any);
    let mut cast = |from: Vec3, dir: Vec3, max: f32| -> Option<(f32, Vec3)> {
        let d = Dir3::new(dir).ok()?;
        ray.cast_ray(Ray3d::new(from, d), &settings).first().map(|(_, h)| (h.distance, h.normal)).filter(|(dist, _)| *dist <= max)
    };
    let delta = Vec3::new(pos.x - old.x, 0.0, pos.z - old.z);
    let len = delta.length();
    if len > 1e-5 {
        let dir = delta / len;
        for h in [0.5, 1.0, 1.5] {
            if let Some((d, n)) = cast(old + Vec3::Y * h, dir, len + RADIUS) {
                let mut n = Vec3::new(n.x, 0.0, n.z).normalize_or_zero();
                if n.dot(dir) > 0.0 {
                    n = -n;
                }
                let pen = len + RADIUS - d;
                if n != Vec3::ZERO && pen > 0.0 {
                    pos.x += n.x * pen;
                    pos.z += n.z * pen;
                    let into = vel.dot(n).min(0.0);
                    vel.x -= n.x * into;
                    vel.z -= n.z * into;
                }
            }
        }
    }
    for h in [0.5, 1.2] {
        for dir in [Vec3::X, Vec3::NEG_X, Vec3::Z, Vec3::NEG_Z] {
            if let Some((d, _)) = cast(*pos + Vec3::Y * h, dir, RADIUS) {
                *pos -= dir * (RADIUS - d);
            }
        }
    }
    if vel.y > 0.0 {
        if let Some((d, _)) = cast(*pos + Vec3::Y * 0.1, Vec3::Y, HEIGHT - 0.1) {
            pos.y = pos.y + 0.1 + d - HEIGHT;
            vel.y = 0.0;
        }
    }
    let top = old.y.max(pos.y) + STEP;
    if let Some((d, _)) = cast(Vec3::new(pos.x, top, pos.z), Vec3::NEG_Y, 200.0) {
        let ground = top - d;
        if vel.y <= 0.0 && pos.y <= ground + if was_grounded { STEP } else { 0.02 } {
            pos.y = ground;
            return true;
        }
    }
    false
}
