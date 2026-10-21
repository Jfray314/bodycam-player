//! Tablet weapon preview (WEAPON / MODIFY / SELECT screens): the gun glb with its mounted attachments on its own render
//! layer, lit and drawn into an image the screens show, like the real menu's 3D render of the current build.
use super::{Screen, Tablet, Ui};
use crate::ui::settings::Settings;
use crate::viewmodel::{gun_model, root_of_entity};
use crate::weapon::Inventory;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ImageRenderTarget, RenderTarget};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};

pub const LAYER: usize = 6;
pub const W: u32 = 1600;
pub const H: u32 = 700;
/// Far below the maps so nothing else is near it.
const AT: Vec3 = Vec3::new(0.0, -300.0, 0.0);

#[derive(Resource)]
pub struct Showcase {
    pub image: Handle<Image>,
    key: String,
    root: Option<Entity>,
    mounted: bool,
}

#[derive(Component)]
struct ShowCam;
#[derive(Component)]
struct ShowRoot;
#[derive(Component, Clone)]
struct ShowAtt;

pub struct ShowcasePlugin;
impl Plugin for ShowcasePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, setup).add_systems(Update, (sync, layer).chain());
    }
}

fn setup(mut cmd: Commands, mut images: ResMut<Assets<Image>>) {
    let image = images.add(Image::new_target_texture(W, H, TextureFormat::Rgba8UnormSrgb, None));
    // Side view: the gun's muzzle (joint -Y) to the left, its up (joint +Z) up, seen from its right side.
    cmd.spawn((
        ShowCam,
        Camera3d::default(),
        Camera { order: -6, is_active: false, clear_color: ClearColorConfig::Custom(Color::NONE), ..default() },
        RenderTarget::Image(ImageRenderTarget { handle: image.clone(), scale_factor: 1.0 }),
        Projection::Perspective(PerspectiveProjection { fov: 24f32.to_radians(), near: 0.05, aspect_ratio: W as f32 / H as f32, ..default() }),
        bevy::core_pipeline::tonemapping::Tonemapping::AcesFitted,
        RenderLayers::layer(LAYER),
        Transform::from_translation(AT + Vec3::new(-0.16, 0.03, 1.15)).looking_at(AT + Vec3::new(-0.16, 0.0, 0.0), Vec3::Y),
    ));
    for (p, i) in [(Vec3::new(-0.6, 0.9, 1.3), 2_500_000.0), (Vec3::new(0.9, 0.3, 1.0), 900_000.0), (Vec3::new(0.0, 0.6, -1.2), 1_200_000.0)] {
        cmd.spawn((PointLight { intensity: i, range: 6.0, shadow_maps_enabled: false, ..default() }, RenderLayers::layer(LAYER), Transform::from_translation(AT + p)));
    }
    cmd.insert_resource(Showcase { image, key: String::new(), root: None, mounted: false });
}

/// Shows ui.weapon with the player's picks while a weapon screen is up; rebuilds when the gun or its picks change.
#[allow(clippy::too_many_arguments)]
fn sync(
    mut cmd: Commands,
    assets: Res<AssetServer>,
    mut sc: ResMut<Showcase>,
    ui: Res<Ui>,
    tablet: Res<Tablet>,
    cfg: Option<Res<Settings>>,
    inv: Option<Single<&Inventory>>,
    mut cam: Single<&mut Camera, With<ShowCam>>,
    joints: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    mut vis: Query<&mut Visibility>,
) {
    let (Some(cfg), Some(inv)) = (cfg, inv) else { return };
    let on = tablet.shown && matches!(ui.screen, Screen::Weapon | Screen::Modify | Screen::Select);
    cam.is_active = on;
    if !on {
        return;
    }
    let row = ui.weapon.clone();
    let picks = crate::attach::picks(&cfg, &row);
    let key = format!("{row}:{}", picks.join(","));
    if key != sc.key {
        if let Some(r) = sc.root.take() {
            cmd.entity(r).despawn();
        }
        sc.key = key;
        sc.mounted = false;
        let model = inv.slots.iter().chain(inv.pool.iter()).find(|w| w.name == row).and_then(|w| gun_model(&w.name, &w.anims));
        if let Some(m) = model {
            let scene: Handle<WorldAsset> = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/guns/{m}.glb")));
            // joint frame -> view: muzzle (-Y) to -X, up (+Z) to +Y
            let rot = Quat::from_mat3(&Mat3::from_cols(Vec3::Z, Vec3::X, Vec3::Y));
            sc.root = Some(cmd.spawn((ShowRoot, Name::new("showcase_gun"), WorldAssetRoot(scene), Transform::from_translation(AT).with_rotation(rot), Visibility::default())).id());
        }
    }
    if let Some(root) = sc.root
        && !sc.mounted
        && joints.iter().any(|(e, n)| n.as_str().eq_ignore_ascii_case("root") && root_of_entity(e, root, &parents))
    {
        sc.mounted = true;
        crate::attach::mount_parts(&mut cmd, &assets, root, &row, &picks, &joints, &parents, &mut vis, ShowAtt);
    }
}

/// Everything under the preview gun renders only on the preview layer.
fn layer(mut cmd: Commands, sc: Res<Showcase>, new: Query<Entity, Added<Mesh3d>>, parents: Query<&ChildOf>) {
    let Some(root) = sc.root else { return };
    for e in &new {
        if root_of_entity(e, root, &parents) {
            cmd.entity(e).insert((RenderLayers::layer(LAYER), NoFrustumCulling));
        }
    }
}
