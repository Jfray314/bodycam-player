mod audio;
mod body;
mod bodyik;
mod camadjust;
mod curve;
mod effects;
mod soft;
mod dev;
mod bots;
mod grid;
mod gunfight;
mod testmap;
mod ui;
mod hud;
mod items;
mod perks;
mod map;
mod player;
mod postfx;
mod recoil;
mod status;
mod viewmodel;
mod sway;
mod weapon;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(AssetPlugin {
            // Converted game assets (fonts, audio, models) live outside git; see REWRITE.md.
            // BODYCAM_ASSETS overrides the root (for a checkout that has no assets-local next to it).
            file_path: std::env::var("BODYCAM_ASSETS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../assets-local").into()),
            ..default()
        }))
        .add_plugins(body::BodyPlugin)
        .add_plugins((player::PlayerPlugin, weapon::WeaponPlugin, hud::HudPlugin, postfx::PostFxPlugin, status::StatusPlugin, audio::AudioPlugin, viewmodel::ViewmodelPlugin, sway::SwayPlugin, recoil::RecoilPlugin, map::MapPlugin, gunfight::GunfightPlugin, grid::GridPlugin, bots::BotsPlugin, camadjust::CamAdjustPlugin, dev::DevPlugin))
        .add_plugins((effects::EffectsPlugin, soft::SoftPlugin, items::ItemsPlugin, perks::PerksPlugin))
        .add_plugins(ui::UiPlugin)
        .insert_resource(ClearColor(Color::srgb(0.55, 0.65, 0.8)))
        .add_systems(Startup, test_range.run_if(|| !map::available()))
        .run();
}

/// Indoor firing-range test room (walls, slatted skylight, crates, target stands) for judging the
/// camera look against in-game screenshots. Not a port of any map.
fn test_range(mut cmd: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>, mut images: ResMut<Assets<Image>>) {
    if grid::wanted() {
        return grid::spawn(&mut cmd, &mut meshes, &mut mats, &mut images);
    }
    if testmap::wanted() {
        return testmap::spawn(&mut cmd, &mut meshes, &mut mats);
    }
    let concrete = mats.add(StandardMaterial { base_color: Color::srgb(0.55, 0.53, 0.5), perceptual_roughness: 0.9, ..default() });
    let floor = mats.add(StandardMaterial { base_color: Color::srgb(0.6, 0.57, 0.52), perceptual_roughness: 0.85, ..default() });
    let crate_mat = mats.add(StandardMaterial { base_color: Color::srgb(0.35, 0.38, 0.28), perceptual_roughness: 0.7, ..default() });
    let card = mats.add(StandardMaterial { base_color: Color::srgb(0.75, 0.62, 0.45), perceptual_roughness: 0.95, ..default() });
    let white = mats.add(StandardMaterial { base_color: Color::srgb(0.9, 0.9, 0.88), perceptual_roughness: 0.6, ..default() });
    let mut boxy = |cmd: &mut Commands, size: Vec3, pos: Vec3, mat: &Handle<StandardMaterial>| {
        cmd.spawn((Mesh3d(meshes.add(Cuboid::from_size(size))), MeshMaterial3d(mat.clone()), Transform::from_translation(pos))).id()
    };
    let (w, h, z0, z1) = (5.0, 4.5, -16.0, 4.0);
    let (len, mid) = (z1 - z0, (z0 + z1) / 2.0);
    boxy(&mut cmd, Vec3::new(2.0 * w, 0.2, len), Vec3::new(0.0, -0.1, mid), &floor);
    boxy(&mut cmd, Vec3::new(0.3, h, len), Vec3::new(-w, h / 2.0, mid), &concrete);
    boxy(&mut cmd, Vec3::new(0.3, h, len), Vec3::new(w, h / 2.0, mid), &concrete);
    boxy(&mut cmd, Vec3::new(2.0 * w, h, 0.3), Vec3::new(0.0, h / 2.0, z0), &concrete);
    boxy(&mut cmd, Vec3::new(2.0 * w, h, 0.3), Vec3::new(0.0, h / 2.0, z1), &concrete);
    // Ceiling with a slatted skylight so the sun throws stripes across floor and wall.
    for i in 0..((len / 0.5) as i32) {
        let z = z0 + 0.25 + i as f32 * 0.5;
        let skylight = (-9.0..-3.0).contains(&z);
        let width = if skylight { 0.12 } else { 0.5 };
        boxy(&mut cmd, Vec3::new(2.0 * w, 0.2, width), Vec3::new(0.0, h, z), &concrete);
    }
    for (x, z) in [(-3.6, -12.0), (-3.6, -10.8), (-2.4, -12.0), (3.8, -6.0)] {
        boxy(&mut cmd, Vec3::splat(1.0), Vec3::new(x, 0.5, z), &crate_mat);
    }
    boxy(&mut cmd, Vec3::new(1.0, 0.6, 0.8), Vec3::new(3.0, 0.3, -2.5), &card);
    for (i, x) in [-1.0f32, 1.0, 3.0].iter().enumerate() {
        let z = -11.0 + i as f32 * 0.3;
        boxy(&mut cmd, Vec3::new(0.06, 1.1, 0.06), Vec3::new(*x, 0.55, z), &white);
        boxy(&mut cmd, Vec3::new(0.8, 0.06, 0.6), Vec3::new(*x, 0.03, z), &white);
        let board = boxy(&mut cmd, Vec3::new(0.6, 0.9, 0.04), Vec3::new(*x, 1.55, z + 0.05), &card);
        cmd.entity(board).insert(weapon::Health(100.0));
    }
    cmd.spawn((
        DirectionalLight { illuminance: 25_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(2.0, 10.0, -2.0).looking_at(Vec3::new(-1.5, 0.0, -7.0), Vec3::Y),
    ));
    cmd.insert_resource(GlobalAmbientLight { brightness: 1200.0, ..default() });
}
