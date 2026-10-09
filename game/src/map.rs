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
            app.add_systems(Startup, setup).add_systems(Update, settle);
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
    // The map's lights are not exported: a dim sun (shadowed, so interiors stay shaded) plus a strong ambient fill.
    cmd.spawn((
        DirectionalLight { illuminance: 1_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(30.0, 60.0, 20.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
    ambient.brightness = 100.0;
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
