//! Bodycam camera look. Bodycam_Player's BodycamBorders (a photographed lens-housing plane on a spring arm) is drawn
//! as a smooth circular corner mask in postfx.wesl instead (the photo's rim was lumpy and off-centre). Then the player CameraComponent blends M_PP_BodycamMaster (lens distortion driven by
//! MPC_PostProcessMaterialSettings + per-channel chromatic aberration) then MI_PP_DiskBlur (priority 400).
//! Parameter values are the cooked ones; the node graphs themselves are compiled away, so the maths is
//! reconstructed. The dark rounded frame edge is the distortion sampling off-screen, not a vignette node.
//! Camera PP settings are UE defaults (no overrides): bloom 0.675, vignette 0.4, histogram auto exposure.
//! F2 toggles the whole stack. `BODYCAM_SCREENSHOT=path` saves a frame ~120 and exits.
use bevy::asset::{RenderAssetUsages, embedded_asset};
use bevy::core_pipeline::fullscreen_material::{FullscreenMaterial, FullscreenMaterialPlugin};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::post_process::auto_exposure::{AutoExposure, AutoExposurePlugin};
use bevy::post_process::bloom::Bloom;
use bevy::post_process::motion_blur::MotionBlur;
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use bevy::render::render_resource::{Extent3d, ShaderType, TextureDimension, TextureFormat};
use bevy::render::RenderApp;
use bevy::shader::ShaderRef;

use crate::player::BodycamCamera;

/// Field order must match `struct Fx` in postfx.wesl.
#[derive(Component, ExtractComponent, Clone, Copy, ShaderType)]
#[extract_app(RenderApp)]
pub struct BodycamPostFx {
    /// MPC LensDistortionIntensity / LensDistortionDensity (ExponentialDensity of r^2).
    pub distortion: f32,
    pub distortion_density: f32,
    /// UV scale applied to the material's Red/Cyan, Green/Magenta, Blue/Yellow offsets (3.5, -1.6, -0.8).
    pub ca: f32,
    /// UE VignetteIntensity (engine default 0.4).
    pub vignette: f32,
    /// M_PP_BodycamMaster AberrationFadeExponent.
    pub aberration_fade_exponent: f32,
    /// MI_PP_DiskBlur BlurRadius (px at 1080p) * BlurRadiusConstant, and BlurFadeExponent.
    pub edge_blur: f32,
    pub blur_fade_exponent: f32,
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
            distortion: 0.33, distortion_density: 1.0, ca: 0.003, vignette: 0.4, aberration_fade_exponent: 2.0,
            edge_blur: 25.0 * 0.8, blur_fade_exponent: 10.0, sharpen: 0.0, // UE Sharpen default 0
            grain: 0.0, temporal_noise: 0.0, banding: 0.0, // FilmGrainIntensity 0
            dirt: 0.25, dirt_threshold: 4.0, dirt_gain: 0.6, exposure_bias: 0.0,
            rolling_shutter: 0.010,
            flash_amount: 0.0, damage_amount: 0.0, explosion_shake: 0.0, dirt_boost: 0.0,
            time: 0.0, yaw_rate: 0.0, pitch_rate: 0.0, pump: 0.0,
            tint_r: 1.0, tint_g: 1.0, tint_b: 1.0,
        }
    }
}

impl FullscreenMaterial for BodycamPostFx {
    fn fragment_shader() -> ShaderRef {
        "embedded://game/postfx.wesl".into()
    }
}

/// Horizontal FOV (UE CameraComponent.FieldOfView = 120, constant, also while aiming).
/// The barrel warp is corner-normalised, so the frame keeps exactly this FOV at the corners.
#[derive(Resource)]
pub struct PostFxState {
    pub base_fov_deg: f32,
    pub enabled: bool,
    mask: Handle<Image>,
}

pub struct PostFxPlugin;
impl Plugin for PostFxPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "postfx.wesl");
        app.add_plugins((FullscreenMaterialPlugin::<BodycamPostFx>::default(), AutoExposurePlugin))
            .add_systems(Startup, setup)
            .add_systems(Update, (attach, toggles, motion_state, apply_fov).chain());
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
    cmd.insert_resource(PostFxState { base_fov_deg: 120.0, enabled: true, mask });
}

fn stack(st: &PostFxState) -> impl Bundle {
    (
        BodycamPostFx::default(),
        Msaa::Off,
        // ponytail: UE BloomIntensity 0.675 (SOG) mapped by eye; Bevy's bloom units differ.
        Bloom { intensity: 0.2, ..Bloom::NATURAL },
        Tonemapping::AcesFitted, // UE filmic tonemapper is ACES-based
        MotionBlur { shutter_angle: 0.5, samples: 4 }, // MotionBlurAmount 0.5
        // Histogram auto exposure, AutoExposureBias +1 (exposure_bias), metering mask stand-in.
        AutoExposure {
            range: -6.0..=6.0,
            speed_brighten: 3.0, // AutoExposureSpeedUp
            speed_darken: 1.0,   // AutoExposureSpeedDown
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

fn toggles(mut cmd: Commands, keys: Res<ButtonInput<KeyCode>>, mut st: ResMut<PostFxState>, cam: Query<Entity, With<BodycamCamera>>) {
    if keys.just_pressed(KeyCode::F2) {
        st.enabled = !st.enabled;
        for e in &cam {
            if st.enabled {
                cmd.entity(e).insert(stack(&st));
            } else {
                cmd.entity(e).remove::<(BodycamPostFx, Bloom, MotionBlur, AutoExposure)>().insert((Tonemapping::TonyMcMapface, Msaa::Sample4));
            }
        }
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

/// UE's FOV is horizontal; Bevy's is vertical.
fn apply_fov(st: Res<PostFxState>, mut q: Query<&mut Projection, With<BodycamCamera>>) {
    for mut proj in &mut q {
        if let Projection::Perspective(p) = &mut *proj {
            p.fov = 2.0 * ((st.base_fov_deg.to_radians() * 0.5).tan() / p.aspect_ratio).atan();
        }
    }
}
