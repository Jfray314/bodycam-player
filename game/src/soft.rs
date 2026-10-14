//! Soft particles for smoke/dust sprites (they were cut flat where they met walls/floor): see soft.wesl.
use crate::player::BodycamCamera;
use bevy::asset::embedded_asset;
use bevy::core_pipeline::prepass::DepthPrepass;
use bevy::pbr::{ExtendedMaterial, MaterialExtension};
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct SoftExt {
    /// Fade length (m) from the surface behind the sprite.
    #[uniform(100)]
    pub dist: f32,
}
impl MaterialExtension for SoftExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://game/soft.wesl".into()
    }
}
pub type SoftMat = ExtendedMaterial<StandardMaterial, SoftExt>;

pub struct SoftPlugin;
impl Plugin for SoftPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "soft.wesl");
        app.add_plugins(MaterialPlugin::<SoftMat>::default()).add_systems(Update, |mut cmd: Commands, q: Query<Entity, Added<BodycamCamera>>| {
            for e in &q {
                cmd.entity(e).insert(DepthPrepass);
            }
        });
    }
}
