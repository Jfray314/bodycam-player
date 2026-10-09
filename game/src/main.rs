mod hud;
mod player;
mod postfx;
mod weapon;

use bevy::prelude::*;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins)
        .add_plugins((player::PlayerPlugin, weapon::WeaponPlugin, hud::HudPlugin, postfx::PostFxPlugin))
        .insert_resource(ClearColor(Color::srgb(0.55, 0.65, 0.8)))
        .add_systems(Startup, test_range)
        .run();
}

/// Minimal firing range so the player has something to stand on and shoot. Not a port of any map.
fn test_range(mut cmd: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    cmd.spawn((Mesh3d(meshes.add(Plane3d::default().mesh().size(100.0, 100.0))), MeshMaterial3d(mats.add(Color::srgb(0.3, 0.3, 0.3)))));
    let cube = meshes.add(Cuboid::new(1.0, 2.0, 1.0));
    let mat = mats.add(Color::srgb(0.6, 0.2, 0.2));
    for i in 0..6 {
        cmd.spawn((
            Mesh3d(cube.clone()),
            MeshMaterial3d(mat.clone()),
            Transform::from_xyz(i as f32 * 2.5 - 6.0, 1.0, -15.0),
            weapon::Health(100.0),
        ));
    }
    cmd.spawn((DirectionalLight { shadow_maps_enabled: true, ..default() }, Transform::from_xyz(4.0, 10.0, 4.0).looking_at(Vec3::ZERO, Vec3::Y)));
}
