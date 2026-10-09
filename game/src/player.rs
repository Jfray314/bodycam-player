//! Port of Bodycam_Player (ALS_Base_CharacterBP child) movement + AC_BodycamCamera.
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};

// ponytail: ALS v4 defaults (cm/s -> m/s). Swap for Bodycam_Player CDO values once that export is decoded.
const WALK: f32 = 1.65;
const RUN: f32 = 3.50;
const SPRINT: f32 = 6.00;
const CROUCH: f32 = 1.50;
const JUMP_VEL: f32 = 4.5;
const GRAVITY: f32 = 9.81;
const EYE: f32 = 1.60;
const EYE_CROUCH: f32 = 1.00;
/// Bodycam is chest-mounted: offset below/in front of the eyes.
const BODYCAM_OFFSET: Vec3 = Vec3::new(0.0, -0.30, -0.15);
const SENS: f32 = 0.0022;

pub struct PlayerPlugin;
impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, (grab_cursor, look, movement, bodycam_shake).chain());
    }
}

#[derive(Component, Default)]
pub struct Player {
    pub yaw: f32,
    pub pitch: f32,
    pub vel: Vec3,
    pub grounded: bool,
    pub crouched: bool,
    pub sprinting: bool,
    pub aiming: bool,
    /// Accumulated walk cycle, drives BodycamShake_Walk* equivalents.
    pub step_phase: f32,
    /// Recoil pitch/yaw kick pending, consumed by `look`.
    pub recoil: Vec2,
}

#[derive(Component)]
pub struct BodycamCamera;

fn spawn(mut cmd: Commands) {
    cmd.spawn((Player { grounded: true, ..default() }, Transform::from_xyz(0.0, 0.0, 0.0), Visibility::default()))
        .with_children(|p| {
            p.spawn((
                BodycamCamera,
                Camera3d::default(),
                // Bodycam's look relies on a very wide lens; default setting is high FOV.
                Projection::Perspective(PerspectiveProjection { fov: 100f32.to_radians(), near: 0.02, ..default() }),
                Transform::from_xyz(0.0, EYE, 0.0) * Transform::from_translation(BODYCAM_OFFSET),
            ));
        });
}

fn grab_cursor(mouse: Res<ButtonInput<MouseButton>>, keys: Res<ButtonInput<KeyCode>>, mut cursor: Single<&mut CursorOptions>) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

fn look(motion: Res<AccumulatedMouseMotion>, cursor: Single<&CursorOptions>, mut q: Single<&mut Player>) {
    let p = &mut **q;
    let d = if cursor.grab_mode == CursorGrabMode::Locked { motion.delta } else { Vec2::ZERO };
    let sens = if p.aiming { SENS * 0.6 } else { SENS };
    p.yaw -= d.x * sens + p.recoil.x;
    p.pitch = (p.pitch - d.y * sens + p.recoil.y).clamp(-1.5, 1.5);
    p.recoil = Vec2::ZERO;
}

fn movement(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut q: Single<(&mut Player, &mut Transform)>) {
    let (p, tf) = &mut *q;
    let dt = time.delta_secs();
    let input = Vec2::new(
        keys.pressed(KeyCode::KeyD) as i8 as f32 - keys.pressed(KeyCode::KeyA) as i8 as f32,
        keys.pressed(KeyCode::KeyW) as i8 as f32 - keys.pressed(KeyCode::KeyS) as i8 as f32,
    )
    .normalize_or_zero();
    p.crouched = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::KeyC);
    // GA_Character_Sprint: forward only, blocked by aim/crouch (GE_BlockSprint).
    p.sprinting = keys.pressed(KeyCode::ShiftLeft) && input.y > 0.0 && !p.crouched && !p.aiming;
    let speed = if p.crouched { CROUCH } else if p.sprinting { SPRINT } else if keys.pressed(KeyCode::AltLeft) || p.aiming { WALK } else { RUN };

    let rot = Quat::from_rotation_y(p.yaw);
    let wish = rot * Vec3::new(input.x, 0.0, -input.y) * speed;
    let accel = if p.grounded { 12.0 } else { 2.0 };
    let horiz = Vec3::new(p.vel.x, 0.0, p.vel.z).lerp(wish, (accel * dt).min(1.0));
    p.vel.x = horiz.x;
    p.vel.z = horiz.z;

    if p.grounded && keys.just_pressed(KeyCode::Space) && !p.crouched {
        p.vel.y = JUMP_VEL;
        p.grounded = false;
    }
    p.vel.y -= GRAVITY * dt;
    tf.translation += p.vel * dt;
    // ponytail: flat ground at y=0 only; add a collision crate when real geometry exists.
    if tf.translation.y <= 0.0 {
        tf.translation.y = 0.0;
        p.vel.y = 0.0;
        p.grounded = true;
    }
    tf.rotation = rot;
    if p.grounded {
        p.step_phase += horiz.length() * dt * 1.6;
    }
}

/// Approximates AC_BodycamCamera: walk pitch/roll/yaw shake, sprint roll, crouch height blend.
fn bodycam_shake(time: Res<Time>, player: Single<&Player>, mut cam: Single<&mut Transform, With<BodycamCamera>>) {
    let p = *player;
    let t = p.step_phase * std::f32::consts::TAU;
    let amp = if p.sprinting { 2.0 } else { 1.0 } * if p.aiming { 0.3 } else { 1.0 };
    let pitch = p.pitch + (t * 2.0).sin() * 0.008 * amp;
    let roll = t.sin() * 0.012 * amp;
    let yaw = t.cos() * 0.004 * amp;
    let target_eye = if p.crouched { EYE_CROUCH } else { EYE };
    let eye = cam.translation.y - BODYCAM_OFFSET.y;
    let eye = eye + (target_eye - eye) * (10.0 * time.delta_secs()).min(1.0);
    cam.translation = Vec3::new(0.0, eye, 0.0) + BODYCAM_OFFSET + Vec3::Y * (t * 2.0).sin().abs() * 0.02 * amp;
    cam.rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll);
}
