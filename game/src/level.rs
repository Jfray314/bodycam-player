//! In-process level switching: the tablet's PLAY list (and the start of the game) asks for a level; `load` tears the old
//! one down and spawns the new one behind the loading screen, without restarting the game.
//! A level = (BODYCAM_MAP "" | grid | test, BODYCAM_START zone, BODYCAM_MODE "" | bomb), seeded from the env at launch.
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use std::collections::HashSet;
use std::sync::RwLock;

static LEVEL: RwLock<Option<[String; 3]>> = RwLock::new(None);

/// The current level: [map, zone, mode].
pub fn level() -> [String; 3] {
    LEVEL.read().unwrap().clone().unwrap_or_else(|| ["BODYCAM_MAP", "BODYCAM_START", "BODYCAM_MODE"].map(|k| std::env::var(k).unwrap_or_default()))
}

/// Everything a level spawned (top-level entities; children go with them). Despawned on a switch.
#[derive(Component)]
pub struct MapRoot;

/// Load this level next frame (Some) or the current one (None, at startup).
#[derive(Resource)]
pub struct LoadLevel(pub Option<[String; 3]>);

pub struct LevelPlugin;
impl Plugin for LevelPlugin {
    fn build(&self, app: &mut App) {
        // First: never in the middle of a frame's gameplay systems (their run conditions already passed)
        app.insert_resource(LoadLevel(None)).add_systems(First, (dev_switch, load.run_if(resource_exists::<LoadLevel>)).chain());
    }
}

/// Dev: BODYCAM_SWITCH="t:map:zone:mode" loads that level t s (sim) after viewmodel ready (switch tests).
fn dev_switch(mut cmd: Commands, time: Res<Time>, ready: Res<crate::dev::Ready>, mut done: Local<bool>) {
    let (Some(r), false) = (ready.0, *done) else { return };
    let Some(v) = std::env::var("BODYCAM_SWITCH").ok() else { return };
    let p: Vec<&str> = v.split(':').collect();
    if p.len() == 4 && p[0].parse::<f32>().is_ok_and(|t| time.elapsed_secs() - r >= t) {
        *done = true;
        cmd.insert_resource(LoadLevel(Some([p[1], p[2], p[3]].map(String::from))));
    }
}

fn load(world: &mut World) {
    let Some(LoadLevel(next)) = world.remove_resource::<LoadLevel>() else { return };
    let first = next.is_none();
    if let Some(l) = next {
        *LEVEL.write().unwrap() = Some(l);
        unload(world);
    }
    // Bevy 0.20 resources are entities too (IsResource): never tag those, unload removes the level's resources by type
    let before: HashSet<Entity> = world.query::<Entity>().iter(world).collect();
    let res = if crate::grid::wanted() || crate::testmap::wanted() || !crate::map::available() {
        world.run_system_once(crate::test_range)
    } else {
        world.run_system_once(crate::map::setup)
    };
    if let Err(e) = res {
        error!("level load: {e}");
    }
    if crate::bomb::wanted() {
        let _ = world.run_system_once(crate::bomb::spawn);
    }
    world.flush();
    let new: Vec<Entity> = world.query_filtered::<Entity, (Without<ChildOf>, Without<bevy::ecs::resource::IsResource>)>().iter(world).filter(|e| !before.contains(e)).collect();
    for e in new {
        world.entity_mut(e).insert(MapRoot);
    }
    if !first {
        let _ = world.run_system_once(after_switch);
    }
    info!("level {:?}", level());
}

/// Drops the old level: its entities, its resources, the bomb-mode bomb, and the lobby's exposure/sky on the camera.
fn unload(world: &mut World) {
    let roots: Vec<Entity> = world.query_filtered::<Entity, With<MapRoot>>().iter(world).collect();
    for e in roots {
        if let Ok(e) = world.get_entity_mut(e) {
            e.despawn();
        }
    }
    crate::grid::unload(world);
    crate::bots::unload(world);
    crate::bomb::unload(world);
    crate::map::unload(world);
    world.insert_resource(GlobalAmbientLight::default());
}

/// New level spawned: player back to the origin (the lobby observer / grid zone then place it), loading screen up,
/// fresh lens stack on the camera (so the lobby's AutoExposure curve is (re)applied or dropped).
fn after_switch(
    mut cmd: Commands,
    mut p: Single<(&mut crate::player::Player, &mut Transform)>,
    st: Res<crate::postfx::PostFxState>,
    cams: Query<Entity, With<crate::player::BodycamCamera>>,
    mut tr: MessageWriter<crate::transition::Reload>,
) {
    let (pl, tf) = &mut *p;
    (tf.translation, pl.yaw, pl.pitch, pl.vel) = (Vec3::ZERO, 0.0, 0.0, Vec3::ZERO);
    for e in &cams {
        cmd.entity(e).remove::<(bevy::light::Skybox, bevy::camera::Exposure)>();
    }
    crate::postfx::apply_enabled(&mut cmd, &st, cams.iter());
    tr.write(crate::transition::Reload);
}
