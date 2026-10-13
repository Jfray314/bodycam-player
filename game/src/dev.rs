//! Dev/test hooks driven by env vars. All times are seconds of sim (virtual) time measured from "viewmodel ready"
//! (arms + gun meshes spawned), so scripts don't need guessed load-time offsets. See REWRITE.md "Fast iteration".
//!   BODYCAM_AUTO="R@6:0.5,LMB@1:2"  hold keys/buttons (default hold 0.1 s)
//!   BODYCAM_SCREENSHOT=path[.png]   one shot at BODYCAM_SCREENSHOT_AT (default 0.5), then exit
//!   BODYCAM_SHOTS="0.5,1,2"         numbered shots <prefix>_1.png.. (prefix = BODYCAM_SCREENSHOT minus .png, default shots/shot), then exit
//!   BODYCAM_RECORD=dir:fps:secs     frame sequence dir/f_0001.png.. at a fixed sim step of 1/fps, then exit
//!   BODYCAM_SPEED=N                 virtual time speed multiplier
use crate::viewmodel::{ArmsRoot, GunRoot};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::time::TimeUpdateStrategy;
use std::time::Duration;

/// Virtual elapsed seconds when the viewmodel became ready.
#[derive(Resource, Default)]
pub struct Ready(pub Option<f32>);

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|s| !s.is_empty())
}

pub struct DevPlugin;
impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Ready>().add_systems(PreUpdate, (mark_ready, autotest.after(bevy::input::InputSystems)).chain()).add_systems(Update, shots);
        if let Some(s) = env("BODYCAM_SPEED").and_then(|v| v.parse::<f32>().ok()) {
            app.add_systems(Startup, move |mut t: ResMut<Time<Virtual>>| {
                t.set_relative_speed(s);
                t.set_max_delta(Duration::from_millis(500));
            });
        }
        if let Some((_, fps, _)) = record_cfg() {
            app.insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(1.0 / fps)));
        }
    }
}

fn record_cfg() -> Option<(String, f32, f32)> {
    let v = env("BODYCAM_RECORD")?;
    let mut p = v.rsplitn(3, ':'); // dir may hold a drive-letter colon
    let (dur, fps, dir) = (p.next()?.parse().ok()?, p.next()?.parse().ok()?, p.next()?.to_string());
    Some((dir, fps, dur))
}

fn has_mesh_under<R: Component>(roots: &Query<(), With<R>>, meshes: &Query<Entity, With<Mesh3d>>, parents: &Query<&ChildOf>) -> bool {
    meshes.iter().any(|mut e| loop {
        if roots.contains(e) {
            break true;
        }
        match parents.get(e) {
            Ok(p) => e = p.parent(),
            Err(_) => break false,
        }
    })
}

fn mark_ready(
    time: Res<Time>,
    real: Res<Time<Real>>,
    mut ready: ResMut<Ready>,
    arms: Query<(), With<ArmsRoot>>,
    guns: Query<(), With<GunRoot>>,
    meshes: Query<Entity, With<Mesh3d>>,
    parents: Query<&ChildOf>,
    mut settle: Local<u32>,
) {
    if ready.0.is_some() {
        return;
    }
    // Real-time fallback so a missing model can't hang a test run forever.
    if (has_mesh_under(&arms, &meshes, &parents) && has_mesh_under(&guns, &meshes, &parents)) || real.elapsed_secs() > 90.0 {
        *settle += 1; // a few frames so animation poses are applied before t=0
        if *settle > 5 {
            ready.0 = Some(time.elapsed_secs());
            info!("viewmodel ready at {:.2}s real", real.elapsed_secs());
        }
    }
}

type Plan = Vec<(Result<KeyCode, MouseButton>, f32, f32)>;

fn autotest(
    time: Res<Time>,
    ready: Res<Ready>,
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut fake: ResMut<crate::sway::FakeLook>,
    mut plan: Local<Option<Plan>>,
) {
    let Some(r) = ready.0 else { return };
    let plan = plan.get_or_insert_with(|| {
        env("BODYCAM_AUTO").unwrap_or_default().split(',').filter_map(|s| {
            let (k, t) = s.split_once('@')?;
            let (t, hold) = t.split_once(':').unwrap_or((t, "0.1"));
            let key = match k {
                "LMB" => Err(MouseButton::Left),
                "RMB" => Err(MouseButton::Right),
                "Ctrl" => Ok(KeyCode::ControlLeft),
                "Shift" => Ok(KeyCode::ShiftLeft),
                "1" => Ok(KeyCode::Digit1),
                "2" => Ok(KeyCode::Digit2),
                "3" => Ok(KeyCode::Digit3),
                "4" => Ok(KeyCode::Digit4),
                "5" => Ok(KeyCode::Digit5),
                "R" => Ok(KeyCode::KeyR),
                "J" => Ok(KeyCode::KeyJ),
                "W" => Ok(KeyCode::KeyW),
                "A" => Ok(KeyCode::KeyA),
                "S" => Ok(KeyCode::KeyS),
                "D" => Ok(KeyCode::KeyD),
                "Q" => Ok(KeyCode::KeyQ),
                "E" => Ok(KeyCode::KeyE),
                "Space" => Ok(KeyCode::Space),
                "F1" => Ok(KeyCode::F1),
                "F3" => Ok(KeyCode::F3),
                "F4" => Ok(KeyCode::F4),
                "F5" => Ok(KeyCode::F5),
                "F7" => Ok(KeyCode::F7),
                "F10" => Ok(KeyCode::F10),
                // Fake mouse motion for the viewmodel sway (sway::FakeLook): counts per frame while held.
                "LookR" => Ok(KeyCode::F24),
                "LookL" => Ok(KeyCode::F23),
                "LookU" => Ok(KeyCode::F22),
                _ => return None,
            };
            Some((key, t.parse().ok()?, hold.parse().ok()?))
        }).collect()
    });
    let t = time.elapsed_secs() - r;
    fake.0 = Vec2::ZERO;
    for (k, at, hold) in plan.iter() {
        let (on, off) = (t >= *at && t < at + hold, t >= at + hold);
        match *k {
            Ok(KeyCode::F24) if on => fake.0.x += 12.0,
            Ok(KeyCode::F23) if on => fake.0.x -= 12.0,
            Ok(KeyCode::F22) if on => fake.0.y -= 12.0,
            _ => {}
        }
        match *k {
            Ok(k) if on => keys.press(k),
            Ok(k) if off && keys.pressed(k) => keys.release(k),
            Err(b) if on => mouse.press(b),
            Err(b) if off && mouse.pressed(b) => mouse.release(b),
            _ => {}
        }
    }
}

/// Screenshot/record scheduler. Plan = (sim time from ready, output path); exits 3 s (real) after the last one.
fn shots(mut cmd: Commands, time: Res<Time>, ready: Res<Ready>, mut st: Local<(Option<Vec<(f32, String)>>, usize, Option<std::time::Instant>)>, mut exit: MessageWriter<AppExit>) {
    let Some(r) = ready.0 else { return };
    let (plan, next, tail) = &mut *st;
    let plan = plan.get_or_insert_with(|| {
        let mk = |p: &str| {
            if let Some(d) = std::path::Path::new(p).parent() {
                let _ = std::fs::create_dir_all(d);
            }
        };
        if let Some((dir, fps, dur)) = record_cfg() {
            let _ = std::fs::create_dir_all(&dir);
            return (0..(fps * dur).ceil() as usize).map(|i| (i as f32 / fps, format!("{dir}/f_{:04}.png", i + 1))).collect();
        }
        let base = env("BODYCAM_SCREENSHOT");
        if let Some(s) = env("BODYCAM_SHOTS") {
            let prefix = base.as_deref().map_or("shots/shot", |b| b.strip_suffix(".png").unwrap_or(b)).to_string();
            mk(&prefix);
            return s.split(',').filter_map(|t| t.trim().parse().ok()).enumerate().map(|(i, t)| (t, format!("{prefix}_{}.png", i + 1))).collect();
        }
        let Some(p) = base else { return vec![] };
        mk(&p);
        vec![(env("BODYCAM_SCREENSHOT_AT").and_then(|v| v.parse().ok()).unwrap_or(0.5), p)]
    });
    if plan.is_empty() {
        return;
    }
    let t = time.elapsed_secs() - r;
    // At most one shot per frame; small slack so fixed-step recording hits every frame despite float error.
    if *next < plan.len() && t + 0.004 >= plan[*next].0 {
        cmd.spawn(Screenshot::primary_window()).observe(save_to_disk(plan[*next].1.clone()));
        *next += 1;
    }
    if *next == plan.len() {
        // Let the async PNG writes finish (a fixed-step recording renders far faster than it saves).
        if tail.get_or_insert_with(std::time::Instant::now).elapsed().as_secs_f32() > 3.0 {
            exit.write(AppExit::Success);
        }
    }
}
