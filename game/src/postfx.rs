//! Bodycam camera look (M_PP_BodycamMaster, FishEye, LensDirt, DiskBlur, grain; cooked graphs are not
//! recoverable, so this is a faithful-by-eye recreation). One fullscreen shader (`postfx.wesl`) driven by
//! `BodycamPostFx`, plus Bevy's AutoExposure / Bloom / MotionBlur / Tonemapping / ColorGrading.
//! F2 toggles the whole stack, F3 cycles presets. `BODYCAM_SCREENSHOT=path` saves a frame ~120 and exits.
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::auto_exposure::{AutoExposure, AutoExposurePlugin};
use bevy::post_process::bloom::Bloom;
use bevy::post_process::motion_blur::MotionBlur;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::render::view::{ColorGrading, ColorGradingGlobal, ColorGradingSection};
use bevy::render::RenderApp;
use bevy::shader::ShaderRef;

use crate::player::BodycamCamera;

/// Field order must match `struct Fx` in postfx.wesl.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType)]
#[extract_app(RenderApp)]
pub struct BodycamPostFx {
    /// Fisheye barrel strength (0 = rectilinear). Scene is rendered at an overscanned FOV to compensate.
    pub distortion: f32,
    /// Extra quartic term: stretches/squeezes the very edge.
    pub edge_stretch: f32,
    /// Chromatic aberration at the corner (fraction of radius, R out / B in).
    pub ca: f32,
    /// Vignette darkness 0..1, start radius (corner-normalised r^2), horizontal squash (>1 = wider oval).
    pub vignette: f32,
    pub vignette_radius: f32,
    pub vignette_oval: f32,
    /// DiskBlur radius in pixels at the corner, and r^2 where it starts.
    pub edge_blur: f32,
    pub edge_blur_start: f32,
    /// Centre unsharp-mask amount.
    pub sharpen: f32,
    /// Luma grain, chroma temporal noise, and compression banding blend.
    pub grain: f32,
    pub temporal_noise: f32,
    pub banding: f32,
    /// Lens dirt strength, HDR threshold where dirt lights up, and gain.
    pub dirt: f32,
    pub dirt_threshold: f32,
    pub dirt_gain: f32,
    /// Static exposure bias in EV (on top of auto exposure).
    pub exposure_bias: f32,
    /// Sensor readout time in seconds; 0 disables jello.
    pub rolling_shutter: f32,
    // Gameplay hooks (0..1): GE_Flashed, StartBloodOnCamera, explosion shake, muzzle-flash dirt boost.
    pub flash_amount: f32,
    pub damage_amount: f32,
    pub explosion_shake: f32,
    pub dirt_boost: f32,
    // Runtime state written by this module.
    pub time: f32,
    pub yaw_rate: f32,
    pub pitch_rate: f32,
    /// Exposure pump in EV (overshoot spring), driven by fast camera turns.
    pub pump: f32,
    /// Colour tint multiplier (cool, slightly teal).
    pub tint_r: f32,
    pub tint_g: f32,
    pub tint_b: f32,
}

impl Default for BodycamPostFx {
    fn default() -> Self {
        Self {
            distortion: 0.40, edge_stretch: 0.12, ca: 0.010,
            vignette: 0.80, vignette_radius: 0.28, vignette_oval: 0.85,
            edge_blur: 3.5, edge_blur_start: 0.20, sharpen: 0.9,
            grain: 0.10, temporal_noise: 0.06, banding: 0.20,
            dirt: 0.8, dirt_threshold: 0.9, dirt_gain: 1.5, exposure_bias: -0.9,
            rolling_shutter: 0.010,
            flash_amount: 0.0, damage_amount: 0.0, explosion_shake: 0.0, dirt_boost: 0.0,
            time: 0.0, yaw_rate: 0.0, pitch_rate: 0.0, pump: 0.0,
            tint_r: 0.98, tint_g: 1.0, tint_b: 1.02,
        }
    }
}

impl BodycamPostFx {
    /// F3 presets: game look, heavy cheap-action-cam, clean Axon-ish.
    fn preset(i: usize) -> Self {
        let d = Self::default();
        match i % 3 {
            1 => Self { distortion: 0.65, edge_stretch: 0.25, ca: 0.022, vignette: 0.8, edge_blur: 7.0, grain: 0.18, temporal_noise: 0.1, banding: 0.5, rolling_shutter: 0.02, ..d },
            2 => Self { distortion: 0.2, edge_stretch: 0.0, ca: 0.004, vignette: 0.35, edge_blur: 1.5, grain: 0.04, temporal_noise: 0.02, banding: 0.1, dirt: 0.4, rolling_shutter: 0.004, ..d },
            _ => d,
        }
    }
}

impl FullscreenMaterial for BodycamPostFx {
    fn fragment_shader() -> ShaderRef {
        "embedded://game/postfx.wesl".into()
    }
}

/// Vertical FOV of the unwarped player camera; the render FOV is widened by the distortion overscan.
#[derive(Resource)]
pub struct PostFxState {
    pub base_fov_deg: f32,
    pub enabled: bool,
    preset: usize,
    mask: Handle<Image>,
}

pub struct PostFxPlugin;
impl Plugin for PostFxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "postfx.wesl");
        app.add_plugins((FullscreenMaterialPlugin::<BodycamPostFx>::default(), AutoExposurePlugin))
            .add_systems(Startup, setup)
            .add_systems(Update, (attach, toggles, motion_state, apply_fov, screenshot).chain());
    }
}

fn setup(mut cmd: Commands, mut images: ResMut<Assets<Image>>) {
    // Centre-weighted metering mask (stand-in for T_ExposureMeteringMask_02).
    let n = 32u32;
    let data = (0..n * n)
        .map(|i| {
            let (x, y) = ((i % n) as f32 / (n - 1) as f32 - 0.5, (i / n) as f32 / (n - 1) as f32 - 0.5);
            (255.0 * (1.0 - (x * x + y * y).sqrt() * 1.6).clamp(0.15, 1.0)) as u8
        })
        .collect();
    let mask = images.add(Image::new(
        Extent3d { width: n, height: n, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::R8Unorm,
        RenderAssetUsages::RENDER_WORLD,
    ));
    cmd.insert_resource(PostFxState { base_fov_deg: 100.0, enabled: true, preset: 0, mask });
}

fn stack(st: &PostFxState) -> impl Bundle {
    (
        BodycamPostFx::preset(st.preset),
        Msaa::Off,
        // Strong, hazy bloom.
        Bloom { intensity: 0.28, low_frequency_boost: 0.8, ..Bloom::NATURAL },
        Tonemapping::AgX,
        // Crushed blacks, desaturated cool grade.
        ColorGrading {
            global: ColorGradingGlobal { temperature: -0.04, post_saturation: 0.82, ..default() },
            shadows: ColorGradingSection { gamma: 1.0, lift: -0.02, contrast: 1.12, ..default() },
            ..default()
        },
        // Camera-rotation smear.
        MotionBlur { shutter_angle: 1.0, samples: 4 },
        // Slow sensor adaptation, centre-weighted.
        AutoExposure {
            range: -6.0..=6.0,
            speed_brighten: 0.6,
            speed_darken: 1.2,
            metering_mask: st.mask.clone(),
            ..default()
        },
    )
}

fn attach(mut cmd: Commands, st: Res<PostFxState>, q: Query<Entity, Added<BodycamCamera>>) {
    for e in &q {
        cmd.entity(e).insert(stack(&st));
    }
}

fn toggles(mut cmd: Commands, keys: Res<ButtonInput<KeyCode>>, mut st: ResMut<PostFxState>, mut q: Query<(Entity, &mut BodycamPostFx), With<BodycamCamera>>, cam: Query<Entity, With<BodycamCamera>>) {
    if keys.just_pressed(KeyCode::F2) {
        st.enabled = !st.enabled;
        for e in &cam {
            if st.enabled {
                cmd.entity(e).insert(stack(&st));
            } else {
                cmd.entity(e).remove::<(BodycamPostFx, Bloom, ColorGrading, MotionBlur, AutoExposure)>().insert((Tonemapping::TonyMcMapface, Msaa::Sample4));
            }
        }
    }
    if keys.just_pressed(KeyCode::F3) && st.enabled {
        st.preset += 1;
        for (_, mut fx) in &mut q {
            let p = BodycamPostFx::preset(st.preset);
            // Keep gameplay/runtime state.
            *fx = BodycamPostFx { flash_amount: fx.flash_amount, damage_amount: fx.damage_amount, explosion_shake: fx.explosion_shake, dirt_boost: fx.dirt_boost, time: fx.time, yaw_rate: fx.yaw_rate, pitch_rate: fx.pitch_rate, pump: fx.pump, ..p };
        }
        info!("bodycam postfx preset {}", st.preset % 3);
    }
}

/// Angular velocity (local yaw/pitch, rad/s), animation time, and an exposure-pump spring kicked by fast turns.
fn motion_state(time: Res<Time>, mut q: Query<(&GlobalTransform, &mut BodycamPostFx)>, mut prev: Local<Option<Quat>>, mut vel: Local<f32>) {
    let dt = time.delta_secs().max(1e-4);
    for (gt, mut fx) in &mut q {
        let rot = gt.rotation();
        if let Some(p) = *prev {
            let (axis, ang) = (p.inverse() * rot).to_axis_angle();
            let w = axis * ang / dt;
            let k = (dt * 12.0).min(1.0);
            fx.yaw_rate += (w.y.clamp(-12.0, 12.0) - fx.yaw_rate) * k;
            fx.pitch_rate += (w.x.clamp(-12.0, 12.0) - fx.pitch_rate) * k;
            // Underdamped spring: overshoot after a fast turn.
            let kick = (w.length() - 2.0).max(0.0) * 0.02;
            *vel += (-18.0 * fx.pump - 3.0 * *vel - kick * 60.0) * dt;
            fx.pump += *vel * dt;
        }
        *prev = Some(rot);
        fx.time = time.elapsed_secs() % 3600.0;
    }
}

/// Render wider than the player's 100 degrees so the fisheye never shows black edges.
fn apply_fov(st: Res<PostFxState>, mut q: Query<(&mut Projection, Option<&BodycamPostFx>), With<BodycamCamera>>) {
    for (mut proj, fx) in &mut q {
        if let Projection::Perspective(p) = &mut *proj {
            let base = (st.base_fov_deg.to_radians() * 0.5).tan();
            let over = fx.map_or(1.0, |f| 1.0 + f.distortion + f.edge_stretch);
            p.fov = 2.0 * (base * over).atan();
        }
    }
}

fn screenshot(mut cmd: Commands, mut frame: Local<u32>, mut exit: MessageWriter<AppExit>) {
    let Ok(path) = std::env::var("BODYCAM_SCREENSHOT") else { return };
    *frame += 1;
    if *frame == 120 {
        cmd.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    } else if *frame == 150 {
        exit.write(AppExit::Success);
    }
}
