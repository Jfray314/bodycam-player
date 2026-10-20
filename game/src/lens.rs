//! Lens layers over the camera image (see lens.wesl): blood droplets (Bodycam_Player.ScreenDroplets, driven by
//! BodycamPostFx.damage_amount from effects.rs) and the T_DirtMask_01 lens dirt. Drawn as a full-screen UI material so the textures are
//! sampled directly; the scene is never re-sampled (the old sparse-tap dirt ghosted and flashed).
use bevy::asset::embedded_asset;
use bevy::image::{ImageAddressMode, ImageLoaderSettings, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

use crate::postfx::BodycamPostFx;

/// Dirt overlay opacity. ponytail: UE only shows the mask through bloom near bright lights (bloom is off here), so a fixed faint layer.
const DIRT: f32 = 0.35;
/// Material 'Droplet Tiling' (value not in the dump).
const TILING: f32 = 1.0;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct LensMat {
    #[uniform(0)]
    p: Vec4,
    #[texture(1)]
    #[sampler(2)]
    drops: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    dirt: Handle<Image>,
}
impl UiMaterial for LensMat {
    fn fragment_shader() -> ShaderRef {
        "embedded://game/lens.wesl".into()
    }
}

pub struct LensPlugin;
impl Plugin for LensPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "lens.wesl");
        app.add_plugins(UiMaterialPlugin::<LensMat>::default()).add_systems(Startup, spawn).add_systems(Update, drive);
    }
}

fn spawn(mut cmd: Commands, assets: Res<AssetServer>, mut mats: ResMut<Assets<LensMat>>) {
    // Data textures: linear, tiled.
    let drops = assets
        .load_builder()
        .with_settings(|s: &mut ImageLoaderSettings| {
            s.is_srgb = false;
            s.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor { address_mode_u: ImageAddressMode::Repeat, address_mode_v: ImageAddressMode::Repeat, ..ImageSamplerDescriptor::linear() });
        })
        .load("textures/fx/screen_droplets.png");
    let dirt = assets.load("textures/fx/lens_dirt.png");
    let m = mats.add(LensMat { p: Vec4::new(0.0, DIRT, TILING, 0.0), drops, dirt });
    cmd.spawn((
        Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() },
        MaterialNode(m),
        GlobalZIndex(-1), // under any HUD text
        Pickable::IGNORE,
    ));
}

fn drive(fx: Option<Single<&BodycamPostFx>>, mut mats: ResMut<Assets<LensMat>>) {
    let Some(fx) = fx else { return };
    // The death cut tints the frame to black (effects.rs): the lens layers fade with it.
    let (x, y) = (fx.damage_amount, DIRT * fx.tint_g.clamp(0.0, 1.0));
    if mats.iter().any(|(_, m)| m.p.x != x || m.p.y != y) {
        for (_, m) in mats.iter_mut() {
            m.p.x = x;
            m.p.y = y;
        }
    }
}
