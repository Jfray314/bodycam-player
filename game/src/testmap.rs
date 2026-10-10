//! `BODYCAM_MAP=test`: tiny flat-lit test map for fast iteration (grid floor, distance stripes at 5/10/25 m, target
//! boards, wall ruler, cover, stairs, doorway). Pure primitives, spawns instantly. Player starts at the origin facing -Z.
// ponytail: the player has flat y=0 ground only (player.rs), so stairs/ramp are visual until real collision exists.
use crate::weapon::Health;
use bevy::prelude::*;

pub fn wanted() -> bool {
    std::env::var("BODYCAM_MAP").is_ok_and(|v| v == "test")
}

pub fn spawn(cmd: &mut Commands, meshes: &mut Assets<Mesh>, mats: &mut Assets<StandardMaterial>) {
    let mut mat = |r: f32, g: f32, b: f32| mats.add(StandardMaterial { base_color: Color::srgb(r, g, b), perceptual_roughness: 0.9, ..default() });
    let (floor, line, line5, wall, cover, board) = (mat(0.32, 0.32, 0.33), mat(0.12, 0.12, 0.12), mat(0.03, 0.03, 0.03), mat(0.45, 0.45, 0.43), mat(0.25, 0.3, 0.2), mat(0.6, 0.45, 0.3));
    let (red, yellow, green, white) = (mat(0.85, 0.15, 0.1), mat(0.9, 0.8, 0.1), mat(0.15, 0.7, 0.2), mat(0.95, 0.95, 0.95));
    let (cmd, meshes) = (std::cell::RefCell::new(cmd), std::cell::RefCell::new(meshes));
    let b = |size: Vec3, pos: Vec3, m: &Handle<StandardMaterial>| {
        cmd.borrow_mut().spawn((Mesh3d(meshes.borrow_mut().add(Cuboid::from_size(size))), MeshMaterial3d(m.clone()), Transform::from_translation(pos))).id()
    };
    let (hw, z0, z1) = (15.0, 8.0, -30.0);
    b(Vec3::new(2.0 * hw, 0.2, z0 - z1), Vec3::new(0.0, -0.1, (z0 + z1) / 2.0), &floor);
    // 1 m grid, every 5 m darker and thicker.
    for i in -15..=15 {
        let m5 = i % 5 == 0;
        b(Vec3::new(if m5 { 0.06 } else { 0.02 }, 0.005, z0 - z1), Vec3::new(i as f32, 0.003, (z0 + z1) / 2.0), if m5 { &line5 } else { &line });
    }
    for z in (z1 as i32)..=(z0 as i32) {
        let m5 = z % 5 == 0;
        b(Vec3::new(2.0 * hw, 0.005, if m5 { 0.06 } else { 0.02 }), Vec3::new(0.0, 0.003, z as f32), if m5 { &line5 } else { &line });
    }
    // Distance stripes at 5 / 10 / 25 m straight ahead, plus a target board at each.
    for (d, m, x) in [(5.0, &red, -0.8), (10.0, &yellow, 0.8), (25.0, &green, 0.0)] {
        b(Vec3::new(3.0, 0.006, 0.25), Vec3::new(0.0, 0.006, -d), m);
        b(Vec3::new(0.06, 1.1, 0.06), Vec3::new(x, 0.55, -d), &white);
        let t = b(Vec3::new(0.6, 0.9, 0.04), Vec3::new(x, 1.55, -d), &board);
        cmd.borrow_mut().entity(t).insert(Health(100.0));
        b(Vec3::new(0.2, 0.2, 0.05), Vec3::new(x, 1.55, -d + 0.03), m); // bullseye
    }
    // Back wall at 30 m with a height ruler (0.5 m ticks, 1 m ticks wider) and a wide target board.
    b(Vec3::new(2.0 * hw, 5.0, 0.3), Vec3::new(0.0, 2.5, z1 - 0.15), &wall);
    for i in 0..=8 {
        let w = if i % 2 == 0 { 0.6 } else { 0.3 };
        b(Vec3::new(w, 0.04, 0.02), Vec3::new(-3.0, i as f32 * 0.5, z1 + 0.01), &red);
    }
    b(Vec3::new(1.5, 1.0, 0.04), Vec3::new(0.0, 1.5, z1 + 0.02), &board);
    // Side walls so lateral motion has something to read against.
    for s in [-1.0, 1.0] {
        b(Vec3::new(0.3, 5.0, z0 - z1), Vec3::new(s * (hw + 0.15), 2.5, (z0 + z1) / 2.0), &wall);
    }
    // Cover boxes and a low wall.
    for (x, z) in [(-4.0, -7.0), (3.0, -9.0), (-3.0, -15.0)] {
        b(Vec3::splat(1.0), Vec3::new(x, 0.5, z), &cover);
    }
    b(Vec3::new(3.0, 1.2, 0.3), Vec3::new(4.0, 0.6, -14.0), &cover);
    // Stairs (left) and doorway in a cross wall (right).
    for i in 0..5 {
        b(Vec3::new(2.0, 0.2 * (i + 1) as f32, 0.4), Vec3::new(-9.0, 0.1 * (i + 1) as f32, -12.0 - 0.4 * i as f32), &wall);
    }
    b(Vec3::new(4.0, 2.5, 0.2), Vec3::new(5.0, 1.25, -18.0), &wall);
    b(Vec3::new(6.0, 2.5, 0.2), Vec3::new(12.0, 1.25, -18.0), &wall);
    b(Vec3::new(2.0, 0.5, 0.2), Vec3::new(8.0, 2.25, -18.0), &wall); // lintel over a 2 m x 2 m opening
    let cmd = cmd.into_inner();
    cmd.spawn((
        DirectionalLight { illuminance: 8_000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(4.0, 12.0, 6.0).looking_at(Vec3::new(0.0, 0.0, -10.0), Vec3::Y),
    ));
    cmd.insert_resource(GlobalAmbientLight { brightness: 700.0, ..default() });
}
