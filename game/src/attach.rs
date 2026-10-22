//! Weapon attachments on the viewmodel gun: every DT_WeaponItems attachment that fits (tools/attall.py, tools/attdata.py ->
//! assets/attachments.json: receiver sockets per gun, per-item sockets / stat modifiers / reticle), meshes in
//! assets-local/models/attachments/<glb>.glb (tools/build_attachments.py). Optic aim points feed WEP.GetActiveAimpointComponent (viewmodel::place_arms);
//! C = GA_ToggleAttachment_Flashlight, Z = GA_ToggleAttachment_Laser, MMB = GA_SwitchMagnifiedOptic/SwitchUsedOptic (IMC_KBM_CharacterOnFoot).
use crate::player::IgnoreTraces;
use crate::ui::settings::Settings;
use crate::viewmodel::{root_of_entity, GunRoot};
use crate::weapon::{Inventory, Regrip};
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::camera::{ImageRenderTarget, RenderTarget, Viewport};
use bevy::render::render_resource::TextureFormat;
use bevy::window::PrimaryWindow;

/// RenderLayers of the arms/gun/attachments: the scope camera renders layer 0 only, so the gun is not in its picture.
pub const VM_LAYER: usize = 2;
use bevy::gltf::GltfMaterialName;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::OnceLock;

/// Every attachment the guns can take (tools/attall.py + tools/attdata.py -> assets/attachments.json): DT_WeaponItems
/// ItemCompatibilities decide what fits, DT_WeaponSkins the visible options. `cls` is the ItemClassID under Item.Attachment
/// ("Muzzle.Suppressor"), `slot` the socket it mounts on (its first segment, snake case: "muzzle", "optic_mount", ...).
#[derive(Deserialize, Clone, Debug)]
pub struct Xf {
    pub loc: [f32; 3],
    pub rot: [f32; 4],
    #[serde(default)]
    pub bone: String,
}
impl Xf {
    fn tf(&self) -> Transform {
        Transform { translation: self.loc.into(), rotation: Quat::from_array(self.rot), ..default() }
    }
}
#[derive(Deserialize, Debug)]
pub struct Item {
    pub cls: String,
    pub slot: String,
    pub label: String,
    /// Mesh file (assets-local/models/attachments/<glb>.glb); skins of one mesh + material set share it.
    pub glb: String,
    /// Mesh sockets (item frame): AimPoint / Reticle* / Viewpoint / Light / Socket_Laser / FireLocation and child mounts.
    pub sockets: HashMap<String, Xf>,
    /// GE_AttachmentStat_* modifiers: (WeaponAttributeSet attribute, "mul" | "add" | "set", value).
    pub fx: Vec<(String, String, f32)>,
    #[serde(default)]
    pub reticle: Option<Reticle>,
    /// Cosmetic.Weapon.Anim.Idle: left-hand grip pose clip (assets/grip_poses.json).
    #[serde(default)]
    pub grip: Option<String>,
    /// DT_WeaponSkins ItemIcons texture (assets-local/ui/tex/icons/<icon>.png).
    #[serde(default)]
    pub icon: Option<String>,
}
#[derive(Deserialize, Debug)]
pub struct Data {
    pub items: HashMap<String, Item>,
    /// Receiver sockets per gun glb (gun-joint space, with their bone).
    pub guns: HashMap<String, HashMap<String, Xf>>,
    pub rows: HashMap<String, String>,
    /// Weapon row -> ids that fit / its DefaultAttachments.
    pub compat: HashMap<String, Vec<String>>,
    pub defaults: HashMap<String, Vec<String>>,
    /// Gun glb -> node names of the default parts baked into it (tools/attachments.py).
    pub baked: HashMap<String, Vec<String>>,
    /// Ids of the old 9-item library (saved settings) -> skin ids.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// Weapon row -> class ("Assault Rifles", "Pistols"...): the SWAP screen tabs.
    #[serde(default)]
    pub classes: HashMap<String, String>,
}

/// Optic reticle (MI_ReticleData_* of the gun's default Reticle item over the optic's OpticReticleMaterial): texture/colour/scale,
/// scope Magnification.
#[derive(Deserialize, Debug)]
pub struct Reticle {
    pub tex: String,
    pub color: [f32; 3],
    pub intensity: f32,
    /// ReticleScaleAndOffset (x, y scale of the texture on the reticle window).
    pub scale: [f32; 4],
    #[serde(default)]
    pub mag: Option<f32>,
}

pub fn data() -> &'static Data {
    static D: OnceLock<Data> = OnceLock::new();
    D.get_or_init(|| serde_json::from_str(include_str!("../../assets/attachments.json")).expect("assets/attachments.json"))
}

/// Ids that fit weapon `row` (the tablet MODIFY list).
pub fn available(row: &str) -> &'static [String] {
    data().compat.get(row).map_or(&[], |v| v.as_slice())
}

/// Slots the current build has a socket for (an optic needs the AK's optic mount, a muzzle device a barrel ...), most used
/// first: the MODIFY tabs.
pub fn slots(s: &Settings, row: &str) -> Vec<&'static str> {
    const ORDER: [&str; 12] = ["optic", "canted_optic", "muzzle", "barrel", "underbarrel", "siderail", "upperbarrel", "stock", "grip", "magazine", "trigger", "slide"];
    let (_, avail) = place(row, &loadout(s, row));
    let mut v: Vec<&'static str> = available(row).iter().map(|id| data().items[id.as_str()].slot.as_str()).filter(|s| avail.contains_key(*s)).collect();
    v.sort_by_key(|s| (ORDER.iter().position(|o| o == s).unwrap_or(ORDER.len()), *s));
    v.dedup();
    v
}

pub fn label(id: &str) -> &str {
    data().items.get(id).map_or(id, |i| i.label.as_str())
}

/// Toggle a pick on `row`: it replaces the default and any other pick of the same slot; toggling a pick off restores the default.
pub fn toggle(s: &mut Settings, row: &str, id: &str) {
    let Some(item) = data().items.get(id) else { return };
    for x in s.attachments.get_mut(row).into_iter().flatten() {
        if let Some(n) = data().aliases.get(x.as_str()) {
            *x = n.clone(); // old library id saved earlier
        }
    }
    if s.has_att(row, id) {
        s.toggle_att(row, id);
        return;
    }
    let same: Vec<String> = s.attachments.get(row).into_iter().flatten().filter(|o| data().items.get(o.as_str()).is_some_and(|i| i.slot == item.slot)).cloned().collect();
    for o in same {
        s.toggle_att(row, &o);
    }
    if !is_default(row, id) {
        s.toggle_att(row, id);
    }
}

pub fn is_default(row: &str, id: &str) -> bool {
    data().defaults.get(row).is_some_and(|d| d.iter().any(|d| d == id))
}

/// The player's picks on `row` that still fit it.
pub fn picks(s: &Settings, row: &str) -> Vec<String> {
    s.attachments.get(row).into_iter().flatten().map(|id| data().aliases.get(id).unwrap_or(id)).filter(|id| available(row).contains(id)).cloned().collect()
}

/// Effective parts: the DefaultAttachments with each pick replacing the default(s) of its slot.
pub fn loadout(s: &Settings, row: &str) -> Vec<String> {
    parts(row, &picks(s, row))
}

/// DefaultAttachments of `row` with `picks` replacing the defaults of their slots.
pub fn parts(row: &str, picks: &[String]) -> Vec<String> {
    let picks: Vec<String> = picks.iter().map(|id| data().aliases.get(id).unwrap_or(id)).filter(|id| available(row).contains(id)).cloned().collect();
    let slot = |id: &str| data().items.get(id).map(|i| i.slot.as_str());
    let mut out: Vec<String> = data().defaults.get(row).into_iter().flatten().filter(|d| !picks.iter().any(|p| slot(p) == slot(d))).cloned().collect();
    out.extend(picks);
    out
}

/// A part placed on the gun: the bone it rides and its transform in that bone's joint frame.
#[derive(Clone, Debug)]
pub struct Placed {
    pub id: String,
    pub bone: String,
    pub tf: Transform,
}

/// Mount tree: each part goes on the socket named after its slot, on the receiver or on another part (a barrel carries the
/// muzzle socket, an optic mount the optic socket ...). A part's sockets replace same-named receiver ones, so a part waits while
/// another unplaced part would still provide its socket. Returns the placed parts and the final socket table.
pub fn place(row: &str, ids: &[String]) -> (Vec<Placed>, HashMap<String, (String, Transform)>) {
    let d = data();
    let mut avail: HashMap<String, (String, Transform)> = d.rows.get(row).and_then(|g| d.guns.get(g)).into_iter().flatten().map(|(n, x)| (n.clone(), (x.bone.clone(), x.tf()))).collect();
    let mut pending: Vec<&str> = ids.iter().map(String::as_str).filter(|i| d.items.contains_key(*i)).collect();
    let mut out = vec![];
    let mut strict = true;
    while !pending.is_empty() {
        let ready = pending.iter().position(|id| {
            let slot = &d.items[*id].slot;
            avail.contains_key(slot) && !(strict && pending.iter().any(|o| o != id && d.items[*o].sockets.contains_key(slot)))
        });
        let Some(k) = ready else {
            if strict {
                strict = false; // a cycle, or the providers can't be placed: take what fits now
                continue;
            }
            break;
        };
        let id = pending.remove(k);
        strict = true;
        let it = &d.items[id];
        let (bone, base) = avail[&it.slot].clone();
        for (n, x) in &it.sockets {
            avail.insert(n.clone(), (bone.clone(), base * x.tf()));
        }
        out.push(Placed { id: id.to_string(), bone, tf: base });
    }
    (out, avail)
}

/// Bullet origin: the mounted muzzle device's FireLocation, else the muzzle socket of the current barrel (bone, joint-frame point, m).
pub fn fire_location(s: &Settings, row: &str) -> Option<(String, Vec3)> {
    if picks(s, row).is_empty() {
        return None; // the gun's own muzzle (weapon.rs)
    }
    let (_, avail) = place(row, &loadout(s, row));
    avail.get("firelocation").or_else(|| avail.get("muzzle")).map(|(b, t)| (b.clone(), t.translation))
}

/// WEP.GetActiveGripData: a part's grip pose clip and the GripPose socket the left hand goes to (gun bone, joint-frame transform).
/// Both are needed (no GripPose socket -> default grip). Defaults-only builds use the baked gun as is: no custom grip.
pub fn grip(s: &Settings, row: &str) -> Option<(&'static str, String, Transform)> {
    let ids = loadout(s, row);
    let pose = ids.iter().find_map(|id| data().items.get(id.as_str())?.grip.as_deref())?;
    let (_, avail) = place(row, &ids);
    avail.get("grippose").map(|(b, t)| (pose, b.clone(), *t))
}

/// Muzzle.Suppressor mounted: Cosmetic.Weapon.SFX.FireSuppressed, no muzzle smoke.
pub fn suppressed(s: &Settings, row: &str) -> bool {
    loadout(s, row).iter().any(|id| data().items.get(id.as_str()).is_some_and(|i| i.cls.starts_with("Muzzle.Suppressor")))
}

/// The mounted parts' GE_AttachmentStat modifiers for one WeaponAttributeSet attribute applied to `base`
/// (UE aggregator: (base + Additive) x Multiplicitive, Override wins).
pub fn stat(s: &Settings, row: &str, attr: &str, base: f32) -> f32 {
    let (mut add, mut mul, mut set) = (0.0, 1.0, None);
    for id in loadout(s, row) {
        for (a, op, v) in data().items.get(id.as_str()).map_or(&[][..], |i| &i.fx[..]) {
            if a == attr {
                match op.as_str() {
                    "add" => add += v,
                    "mul" => mul *= v,
                    _ => set = Some(*v),
                }
            }
        }
    }
    set.unwrap_or((base + add) * mul)
}

/// Sighting optics: parts with an AimPoint in the Optic / Canted Optic slots that are not the gun's default sights.
fn is_optic(row: &str, id: &str) -> bool {
    data().items.get(id).is_some_and(|i| matches!(i.slot.as_str(), "optic" | "canted_optic") && i.sockets.contains_key("aimpoint")) && !is_default(row, id)
}
fn is_magnified(id: &str) -> bool {
    data().items.get(id).and_then(|i| i.reticle.as_ref()).and_then(|r| r.mag).is_some()
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
    /// Entity of the mounted magnified optic (scope camera attaches at its Viewpoint socket).
    pub scope: Option<(Entity, Xf)>,
    /// Eye relief (m) behind the AimPoint of the active optic.
    pub eye_relief: f32,
}

#[derive(Component, Clone)]
struct AttMount;
#[derive(Component)]
struct FlashLight;
#[derive(Component)]
struct LaserBeam;
#[derive(Component)]
struct LaserDot;
/// Red dot / holo reticle quad: aimed shots go eye -> this point (weapon.rs muzzle_basis).
#[derive(Component)]
pub struct ReticleQuad;

pub struct AttachPlugin;
impl Plugin for AttachPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AttState>()
            .add_systems(Startup, scope_setup)
            .add_systems(Update, (preload, apply_mods, mount_sys, body_atts, glass, switch_input, state, laser).chain())
            .add_systems(PostUpdate, scope.after(bevy::transform::TransformSystems::Propagate));
    }
}

/// Every gun's stats / rates = its GE_WeaponStat values with the loadout's GE_AttachmentStat modifiers (on load and on every
/// tablet change). ponytail: the rate / recoil attributes have no GE_WeaponStat base; 1.0 assumed (the BPs read 0 as 1).
fn apply_mods(cfg: Option<Res<Settings>>, inv: Option<Single<&mut Inventory>>, mut seen: Local<bool>) {
    let (Some(cfg), Some(mut inv)) = (cfg, inv) else { return };
    if *seen && !cfg.is_changed() {
        return;
    }
    *seen = true;
    let inv = &mut **inv;
    for w in inv.slots.iter_mut().chain(inv.pool.iter_mut()).filter(|w| w.kind == crate::weapon::ItemKind::Gun) {
        let (r, b) = (w.name.clone(), w.base.clone());
        let f = |attr: &str, v: Option<f32>| v.map(|v| stat(&cfg, &r, attr, v));
        let one = |attr: &str| stat(&cfg, &r, attr, 1.0);
        w.stats = crate::weapon::Stats {
            damage: f("Damage", b.damage),
            fire_rate: f("FireRate", b.fire_rate),
            instability: f("Instability", b.instability),
            mag_amount: f("MagAmount", b.mag_amount),
            ammo_capacity: f("AmmoCapacity", b.ammo_capacity).map(f32::round),
            spread_hip_x: f("SpreadHipX", b.spread_hip_x),
            spread_hip_y: f("SpreadHipY", b.spread_hip_y),
            spread_aim_x: f("SpreadAimX", b.spread_aim_x),
            spread_aim_y: f("SpreadAimY", b.spread_aim_y),
            ..b
        };
        w.mods = crate::weapon::Mods {
            reload: one("ReloadTime"),
            switch: one("SwitchSpeed"),
            ads: one("ADSSpeed"),
            cam_v: one("CameraRecoilStepVertical") * one("CameraRecoilStep"),
            cam_h: one("CameraRecoilStepHorizontal") * one("CameraRecoilStep"),
            pitch: one("VerticalRecoil"),
            yaw: one("HorizontalRecoil"),
            kick: one("KickHip"),
            kick_up: one("KickUp"),
            speed: one("Speed"),
        };
    }
}

/// Active optic: the primary (Optic slot) sight at index 0, the canted one at 1; falls back to whichever is mounted.
fn active_optic<'a>(row: &str, ids: &'a [String], idx: usize) -> Option<&'a str> {
    let slot = |id: &str| data().items.get(id).map(|i| i.slot.as_str());
    let primary = ids.iter().find(|i| is_optic(row, i) && slot(i) == Some("optic"));
    let canted = ids.iter().find(|i| is_optic(row, i) && slot(i) == Some("canted_optic"));
    (if idx == 1 { canted.or(primary) } else { primary.or(canted) }).map(String::as_str)
}

/// Mounts `row`'s parts (`picks` over the defaults) on a spawned gun scene: the baked default parts that get placed or replaced
/// are hidden, the rest spawn under their bones tagged with `marker`. No picks: the baked gun as is. Returns (part root, id).
#[allow(clippy::too_many_arguments)]
pub(crate) fn mount_parts(
    cmd: &mut Commands,
    assets: &AssetServer,
    gun: Entity,
    row: &str,
    picks: &[String],
    joints: &Query<(Entity, &Name)>,
    parents: &Query<&ChildOf>,
    vis: &mut Query<&mut Visibility>,
    marker: impl Component + Clone,
) -> Vec<(Entity, String)> {
    let d = data();
    let parts = parts(row, picks);
    let placed = if picks.is_empty() { vec![] } else { place(row, &parts).0 };
    for node in d.rows.get(row).and_then(|g| d.baked.get(g)).into_iter().flatten() {
        let id = node.to_lowercase();
        let keep = picks.is_empty() || (parts.contains(&id) && !placed.iter().any(|p| p.id == id));
        if let Some(e) = joints.iter().find(|(e, n)| n.as_str() == node && root_of_entity(*e, gun, parents)).map(|j| j.0)
            && let Ok(mut v) = vis.get_mut(e)
        {
            *v = if keep { Visibility::Inherited } else { Visibility::Hidden };
        }
    }
    let mut out = vec![];
    for p in placed {
        let Some(bone) = joints.iter().find(|(e, n)| n.as_str().eq_ignore_ascii_case(&p.bone) && root_of_entity(*e, gun, parents)).map(|j| j.0) else { continue };
        let scene: Handle<bevy::world_serialization::WorldAsset> = assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/attachments/{}.glb", d.items[p.id.as_str()].glb)));
        let root = cmd.spawn((marker.clone(), Name::new(format!("att_{}", p.id)), WorldAssetRoot(scene), p.tf)).id();
        cmd.entity(bone).add_child(root);
        out.push((root, p.id));
    }
    out
}

/// Attachment parts on a third-person gun (remote players, the own world body): CharacterState.atts = that player's picks.
#[derive(Component, Clone)]
struct BodyAtt;
#[derive(Component)]
struct BodyAttKey(String);

#[allow(clippy::too_many_arguments)]
fn body_atts(
    mut cmd: Commands,
    assets: Res<AssetServer>,
    guns: Query<(Entity, &crate::character::CharGun, &ChildOf, Option<&BodyAttKey>)>,
    roots: Query<&ChildOf, With<crate::body::BodyRoot>>,
    chars: Query<&crate::character::CharacterState>,
    joints: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    children: Query<&Children>,
    old: Query<(), With<BodyAtt>>,
    mut vis: Query<&mut Visibility>,
) {
    for (gun, g, co, key) in &guns {
        let Some(st) = g.joint.and(roots.get(co.parent()).ok()).and_then(|r| chars.get(r.parent()).ok()) else { continue };
        // item just changed: sync_guns is despawning this gun this frame, its replacement gets the parts
        if crate::character::model_of(&st.item) != Some(g.model) {
            continue;
        }
        let k =format!("{}:{}", st.item, st.atts.join(","));
        if key.is_some_and(|x| x.0 == k) {
            continue;
        }
        for e in children.iter_descendants(gun).filter(|e| old.contains(*e)) {
            cmd.entity(e).despawn();
        }
        mount_parts(&mut cmd, &assets, gun, &st.item, &st.atts, &joints, &parents, &mut vis, BodyAtt);
        cmd.entity(gun).insert(BodyAttKey(k));
    }
}

/// Every part of every gun's loadout stays loaded (strong handles), so mounting after a tablet change or a swap is instant
/// instead of waiting on the glb load.
fn preload(cfg: Option<Res<Settings>>, assets: Res<AssetServer>, mut held: Local<HashMap<String, Handle<bevy::world_serialization::WorldAsset>>>) {
    let Some(cfg) = cfg.filter(|c| c.is_changed()) else { return };
    let d = data();
    for row in cfg.attachments.keys() {
        for id in parts(row, &picks(&cfg, row)) {
            if let Some(item) = d.items.get(id.as_str()) {
                held.entry(item.glb.clone()).or_insert_with(|| assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/attachments/{}.glb", item.glb))));
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
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
    mut vis: Query<&mut Visibility>,
    mut done: Local<(Option<Entity>, String)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut st: ResMut<AttState>,
) {
    let (Some(inv), Some(cfg)) = (inv, cfg) else { return };
    let Some((gun, g)) = guns.iter().next() else { return };
    if tablet.shown || g.0 == "Tablet" {
        return;
    }
    let row = inv.slots[inv.current].name.clone();
    // weapon just changed: attach_gun is replacing this gun scene, mount on the new one
    if crate::viewmodel::gun_model(&row, &inv.slots[inv.current].anims) != Some(g.0) {
        return;
    }
    let picks = picks(&cfg, &row);
    let key = format!("{row}:{}", picks.join(","));
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
    st.scope = None;
    let d = data();
    for (root, id) in mount_parts(&mut cmd, &assets, gun, &row, &picks, &joints, &parents, &mut vis, AttMount) {
        let item = &d.items[id.as_str()];
        if is_magnified(&id) && let Some(v) = item.sockets.get("viewpoint") {
            st.scope = Some((root, v.clone()));
        }
        // WEP optics: the reticle quad sits on the optic's Reticle socket (MI_ReticleData_*: ReticleColor, unlit).
        // ponytail: window size 7 cm x ReticleScaleAndOffset (guess).
        if let Some(rt) = item.reticle.as_ref().filter(|r| r.mag.is_none())
            && let Some(r) = item.sockets.get("reticle").or_else(|| item.sockets.get("reticle_canted"))
        {
            cmd.spawn((
                ReticleQuad,
                Mesh3d(meshes.add(Rectangle::new(0.07 * rt.scale[0], 0.07 * rt.scale[1]))),
                MeshMaterial3d(reticle_mat(&assets, &mut mats, rt)),
                r.tf() * Transform::from_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
                NoFrustumCulling,
                IgnoreTraces,
                RenderLayers::layer(VM_LAYER),
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
            // ponytail: beam colour by mesh (the MAWL is the green one); the BP's colour parameter is not in the data.
            let col = if item.glb.contains("mawl") { Color::srgb(0.1, 1.0, 0.2) } else { Color::srgb(1.0, 0.05, 0.05) };
            let m = mats.add(StandardMaterial { base_color: col, emissive: col.to_linear() * 8.0, unlit: true, alpha_mode: AlphaMode::Add, ..default() });
            cmd.spawn((LaserBeam, Mesh3d(meshes.add(Cuboid::new(0.0015, 1.0, 0.0015).mesh().build().translated_by(Vec3::new(0.0, -0.5, 0.0)))), MeshMaterial3d(m.clone()), Visibility::Hidden, l.tf(), NoFrustumCulling, IgnoreTraces, RenderLayers::layer(VM_LAYER), ChildOf(root)));
            cmd.spawn((AttMount, LaserDot, Mesh3d(meshes.add(Sphere::new(0.012))), MeshMaterial3d(m), Visibility::Hidden, Transform::default(), NoFrustumCulling, IgnoreTraces, RenderLayers::layer(VM_LAYER)));
        }
    }
}

/// Reticle quad material: the MI_ReticleData texture (white mask, alpha = luminance) tinted by ReticleColor x ReticleIlluminationIntensity, unlit.
fn reticle_mat(assets: &AssetServer, mats: &mut Assets<StandardMaterial>, r: &Reticle) -> Handle<StandardMaterial> {
    let tex = assets.load(format!("models/attachments/reticles/{}.png", r.tex));
    mats.add(StandardMaterial {
        base_color: Color::LinearRgba(LinearRgba::rgb(r.color[0], r.color[1], r.color[2]) * r.intensity.min(1.5)),
        base_color_texture: Some(tex),
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
fn switch_input(keys: Res<ButtonInput<KeyCode>>, mouse: Res<ButtonInput<MouseButton>>, tablet: Res<crate::ui::Tablet>, mut st: ResMut<AttState>, mut regrip: MessageWriter<Regrip>, mut snd: MessageWriter<crate::audio::PlayCue>, cfg: Option<Res<Settings>>, inv: Option<Single<&Inventory>>, player: Option<Single<&crate::player::Player>>) {
    if tablet.shown || player.is_some_and(|p| p.locked) { // piloting the drone: C is the drone's own light (perks.rs)
        return;
    }
    // Flashlight.OnRep_bIsFlashlightOn / Laser.OnRep_bIsLaserOn: S_WEP_*_Click_Cue on every toggle (only with the device mounted).
    let has = |f: &dyn Fn(&str) -> bool| match (&cfg, &inv) {
        (Some(c), Some(i)) => loadout(c, &i.slots[i.current].name).iter().any(|id| f(id)),
        _ => false,
    };
    let click = |snd: &mut MessageWriter<crate::audio::PlayCue>, k: &str| {
        snd.write(crate::audio::PlayCue { bools: vec![], cue: format!("/Game/BodycamWeapons/LowPolyShooterPack/Audio/WAV/Weapons/S_WEP_{k}_Click_Cue"), dist_cm: 0.0 });
    };
    if keys.just_pressed(KeyCode::KeyC) {
        st.flashlight = !st.flashlight;
        if has(&|id| data().items[id].sockets.contains_key("light")) {
            click(&mut snd, "Flashlight");
        }
    }
    if keys.just_pressed(KeyCode::KeyZ) {
        st.laser = !st.laser;
        if has(&|id| data().items[id].sockets.contains_key("socket_laser")) {
            click(&mut snd, "Laser");
        }
    }
    if mouse.just_pressed(MouseButton::Middle)
        && let (Some(cfg), Some(inv)) = (cfg, inv)
    {
        let row = &inv.slots[inv.current].name;
        if loadout(&cfg, row).iter().filter(|i| is_optic(row, i)).count() > 1 {
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
    let optic = active_optic(row, &ids, st.optic);
    let mut relief = 0.0;
    st.aim = optic.and_then(|id| {
        let placed = place(row, &ids).0;
        let p = placed.iter().find(|p| p.id == id)?;
        let a = data().items[id].sockets.get("aimpoint")?;
        // AdsSettings.AimSightDistance / AimMagnifiedSightDistance (cm): the eye sits that far behind the optic's AimPoint.
        let ads = &inv.slots[inv.current].anims["AdsSettings"];
        let mag = is_magnified(id);
        let d = ads[if mag { "AimMagnifiedSightDistance" } else { "AimSightDistance" }].as_f64().unwrap_or(if mag { 2.5 } else { 12.0 }) as f32;
        relief = d * 0.01;
        Some((p.bone.clone(), p.tf.transform_point(Vec3::from(a.loc)) + Vec3::Y * d * 0.01))
    });
    st.eye_relief = relief;
    // MI_ScopeOpticReticle_*.Magnification
    st.zoom = optic.and_then(|o| data().items[o].reticle.as_ref()?.mag).unwrap_or(1.0);
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

// ---- magnified optic: scope capture (BodycamScopeCaptureComponent at the optic's Viewpoint socket, SetMagnification) ----
const SCOPE_PX: u32 = 512;
// ACOG eyepiece glass (acog.glb MI_ScopeOpticGlass_AGO rear slot): radius 1.08 cm, 3.0 cm in front of the AimPoint socket.
// ponytail: scope-specific lens numbers; other magnified optics reuse them (only the ACOG is in the library). Magnification comes from the data.
const LENS_R: f32 = 0.0108;
const LENS_BEHIND: f32 = 0.030;

#[derive(Component)]
struct ScopeCam;
#[derive(Component)]
struct ScopeOverlay;

fn scope_setup(mut cmd: Commands, mut images: ResMut<Assets<Image>>, assets: Res<AssetServer>) {
    let img = images.add(Image::new_target_texture(SCOPE_PX, SCOPE_PX, TextureFormat::Rgba8UnormSrgb, None));
    cmd.spawn((
        ScopeCam,
        Camera3d::default(),
        Camera { order: -5, viewport: Some(Viewport { physical_size: UVec2::ONE, ..default() }), clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
        RenderTarget::Image(ImageRenderTarget { handle: img.clone(), scale_factor: 1.0 }),
        Projection::Perspective(PerspectiveProjection { near: 0.05, aspect_ratio: 1.0, ..default() }),
        bevy::core_pipeline::tonemapping::Tonemapping::AcesFitted,
        Transform::default(),
    ));
    let o = cmd
        .spawn((
            ScopeOverlay,
            Node { position_type: PositionType::Absolute, display: Display::None, border_radius: BorderRadius::MAX, overflow: Overflow::clip(), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() },
            ImageNode::new(img),
            GlobalZIndex(-10),
        ))
        .id();
    // MI_ReticleMagnifiedData_*: reticle texture scaled by ReticleScaleAndOffset over the lens.
    // every scope's default reticle is the gun's "Magnified Dot MK1 Red" item, so one overlay serves them all
    if let Some(r) = data().items.values().filter_map(|i| i.reticle.as_ref()).find(|r| r.mag.is_some()) {
        let c = Color::srgb(r.color[0], r.color[1], r.color[2]);
        cmd.spawn((
            Node { position_type: PositionType::Absolute, width: percent(r.scale[0] * 100.0), height: percent(r.scale[1] * 100.0), ..default() },
            ImageNode::new(assets.load(format!("models/attachments/reticles/{}.png", r.tex))).with_color(c),
            ChildOf(o),
        ));
    }
}

fn scope(
    st: Res<AttState>,
    ads: Res<crate::sway::Ads>,
    globals: Query<&GlobalTransform, Without<ScopeCam>>,
    main: Single<&Projection, (With<crate::player::BodycamCamera>, Without<ScopeCam>)>,
    win: Single<&Window, With<PrimaryWindow>>,
    mut cam: Single<(&mut Camera, &mut Transform, &mut GlobalTransform, &mut Projection), With<ScopeCam>>,
    mut overlay: Single<(&mut Node, &mut ImageNode), With<ScopeOverlay>>,
) {
    let on = st.zoom > 1.0 && ads.alpha > 0.05;
    let (cam_c, cam_t, cam_g, cam_p) = &mut *cam;
    let Some((root, vp)) = st.scope.as_ref().filter(|_| on) else {
        // ponytail: never deactivated (a camera switched on mid-run panics Bevy's directional-light cascades); parked on a 1 px viewport instead.
        cam_c.viewport = Some(Viewport { physical_size: UVec2::ONE, ..default() });
        overlay.0.display = Display::None;
        return;
    };
    let Ok(rg) = globals.get(*root) else { return };
    // camera looks along the optic's forward (joint -Y) with up +Z, like crate::viewmodel::ads_axes
    let local = Transform { translation: Vec3::from(vp.loc), rotation: Quat::from_array(vp.rot) * Quat::from_mat3(&Mat3::from_cols(Vec3::NEG_X, Vec3::Z, Vec3::Y)), ..default() };
    let world = rg.mul_transform(local);
    **cam_t = world.compute_transform();
    **cam_g = world;
    cam_c.viewport = None;
    // eye (camera point) to eyepiece distance: eye relief behind the AimPoint, which is LENS_BEHIND behind the glass
    let dist = LENS_BEHIND + st.eye_relief;
    let alpha = (LENS_R / dist).atan();
    if let Projection::Perspective(p) = &mut **cam_p {
        p.fov = 2.0 * (alpha.tan() / st.zoom).atan();
    }
    let vfov = if let Projection::Perspective(p) = *main { p.fov } else { 1.0 };
    let h = win.height();
    let focal = h * 0.5 / (vfov * 0.5).tan();
    let r = (focal * LENS_R / dist).min(h * 0.45);
    let (n, img) = &mut *overlay;
    n.display = Display::Flex;
    n.left = px(win.width() * 0.5 - r);
    n.top = px(h * 0.5 - r);
    n.width = px(2.0 * r);
    n.height = px(2.0 * r);
    img.color = Color::WHITE.with_alpha(((ads.alpha - 0.7) / 0.3).clamp(0.0, 1.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_mount_and_muzzle_device_extends_the_barrel() {
        let d = data();
        for row in ["M4A1", "AK47", "Glock17"] {
            let ids = &d.defaults[row];
            assert_eq!(place(row, ids).0.len(), ids.len(), "{row}: every default part finds its socket");
        }
        // A suppressor goes on the barrel's muzzle socket and its FireLocation sits further along the bore (joint -Y).
        let ids = d.defaults["M4A1"].clone();
        let sup = available("M4A1").iter().find(|i| d.items[i.as_str()].cls.starts_with("Muzzle.Suppressor") && d.items[i.as_str()].sockets.contains_key("firelocation")).expect("an M4 suppressor");
        let (_, bare) = place("M4A1", &ids);
        let (_, with) = place("M4A1", &[ids.clone(), vec![sup.clone()]].concat());
        assert!(with["firelocation"].1.translation.y < bare["muzzle"].1.translation.y - 0.05);
    }

    #[test]
    fn stat_applies_modifiers() {
        // a non-default M4 part with one modifier on an attribute nothing else in the build touches
        let found = available("M4A1").iter().filter(|i| !is_default("M4A1", i)).find_map(|id| {
            let fx = &data().items[id.as_str()].fx;
            let [(attr, op, v)] = fx.as_slice() else { return None };
            let mut s = Settings::default();
            toggle(&mut s, "M4A1", id);
            let alone = loadout(&s, "M4A1").iter().filter(|o| *o != id).all(|o| data().items[o.as_str()].fx.iter().all(|f| &f.0 != attr));
            alone.then(|| (s, attr.clone(), op.clone(), *v))
        });
        let (s, attr, op, v) = found.expect("an M4 part with a lone modifier");
        let want = match op.as_str() {
            "mul" => 2.0 * v,
            "add" => 2.0 + v,
            _ => v,
        };
        assert!((stat(&s, "M4A1", &attr, 2.0) - want).abs() < 1e-5);
    }
}
