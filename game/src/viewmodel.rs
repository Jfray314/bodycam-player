//! First-person viewmodel: Bodycam_Player's `Arms` (SKM_HumanArmsLongSleeves_Morphers, AnimClass Viewmodel_C)
//! with the equipped gun attached at `ik_hand_gun` (BP_EquippableItem.ItemAttachBone = VB ik_hand_gun, identity).
//! Models/clips come from tools/build_models.py (assets-local/models). The arms hang off the body-driven view
//! pose (`Player::view_base`) so the head's FP_Camera socket - where UE attaches the camera - sits on the camera,
//! while camera-only bob/shakes move the view but not the gun, as in UE.
//! Animation: base loop (Idle_Loop01 / Walk_Loop / AimIdlePose) plus the BAS montages from `MontageStarted`
//! and `Fired`, crossfaded; the gun plays the matching A_W_* clip.
use crate::player::Player;
use crate::weapon::{ActionKind, Fired, Inventory, MontageStarted};
use bevy::animation::AnimationTargetId;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use std::collections::HashMap;
use std::time::Duration;

/// FP_Camera socket in arms model space at idle: head bone (-0.008, 1.565, -0.061) + socket (10.47, 15.56, 0) cm
/// in head-bone axes (bone X = up, bone -Y = forward). Matches the live-measured camera mount within 5 cm.
const FP_CAMERA_SOCKET: Vec3 = Vec3::new(-0.008, 1.670, -0.217);
const BLEND: Duration = Duration::from_millis(150);

/// Stand-in for the FP_Action_Weight anim curve: UE pitches the camera by BAS ActionPitchAngle.x * SpineAdditiveActionWeight
/// (-50 * 0.6 = -30 deg) * FP_Action_Weight(t). Mag check needs the full -30 to keep the hands where idle has them;
/// reload measured in-game plateaus at -13 deg (camera_motion.json sequences.reload) -> ~0.43.
// ponytail: flat per-action weight, sample the real curve once curves.json exists.
fn action_weight(kind: ActionKind) -> f32 {
    match kind {
        ActionKind::MagCheck | ActionKind::Inspect => 1.0,
        ActionKind::Reload => 0.43,
        ActionKind::Holster | ActionKind::Draw => 0.0,
    }
}

pub struct ViewmodelPlugin;
impl Plugin for ViewmodelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load)
            .add_systems(Update, (spawn_arms, hook_players, no_cull, attach_gun, play_events, base_loop).chain())
            .add_systems(Last, debug_bones)
            .add_systems(
                PostUpdate,
                (place_arms, follow_head.after(bevy::app::AnimationSystems)).before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

#[derive(Resource)]
struct ViewmodelAssets {
    arms: Handle<Gltf>,
    guns: HashMap<&'static str, Handle<Gltf>>,
    /// Per-gun arm clips (models/arms_anims/<gun>.glb, same skeleton); arms.glb only carries M4A1/Glock.
    arm_anims: Vec<Handle<Gltf>>,
}

/// glb under models/guns for a weapon, from its BAS animation folder (e.g. Animations/M4a1/...).
fn gun_model(anims: &serde_json::Value) -> Option<&'static str> {
    let path = anims["Fire"][0]["Character"]["path"].as_str()?;
    let folder = path.split("/Animations/").nth(1)?.split('/').next()?.to_ascii_lowercase();
    Some(match folder.as_str() {
        "m4a1" => "M4A1",
        "mglock17" | "glock" => "Glock17",
        "akm" => "Akm",
        "deagle" => "Deagle",
        "fal" => "FAL",
        "mossberg590" => "Mossberg590",
        "remington700" => "Remington700",
        "revolver" => "Revolver",
        "scar-h" => "SCAR-H",
        "svd" => "SVD",
        "uzi" => "UZI",
        "mp9" => "MP9",
        "vector" => "Vector",
        "sg5-k" => "MP5",
        "benelli" => "Benelli",
        "crossbow" => "Crossbow",
        "vaiga" => "Vaiga",
        _ => return None,
    })
}

fn load(mut cmd: Commands, assets: Res<AssetServer>) {
    let names = ["M4A1", "Glock17", "Akm", "Deagle", "FAL", "Mossberg590", "Remington700", "Revolver", "SCAR-H", "SVD", "UZI", "MP9", "Vector", "MP5", "Benelli", "Crossbow", "Vaiga"];
    let guns = names.iter().map(|n| (*n, assets.load(format!("models/guns/{n}.glb")))).collect();
    let arm_anims = names[2..].iter().map(|n| assets.load(format!("models/arms_anims/{n}.glb"))).collect();
    cmd.insert_resource(ViewmodelAssets { arms: assets.load("models/arms.glb"), guns, arm_anims });
}

#[derive(Component)]
struct ArmsRoot;
#[derive(Component)]
struct GunRoot(&'static str);

/// Arms animation state: clip name -> graph node, and what's playing.
#[derive(Component)]
struct Anim {
    nodes: HashMap<String, AnimationNodeIndex>,
    current: Option<String>,
    /// A montage plays until this time (seconds since startup); afterwards the base loop resumes.
    montage_until: f32,
    /// Camera pitch (deg) the running montage asks for; 0 for fire.
    action_pitch: f32,
}

fn clip_graph<'a>(gs: impl IntoIterator<Item = &'a Gltf>, graphs: &mut Assets<AnimationGraph>) -> (Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>) {
    let mut graph = AnimationGraph::new();
    let nodes = gs
        .into_iter()
        .flat_map(|g| g.named_animations.iter())
        .map(|(n, c)| (n.to_string(), graph.add_clip(c.clone(), 1.0, graph.root)))
        .collect();
    (graphs.add(graph), nodes)
}

fn spawn_arms(
    mut cmd: Commands,
    vm: Res<ViewmodelAssets>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    assets: Res<AssetServer>,
    player: Single<Entity, With<Player>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(g) = gltfs.get(&vm.arms) else { return };
    // Wait for every per-gun clip file (a missing one just contributes no clips).
    if vm.arm_anims.iter().any(|h| !gltfs.contains(h) && !assets.get_load_state(h).is_some_and(|s| s.is_failed())) {
        return;
    }
    *done = true;
    let (graph, nodes) = clip_graph(std::iter::once(g).chain(vm.arm_anims.iter().filter_map(|h| gltfs.get(h))), &mut graphs);
    let arms = cmd
        .spawn((
            ArmsRoot,
            WorldAssetRoot(g.scenes[0].clone()),
            AnimationGraphHandle(graph),
            Anim { nodes, current: None, montage_until: 0.0, action_pitch: 0.0 },
            Transform::default(),
            crate::player::IgnoreTraces,
        ))
        .id();
    cmd.entity(*player).add_child(arms);
}

fn root_of<T: Component>(mut e: Entity, parents: &Query<&ChildOf>, roots: &Query<(), With<T>>) -> Option<Entity> {
    loop {
        if roots.contains(e) {
            return Some(e);
        }
        e = parents.get(e).ok()?.parent();
    }
}

/// glTF puts the AnimationPlayer on a node inside the scene; give it the graph + transitions of its root.
fn hook_players(
    mut cmd: Commands,
    new: Query<Entity, Added<AnimationPlayer>>,
    parents: Query<&ChildOf>,
    arms: Query<(), With<ArmsRoot>>,
    guns: Query<(), With<GunRoot>>,
    graphs: Query<&AnimationGraphHandle>,
) {
    for e in &new {
        let root = root_of(e, &parents, &arms).or_else(|| root_of(e, &parents, &guns));
        if let Some(r) = root
            && let Ok(g) = graphs.get(r)
        {
            cmd.entity(e).insert((g.clone(), AnimationTransitions::new(), AnimLink(r)));
        }
    }
}

/// The viewmodel is always in front of the camera; skinned-mesh bounds go stale under montages and got culled.
fn no_cull(
    mut cmd: Commands,
    new: Query<Entity, Added<Mesh3d>>,
    parents: Query<&ChildOf>,
    arms: Query<(), With<ArmsRoot>>,
    guns: Query<(), With<GunRoot>>,
) {
    for e in &new {
        if root_of(e, &parents, &arms).is_some() || root_of(e, &parents, &guns).is_some() {
            cmd.entity(e).insert((bevy::camera::visibility::NoFrustumCulling, crate::player::IgnoreTraces));
        }
    }
}

/// Links a glTF AnimationPlayer entity to its viewmodel root.
#[derive(Component)]
struct AnimLink(Entity);

/// Spawns the equipped gun under the arms' ik_hand_gun joint (identity), respawning on weapon change.
fn attach_gun(
    mut cmd: Commands,
    vm: Res<ViewmodelAssets>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    inv: Option<Single<&Inventory>>,
    joints: Query<(Entity, &Name, &AnimationTargetId)>,
    parents: Query<&ChildOf>,
    arms: Query<(), With<ArmsRoot>>,
    guns: Query<(Entity, &GunRoot)>,
) {
    let Some(inv) = inv else { return };
    let Some(want) = gun_model(&inv.slots[inv.current].anims) else { return };
    if guns.iter().any(|(_, g)| g.0 == want) {
        return;
    }
    let Some(hand) = joints.iter().find(|(e, n, _)| n.as_str() == "ik_hand_gun" && root_of(*e, &parents, &arms).is_some()).map(|j| j.0) else { return };
    let Some(g) = vm.guns.get(want).and_then(|h| gltfs.get(h)) else { return };
    for (e, _) in &guns {
        cmd.entity(e).despawn();
    }
    let (graph, nodes) = clip_graph([g], &mut graphs);
    let gun = cmd
        .spawn((GunRoot(want), WorldAssetRoot(g.scenes[0].clone()), AnimationGraphHandle(graph), Anim { nodes, current: None, montage_until: 0.0, action_pitch: 0.0 }, Transform::default()))
        .id();
    cmd.entity(hand).add_child(gun);
}

fn play(anim: &mut Anim, player: &mut AnimationPlayer, tr: &mut AnimationTransitions, clip: &str, repeat: bool) -> bool {
    let Some(&node) = anim.nodes.get(clip) else { return false };
    let a = tr.play(player, node, BLEND);
    if repeat {
        a.repeat();
    }
    anim.current = Some(clip.into());
    true
}

/// "/.../A_FP_M4A1_Draw_Montage.A_FP_M4A1_Draw_Montage" -> "A_FP_M4A1_Draw".
fn clip_of(montage_path: &str) -> String {
    montage_path.rsplit('.').next().unwrap_or("").trim_end_matches("_Montage").to_string()
}

fn play_events(
    time: Res<Time>,
    mut montages: MessageReader<MontageStarted>,
    mut fired: MessageReader<Fired>,
    inv: Option<Single<&Inventory>>,
    mut roots: Query<(&mut Anim, Has<ArmsRoot>)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions, &AnimLink)>,
) {
    let mut starts: Vec<(String, f32, f32)> = montages
        .read()
        .map(|m| {
            let a = inv.as_ref().map(|i| &i.slots[i.current].anims);
            let bas = |k: &str| a.and_then(|a| a[k].as_f64()).unwrap_or(0.0) as f32;
            let max = a.and_then(|a| a["ActionPitchAngle"]["x"].as_f64()).unwrap_or(0.0) as f32 * bas("SpineAdditiveActionWeight");
            (clip_of(&m.montage.path), m.montage.length, max * action_weight(m.kind))
        })
        .collect();
    if fired.read().count() > 0
        && let Some(inv) = &inv
        && let Some(path) = inv.slots[inv.current].anims["Fire"][0]["Character"]["path"].as_str()
    {
        starts.push((clip_of(path), 0.0, 0.0)); // fire is additive-ish: play, but let the base loop resume right away
    }
    let now = time.elapsed_secs();
    for (clip, len, pitch) in starts {
        for (mut player, mut tr, link) in &mut players {
            let Ok((mut anim, is_arms)) = roots.get_mut(link.0) else { continue };
            let name = if is_arms { clip.clone() } else { clip.replacen("A_FP_", "A_W_", 1) };
            if play(&mut anim, &mut player, &mut tr, &name, false) {
                anim.montage_until = now + if len > 0.0 { len } else { 0.25 };
                anim.action_pitch = pitch;
            }
        }
    }
}

/// Base loop when no montage is playing: aim idle, walk loop or idle loop of the current weapon.
fn base_loop(
    time: Res<Time>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    mut roots: Query<&mut Anim, With<ArmsRoot>>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions, &AnimLink)>,
) {
    let Some(inv) = inv else { return };
    let anims = &inv.slots[inv.current].anims;
    let Some(fire) = anims["Fire"][0]["Character"]["path"].as_str() else { return };
    let prefix = clip_of(fire).trim_end_matches("Fire").to_string(); // "A_FP_M4A1_"
    let aim = anims["AimIdlePose"]["path"].as_str().map(clip_of);
    // Walk_Loop is BAS CurveWalk: a curve source for the procedural walk motion, not a pose (played raw it snaps the
    // arms to the ref pose). ponytail: walking keeps the idle loop; drive procedural walk sway from its curves later.
    let want = match (p.aiming, aim) {
        (true, Some(a)) => a,
        _ => format!("{prefix}Idle_Loop01"),
    };
    let now = time.elapsed_secs();
    for (mut player, mut tr, link) in &mut players {
        let Ok(mut anim) = roots.get_mut(link.0) else { continue };
        if now < anim.montage_until || anim.current.as_deref() == Some(want.as_str()) {
            continue;
        }
        if !play(&mut anim, &mut player, &mut tr, &want, true) {
            play(&mut anim, &mut player, &mut tr, &format!("{prefix}Idle_Loop01"), true);
        }
    }
}

/// UE attaches the camera to the arms' FP_Camera socket, so montages that move the head (mag check looks down at
/// the gun, reloads tilt) move the view. Delta D = head_now * head_idle^-1 in arms space (the socket offset cancels);
/// applied only while a montage plays, because the measured walk/idle camera curves already contain the base loops.
fn follow_head(
    time: Res<Time>,
    p: Single<&Player>,
    arms: Single<(Entity, &Anim), With<ArmsRoot>>,
    joints: Query<(Entity, &Name)>,
    locals: Query<&Transform, Without<crate::player::BodycamCamera>>,
    parents: Query<&ChildOf>,
    mut cam: Single<&mut Transform, With<crate::player::BodycamCamera>>,
    mut head: Local<Option<Entity>>,
    mut idle: Local<Option<Transform>>,
    mut weight: Local<f32>,
) {
    let (root, anim) = *arms;
    if head.is_none() {
        *head = joints.iter().find(|(e, n)| n.as_str() == "head" && root_of_entity(*e, root, &parents)).map(|j| j.0);
    }
    let Some(h) = *head else { return };
    // Head transform in arms-root space: product of local transforms up the joint chain.
    let mut rel = *locals.get(h).unwrap();
    let mut e = h;
    while let Ok(c) = parents.get(e) {
        e = c.parent();
        if e == root {
            break;
        }
        rel = *locals.get(e).unwrap() * rel;
    }
    let montage = time.elapsed_secs() < anim.montage_until;
    if !montage && anim.current.is_some() {
        *idle = Some(rel); // track the base loop's head so the montage delta starts from where the base pose was
    }
    let Some(idle) = *idle else { return };
    *weight = crate::status::finterp_to(*weight, if montage { 1.0 } else { 0.0 }, time.delta_secs(), 1.0 / BLEND.as_secs_f32());
    let d = Transform::from_matrix(rel.to_matrix() * idle.to_matrix().inverse());
    let d = Transform { translation: d.translation * *weight, rotation: Quat::IDENTITY.slerp(d.rotation, *weight), ..default() };
    let to_socket = Transform::from_translation(FP_CAMERA_SOCKET);
    let residual = Transform::from_matrix(p.view_base.to_matrix().inverse() * cam.to_matrix());
    // Camera-only pitch-down during actions so the gun the montage brings to the chest stays in frame (arms stay on view_base).
    let action_pitch = Transform::from_rotation(Quat::from_rotation_x(anim.action_pitch.to_radians() * *weight));
    **cam = Transform::from_matrix(
        p.view_base.to_matrix()
            * to_socket.to_matrix().inverse()
            * d.to_matrix()
            * to_socket.to_matrix()
            * residual.to_matrix()
            * action_pitch.to_matrix(),
    );
}

fn root_of_entity(mut e: Entity, root: Entity, parents: &Query<&ChildOf>) -> bool {
    while let Ok(c) = parents.get(e) {
        e = c.parent();
        if e == root {
            return true;
        }
    }
    false
}

/// Arms root = body view pose * inverse(FP_Camera socket): the socket lands on the un-shaken camera point.
fn place_arms(p: Single<&Player>, mut arms: Query<&mut Transform, With<ArmsRoot>>) {
    for mut t in &mut arms {
        *t = p.view_base * Transform::from_translation(-FP_CAMERA_SOCKET);
    }
}

/// Dev: BODYCAM_DEBUG_BONES=1 logs key arm bones (arms-root space) twice a second.
pub fn debug_bones(
    time: Res<Time>,
    arms: Query<&GlobalTransform, With<ArmsRoot>>,
    cam: Single<&GlobalTransform, With<crate::player::BodycamCamera>>,
    joints: Query<(&Name, &GlobalTransform)>,
    morphs: Query<(Option<&Name>, &bevy::mesh::morph::MorphWeights)>,
    meshes: Query<(Option<&Name>, &ViewVisibility, &InheritedVisibility, &GlobalTransform), With<Mesh3d>>,
    mut last: Local<f32>,
) {
    if std::env::var("BODYCAM_DEBUG_BONES").is_err() || time.elapsed_secs() - *last < 0.5 {
        return;
    }
    *last = time.elapsed_secs();
    let Ok(r) = arms.single() else { return };
    info!("t={:.1} arms root world {:?}", *last, r.translation());
    let cam = cam.affine().inverse();
    for (n, gt) in joints.iter().filter(|(n, _)| ["ik_hand_gun", "hand_r", "hand_l"].contains(&n.as_str())) {
        let t = cam.transform_point3(gt.translation());
        // Camera looks down -Z: elevation of the bone relative to view forward (negative = below centre).
        info!("t={:.1} {} cam {:?} elev {:.1} deg", *last, n.as_str(), t, t.y.atan2(-t.z).to_degrees());
    }
    for (n, vv, iv, gt) in &meshes {
        info!("t={:.1} mesh {} view_visible={} inherited={} world={:?}", *last, n.map(|n| n.as_str()).unwrap_or("?"), vv.get(), iv.get(), gt.translation());
    }
    for (n, w) in &morphs {
        let mx = w.weights().iter().fold(0.0f32, |a, b| a.max(b.abs()));
        info!("t={:.1} morph {} n={} max|w|={}", *last, n.map(|n| n.as_str()).unwrap_or("?"), w.weights().len(), mx);
    }
}
