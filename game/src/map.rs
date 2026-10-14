//! The real Bodycam lobby / training range (assets-local/maps/scene.glb + scene.json, from `ue-export scene`).
//! Falls back to the grey-box range in main.rs when the asset is missing.
//! Collision: ray casts against the map's render meshes (no physics crate in the project).
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings, RayCastBackfaces, RayCastVisibility};
use bevy::asset::RenderAssetUsages;
use bevy::camera::Exposure;
use bevy::light::Skybox;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension};
use bevy::curve::cubic_splines::LinearSpline;
use bevy::post_process::auto_exposure::{AutoExposure, AutoExposureCompensationCurve};
use bevy::gltf::GltfMaterialName;
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
    if crate::testmap::wanted() || crate::grid::wanted() {
        return false;
    }
    let r = root();
    let s = scene_name();
    std::path::Path::new(&format!("{r}/maps/{s}.glb")).exists() && std::path::Path::new(&format!("{r}/maps/{s}.json")).exists()
}
/// Scene file base name in assets-local/maps (BODYCAM_SCENE overrides, e.g. the re-export `scene2`).
fn scene_name() -> String {
    std::env::var("BODYCAM_SCENE").unwrap_or_else(|_| "scene2".into())
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
    /// glTF material name -> EPhysicalSurface index (PhysMat), absent = default.
    #[serde(default)]
    surfaces: std::collections::HashMap<String, u8>,
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
/// Marks that the level supplies UE auto-exposure; `apply_exposure` configures the camera's AutoExposure from it.
#[derive(Resource)]
struct MapExposure(Handle<AutoExposureCompensationCurve>, Handle<Image>);
/// Marks lights that may cast shadows when among the nearest `MAX_SHADOW_LIGHTS` to the camera.
#[derive(Component)]
struct ShadowCandidate;
const MAX_SHADOW_LIGHTS: usize = 6;
/// UE auto exposure resolved over the three always-loaded unbound PostProcessVolumes (a higher priority only
/// overrides properties whose bOverride_ flag is set): Lighting_test_2 (prio 20) min -7 / max -2 EV100, LowPercent 40;
/// GlobalPostProcess (prio 10) speeds 13/13 (its -5.5/-1.5 are not overridden); LobbyB (prio 0) min -7. Histogram method.
const UE_AE_MIN: f32 = -7.0;
const UE_AE_MAX: f32 = -2.0;
const UE_AE_SPEED: f32 = 13.0;
/// Camera pre-exposure EV100 (Bevy scales the scene by 2^-ev100 / 1.2, the same formula UE uses); the AE ranges are shifted by it.
const CAM_EV100: f32 = 4.0;
/// Bevy log2 pre-exposed luminance = UE EV100 - this (UE EV100 = log2(L * 100 / 12.5) = log2(8 L)).
const UE_TO_BEVY: f32 = 3.25 + CAM_EV100; // 3 + log2(1.2) = 3.263, rounded to 1/16 so the curve segments join exactly in f32
/// Bevy's AE maps the average to 1.0; UE's maps it to middle grey.
const MIDDLE_GREY: f32 = 0.18;
/// Histogram span in UE EV100 (Bevy ignores pixels below and clamps those above).
const UE_HIST: (f32, f32) = (-16.0, 4.0);
/// Stand-in fill for Lumen indirect bounce (cd/m2); the level's skylight is mostly occluded indoors.
const AMBIENT: f32 = 0.2;
/// Sky cubemap scale (cd/m2). Ultra_Dynamic_Sky (LobbyB) draws its sky at runtime (SkyAtmosphere + clouds, nothing cooked): a gradient + sun glow stands in.
const SKY_NITS: f32 = 300.0;

/// Gradient sky cube (zenith blue -> pale horizon -> grey ground) with a sun glow towards `sun` (unit vector to the sun, Bevy axes).
fn sky_cube(sun: Vec3) -> Image {
    const N: usize = 32;
    let mut data = Vec::with_capacity(N * N * 6 * 4);
    for f in 0..6 {
        for y in 0..N {
            for x in 0..N {
                let (u, v) = (2.0 * (x as f32 + 0.5) / N as f32 - 1.0, 2.0 * (y as f32 + 0.5) / N as f32 - 1.0);
                let d = match f {
                    0 => Vec3::new(1.0, -v, -u),
                    1 => Vec3::new(-1.0, -v, u),
                    2 => Vec3::new(u, 1.0, v),
                    3 => Vec3::new(u, -1.0, -v),
                    4 => Vec3::new(u, -v, 1.0),
                    _ => Vec3::new(-u, -v, -1.0),
                }
                .normalize();
                let t = d.y.max(0.0).sqrt();
                let sky = if d.y < 0.0 { Vec3::splat(0.35) } else { Vec3::new(0.78, 0.86, 1.0).lerp(Vec3::new(0.28, 0.5, 0.95), t) };
                let g = d.dot(sun).max(0.0);
                let c = sky + Vec3::new(1.0, 0.9, 0.7) * (g.powf(8.0) * 0.25 + g.powf(400.0) * 4.0);
                data.extend(c.to_array().map(|v| (v.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8));
                data.push(255);
            }
        }
    }
    let mut img = Image::new(Extent3d { width: N as u32, height: (N * 6) as u32, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    img.reinterpret_stacked_2d_as_array(6);
    img.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    img
}
#[derive(serde::Deserialize)]
struct Start {
    loc_cm: [f32; 3],
    yaw_deg: f32,
}

/// UE (cm, X fwd, Y right, Z up) -> Bevy (m, -Z fwd, X right, Y up), same as the exporter.
fn ue_to_bevy(p: [f32; 3]) -> Vec3 {
    Vec3::new(p[1], p[2], -p[0]) * 0.01
}

fn setup(mut cmd: Commands, assets: Res<AssetServer>, mut ambient: ResMut<GlobalAmbientLight>, mut curves: ResMut<Assets<AutoExposureCompensationCurve>>, mut images: ResMut<Assets<Image>>) {
    let sn = scene_name();
    let info: SceneInfo = serde_json::from_str(&std::fs::read_to_string(format!("{}/maps/{sn}.json", root())).expect("scene json")).expect("scene json parse");
    let surfaces = info.surfaces.clone();
    let start = ue_to_bevy(info.player_start.loc_cm);
    let yaw = -info.player_start.yaw_deg.to_radians();
    cmd.insert_resource(MapState { frames: i32::MAX });
    // Ultra_Dynamic_Sky's sun (the directional with its intensity set): the light travels along dir_ue, the sun lies opposite.
    let sun_dir = info.lights.iter().find(|l| l.kind == "directional" && l.intensity > 0.0).map_or(Vec3::new(-0.4, 0.8, 0.45), |l| -ue_to_bevy(l.dir_ue).normalize());
    cmd.spawn((WorldAssetRoot(assets.load(format!("maps/{sn}.glb#Scene0"))), Transform::default()))
        .observe(move |ev: On<WorldInstanceReady>, mut cmd: Commands, children: Query<&Children>, meshes: Query<Option<&GltfMaterialName>, With<Mesh3d>>, mut player: Single<(&mut Player, &mut Transform)>, mut st: ResMut<MapState>| {
            for e in children.iter_descendants(ev.entity) {
                if let Ok(name) = meshes.get(e) {
                    cmd.entity(e).insert((MapGeometry, RayCastBackfaces));
                    // PhysMat SurfaceType of the material (scene.json "surfaces"): footsteps + bullet impacts read it.
                    if let Some(s) = name.and_then(|n| surfaces.get(&n.0)) {
                        cmd.entity(e).insert(crate::weapon::SurfaceMat(*s));
                    }
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
        // UE clamps the metered EV100 to [min, max]; Bevy has no clamp, so fold it into the compensation curve:
        // target = -avg + c becomes -clamp(avg, lo, hi) + c, i.e. y(avg) = avg - clamp(avg) + c (the histogram itself spans all of UE_HIST).
        let (x0, x1, lo, hi, c) = (UE_HIST.0 - UE_TO_BEVY, UE_HIST.1 - UE_TO_BEVY, UE_AE_MIN - UE_TO_BEVY, UE_AE_MAX - UE_TO_BEVY, (MIDDLE_GREY.log2() * 16.0).round() / 16.0);
        let curve = AutoExposureCompensationCurve::from_curve(LinearSpline::new([vec2(x0, x0 - lo + c), vec2(lo, c), vec2(hi, c), vec2(x1, x1 - hi + c)])).expect("exposure curve");
        cmd.insert_resource(MapExposure(curves.add(curve), images.add(sky_cube(sun_dir))));
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

fn apply_exposure(mut cmd: Commands, ex: Option<Res<MapExposure>>, mut q: Query<(Entity, &mut AutoExposure), Added<AutoExposure>>) {
    let Some(ex) = ex else { return };
    for (e, mut ae) in &mut q {
        ae.range = UE_HIST.0 - UE_TO_BEVY..=UE_HIST.1 - UE_TO_BEVY;
        ae.filter = 0.4..=0.9;
        // Bevy's shader swaps the two speeds (brightening uses speed_darken); equal here anyway.
        ae.speed_brighten = UE_AE_SPEED;
        ae.speed_darken = UE_AE_SPEED;
        ae.compensation_curve = ex.0.clone();
        cmd.entity(e).insert((Exposure { ev100: CAM_EV100 }, Skybox { image: Some(ex.1.clone()), brightness: SKY_NITS, rotation: Quat::IDENTITY }));
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
