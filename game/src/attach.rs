//! Weapon attachments on the viewmodel gun: data from DT_WeaponItems/DT_WeaponSkins via tools/attlib.py + tools/attsockets.py
//! (assets/attachments.json: per-gun mount sockets in gun-joint space, per-item AimPoint/Reticle/Light/Laser sockets), meshes in
//! assets-local/models/attachments/<id>.glb. Optic aim points feed WEP.GetActiveAimpointComponent (viewmodel::place_arms);
//! C = GA_ToggleAttachment_Flashlight, Z = GA_ToggleAttachment_Laser, MMB = GA_SwitchMagnifiedOptic/SwitchUsedOptic (IMC_KBM_CharacterOnFoot).
use crate::player::IgnoreTraces;
use crate::ui::settings::Settings;
use crate::viewmodel::{root_of_entity, GunRoot};
use crate::weapon::{Inventory, Regrip};
use bevy::camera::visibility::NoFrustumCulling;
use bevy::gltf::GltfMaterialName;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Selectable attachments (assets/attachments.json items; labels from DT_WeaponSkins ItemName).
pub const IDS: [(&str, &str); 9] = [
    ("reddot", "RED DOT"),
    ("holo", "HOLOGRAPHIC"),
    ("acog", "ACOG 4X"),
    ("canted", "CANTED DOT"),
    ("suppressor", "SUPPRESSOR"),
    ("laser", "LASER (GREEN)"),
    ("peq", "LASER (RED)"),
    ("flashlight", "FLASHLIGHT"),
    ("foregrip", "FOREGRIP"),
];

#[derive(Deserialize, Clone, Debug)]
pub struct Xf {
    pub loc: [f32; 3],
    pub rot: [f32; 4],
    #[serde(default)]
    pub bone: String,
}
impl Xf {
    pub fn at(&self, p: Vec3) -> Vec3 {
        Vec3::from(self.loc) + Quat::from_array(self.rot) * p
    }
    fn tf(&self) -> Transform {
        Transform { translation: self.loc.into(), rotation: Quat::from_array(self.rot), ..default() }
    }
}
#[derive(Deserialize, Debug)]
pub struct Item {
    pub kind: String,
    pub sockets: HashMap<String, Xf>,
}
#[derive(Deserialize, Debug)]
pub struct Data {
    pub items: HashMap<String, Item>,
    pub guns: HashMap<String, HashMap<String, Xf>>,
    pub rows: HashMap<String, String>,
}

pub fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(include_str!("../../assets/attachments.json")).expect("assets/attachments.json"))
}

/// Gun socket candidates for an attachment kind (ItemClassID parent tag -> socket on the receiver or its barrel/forearm part).
fn candidates(kind: &str) -> &'static [&'static str] {
    match kind {
        "optic" | "magnified" => &["optic", "optic_mount"],
        "canted" => &["canted_optic_mount"],
        "muzzle" => &["muzzle"],
        "laser" => &["upperbarrel", "siderail"],
        "flashlight" => &["siderail", "underbarrel"],
        "underbarrel" => &["underbarrel"],
        _ => &[],
    }
}

/// (socket name, transform) where `id` mounts on weapon `row`, if the gun has a matching socket.
pub fn mount(row: &str, id: &str) -> Option<(&'static str, &'static Xf)> {
    let d = data();
    let sock = d.guns.get(d.rows.get(row)?)?;
    let kind = d.items.get(id)?.kind.as_str();
    candidates(kind).iter().find_map(|c| sock.get(*c).map(|x| (*c, x)))
}

/// Attachment ids selectable on `row` (the tablet MODIFY screen).
pub fn available(row: &str) -> Vec<(&'static str, &'static str)> {
    IDS.iter().filter(|(id, _)| mount(row, id).is_some()).copied().collect()
}

/// Toggle `id` on `row`; ids that share the gun socket are replaced (one optic, one siderail device...).
pub fn toggle(s: &mut Settings, row: &str, id: &str) {
    if s.has_att(row, id) {
        s.toggle_att(row, id);
        return;
    }
    let slot = mount(row, id).map(|m| m.0);
    let others: Vec<String> = s.attachments.get(row).into_iter().flatten().filter(|o| mount(row, o).map(|m| m.0) == slot).cloned().collect();
    for o in others {
        s.toggle_att(row, &o);
    }
    s.toggle_att(row, id);
}

/// Mounted ids that still fit the gun, optics ordered primary first.
pub fn loadout(s: &Settings, row: &str) -> Vec<String> {
    s.attachments.get(row).into_iter().flatten().filter(|id| mount(row, id).is_some()).cloned().collect()
}

fn is_optic(id: &str) -> bool {
    matches!(data().items.get(id).map(|i| i.kind.as_str()), Some("optic" | "magnified" | "canted"))
}

#[derive(Resource, Default)]
pub struct AttState {
    /// WEP.CurrentOpticIndex: 0 = Optic socket sight, 1 = canted sight.
    pub optic: usize,
    pub flashlight: bool,
    pub laser: bool,
    /// Active optic's AimPoint (gun bone, joint-space point) while an optic is mounted: WEP.GetActiveAimpointComponent.
    pub aim: Option<(String, Vec3)>,
    /// Magnification of the active scope (1 = none).
    pub zoom: f32,
}

#[derive(Component)]
struct AttMount;
#[derive(Component)]
struct FlashLight;
#[derive(Component)]
struct LaserBeam;
#[derive(Component)]
struct LaserDot;

pub struct AttachPlugin;
impl Plugin for AttachPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AttState>().add_systems(Update, (mount_sys, glass, switch_input, state, laser).chain());
    }
}

/// Active optic id: the primary (Optic socket) sight at index 0, the canted one at 1; falls back to whichever is mounted.
fn active_optic<'a>(ids: &'a [String], idx: usize) -> Option<&'a str> {
    let kind = |id: &str| data().items.get(id).map(|i| i.kind.as_str());
    let primary = ids.iter().find(|i| matches!(kind(i), Some("optic" | "magnified")));
    let canted = ids.iter().find(|i| kind(i) == Some("canted"));
    (if idx == 1 { canted.or(primary) } else { primary.or(canted) }).map(String::as_str)
}

fn mount_sys(
    mut cmd: Commands,
    assets: Res<AssetServer>,
    cfg: Option<Res<Settings>>,
    inv: Option<Single<&Inventory>>,
    tablet: Res<crate::ui::Tablet>,
    guns: Query<(Entity, &GunRoot)>,
    joints: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    old: Query<Entity, With<AttMount>>,
    mut done: Local<(Option<Entity>, String)>,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let (Some(inv), Some(cfg)) = (inv, cfg) else { return };
    let Some((gun, g)) = guns.iter().next() else { return };
    if tablet.shown || g.0 == "Tablet" {
        return;
    }
    let row = inv.slots[inv.current].name.clone();
    let ids = loadout(&cfg, &row);
    let key = format!("{row}:{}", ids.join(","));
    if *done == (Some(gun), key.clone()) {
        return;
    }
    // the gun scene must have spawned its joints
    if !joints.iter().any(|(e, n)| n.as_str().eq_ignore_ascii_case("root") && root_of_entity(e, gun, &parents)) {
        return;
    }
    for e in &old {
        cmd.entity(e).despawn();
    }
    *done = (Some(gun), key);
    let d = data();
    let reticle = reticle_mat(&mut images, &mut mats);
    for id in &ids {
        let Some((_, xf)) = mount(&row, id) else { continue };
        let Some(bone) = joints.iter().find(|(e, n)| n.as_str().eq_ignore_ascii_case(&xf.bone) && root_of_entity(*e, gun, &parents)).map(|j| j.0) else { continue };
        let item = &d.items[id.as_str()];
        let scene: Handle<bevy::world_serialization::WorldAsset> = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/attachments/{id}.glb")));
        let root = cmd.spawn((AttMount, Name::new(format!("att_{id}")), WorldAssetRoot(scene), xf.tf())).id();
        cmd.entity(bone).add_child(root);
        // WEP optics: the reticle quad sits on the optic's Reticle socket (MI_ReticleData_*: ReticleColor red, unlit).
        // ponytail: procedural dot/crosshair stands in for the reticle textures of the MI_ReticleData parents.
        if let Some(r) = item.sockets.get("reticle").or_else(|| item.sockets.get("reticle_magnified")).or_else(|| item.sockets.get("reticle_canted")).filter(|_| is_optic(id)) {
            let size = if item.kind == "magnified" { 0.012 } else { 0.007 };
            cmd.spawn((
                Mesh3d(meshes.add(Rectangle::new(size, size))),
                MeshMaterial3d(reticle.clone()),
                r.tf() * Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                NoFrustumCulling,
                IgnoreTraces,
                ChildOf(root),
            ));
        }
        if let Some(l) = item.sockets.get("light") {
            // Flashlight BP: MainLight SpotLight inner 8 / outer 25 deg, attenuation 3200 cm, LightFalloffExponent 5.
            cmd.spawn((
                FlashLight,
                SpotLight { intensity: 400_000.0, range: 32.0, inner_angle: 8f32.to_radians(), outer_angle: 25f32.to_radians(), shadow_maps_enabled: false, ..default() },
                Visibility::Hidden,
                l.tf() * Transform::from_rotation(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)),
                ChildOf(root),
            ));
        }
        if let Some(l) = item.sockets.get("socket_laser") {
            // Laser BP: SM_Laser_Beam stretched to the trace hit (x1000 scale when nothing is hit), sphere dot at the hit.
            let col = if id == "laser" { Color::srgb(0.1, 1.0, 0.2) } else { Color::srgb(1.0, 0.05, 0.05) };
            let m = mats.add(StandardMaterial { base_color: col, emissive: col.to_linear() * 8.0, unlit: true, alpha_mode: AlphaMode::Add, ..default() });
            cmd.spawn((LaserBeam, Mesh3d(meshes.add(Cuboid::new(0.0015, 1.0, 0.0015).mesh().build().translated_by(Vec3::new(0.0, -0.5, 0.0)))), MeshMaterial3d(m.clone()), Visibility::Hidden, l.tf(), NoFrustumCulling, IgnoreTraces, ChildOf(root)));
            cmd.spawn((AttMount, LaserDot, Mesh3d(meshes.add(Sphere::new(0.012))), MeshMaterial3d(m), Visibility::Hidden, Transform::default(), NoFrustumCulling, IgnoreTraces));
        }
    }
}

/// Procedural red-dot reticle: bright core + halo (MI_ReticleData ReticleIlluminationIntensity/ReticleHalo), alpha-blended.
fn reticle_mat(images: &mut Assets<Image>, mats: &mut Assets<StandardMaterial>) -> Handle<StandardMaterial> {
    use bevy::asset::RenderAssetUsages;
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
    const N: u32 = 64;
    let mut px = Vec::with_capacity((N * N * 4) as usize);
    for y in 0..N {
        for x in 0..N {
            let d = (Vec2::new(x as f32, y as f32) - Vec2::splat(N as f32 / 2.0 - 0.5)).length() / (N as f32 / 2.0);
            let a = (1.0 - ((d - 0.12) / 0.04).clamp(0.0, 1.0)).max((1.0 - d).clamp(0.0, 1.0).powi(3) * 0.35);
            px.extend_from_slice(&[255, 20, 20, (a * 255.0) as u8]);
        }
    }
    let img = Image::new(Extent3d { width: N, height: N, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::RENDER_WORLD);
    let tex = images.add(img);
    mats.add(StandardMaterial {
        base_color_texture: Some(tex.clone()),
        emissive_texture: Some(tex),
        emissive: LinearRgba::rgb(6.0, 6.0, 6.0),
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        cull_mode: None,
        depth_bias: 4.0,
        ..default()
    })
}

/// Optic/scope glass slots (MI_OpticGlass_*, MI_ScopeOpticGlass_*) are see-through: the glb has them as opaque PBR.
fn glass(mut mats: ResMut<Assets<StandardMaterial>>, new: Query<(&GltfMaterialName, &MeshMaterial3d<StandardMaterial>), Added<GltfMaterialName>>) {
    for (n, mh) in &new {
        let l = n.0.to_lowercase();
        if (l.contains("glass") || l.contains("lens")) && let Some(mut m) = mats.get_mut(&mh.0) {
            m.base_color = Color::srgba(0.7, 0.85, 0.9, 0.08);
            m.alpha_mode = AlphaMode::Blend;
            m.metallic = 0.0;
            m.perceptual_roughness = 0.05;
            m.reflectance = 0.2;
        }
    }
}

/// C / Z toggles (GA_ToggleAttachment_*), MMB = SwitchUsedOptic (+ ForceReGripMotion(0.1)).
fn switch_input(keys: Res<ButtonInput<KeyCode>>, mouse: Res<ButtonInput<MouseButton>>, tablet: Res<crate::ui::Tablet>, mut st: ResMut<AttState>, mut regrip: MessageWriter<Regrip>, cfg: Option<Res<Settings>>, inv: Option<Single<&Inventory>>) {
    if tablet.shown {
        return;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        st.flashlight = !st.flashlight;
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        st.laser = !st.laser;
    }
    if mouse.just_pressed(MouseButton::Middle)
        && let (Some(cfg), Some(inv)) = (cfg, inv)
    {
        let ids = loadout(&cfg, &inv.slots[inv.current].name);
        if ids.iter().filter(|i| is_optic(i)).count() > 1 {
            st.optic = 1 - st.optic;
            regrip.write(Regrip { delay: 0.1 });
        }
    }
}

/// Per-frame derived state: active AimPoint (optic socket on its mount), zoom, light/laser visibility.
fn state(
    cfg: Option<Res<Settings>>,
    inv: Option<Single<&Inventory>>,
    mut st: ResMut<AttState>,
    mut lights: Query<&mut Visibility, (With<FlashLight>, Without<LaserBeam>)>,
    mut beams: Query<&mut Visibility, (With<LaserBeam>, Without<FlashLight>, Without<LaserDot>)>,
) {
    let (Some(inv), Some(cfg)) = (inv, cfg) else { return };
    let row = &inv.slots[inv.current].name;
    let ids = loadout(&cfg, row);
    let optic = active_optic(&ids, st.optic);
    st.aim = optic.and_then(|id| {
        let (_, xf) = mount(row, id)?;
        let a = data().items[id].sockets.get("aimpoint")?;
        // AdsSettings.AimSightDistance / AimMagnifiedSightDistance (cm): the eye sits that far behind the optic's AimPoint.
        let ads = &inv.slots[inv.current].anims["AdsSettings"];
        let mag = data().items[id].kind == "magnified";
        let d = ads[if mag { "AimMagnifiedSightDistance" } else { "AimSightDistance" }].as_f64().unwrap_or(if mag { 2.5 } else { 12.0 }) as f32;
        Some((xf.bone.clone(), xf.at(Vec3::from(a.loc)) + Vec3::Y * d * 0.01))
    });
    st.zoom = if optic.is_some_and(|o| data().items[o].kind == "magnified") { 4.0 } else { 1.0 };
    let (fl, la) = (st.flashlight, st.laser);
    for mut v in &mut lights {
        *v = if fl { Visibility::Inherited } else { Visibility::Hidden };
    }
    for mut v in &mut beams {
        *v = if la { Visibility::Inherited } else { Visibility::Hidden };
    }
}

/// Laser: beam stretched to the first hit along the socket's forward (LineTraceSingle), dot at the hit.
fn laser(
    st: Res<AttState>,
    mut beams: Query<(&mut Transform, &GlobalTransform), (With<LaserBeam>, Without<LaserDot>)>,
    mut dots: Query<(&mut Transform, &mut Visibility), With<LaserDot>>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
) {
    let Some((mut bt, bg)) = beams.iter_mut().next() else { return };
    // the laser mesh points along the item's UE +Y = joint -Y
    let fwd = Dir3::new(-Vec3::from(bg.affine().matrix3.y_axis).normalize_or_zero()).unwrap_or(Dir3::NEG_Z);
    let origin = bg.translation();
    let hit = if st.laser {
        let filter = |e: Entity| !ignore.contains(e);
        ray.cast_ray(Ray3d::new(origin, fwd), &MeshRayCastSettings::default().with_filter(&filter)).first().map(|(_, h)| h.distance)
    } else {
        None
    };
    bt.scale = Vec3::new(1.0, hit.unwrap_or(100.0).min(100.0), 1.0);
    if let Some((mut dt, mut dv)) = dots.iter_mut().next() {
        dt.translation = origin + *fwd * hit.unwrap_or(0.0);
        *dv = if hit.is_some() { Visibility::Inherited } else { Visibility::Hidden };
    }
}
