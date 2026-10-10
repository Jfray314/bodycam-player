//! First-person viewmodel: Bodycam_Player's `Arms` (SKM_HumanArmsLongSleeves_Morphers, AnimClass Viewmodel_C)
//! with the equipped gun attached at `ik_hand_gun` (BP_EquippableItem.ItemAttachBone = VB ik_hand_gun, identity).
//! Models/clips come from tools/build_models.py (assets-local/models). The arms hang off the body-driven view
//! pose (`Player::view_base`) so the head's FP_Camera socket - where UE attaches the camera - sits on the camera,
//! while camera-only bob/shakes move the view but not the gun, as in UE.
//! Animation: base loop (Idle_Loop01 / Walk_Loop / AimIdlePose) plus the BAS montages from `MontageStarted`
//! and `Fired`, crossfaded; the gun plays the matching A_W_* clip.
use crate::player::Player;
use crate::weapon::{ActionKind, Fired, HandShake, Inventory, MontageStarted, Regrip};
use rand::Rng;
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
            .init_resource::<StressAnim>()
            .init_resource::<RegripState>()
            .add_systems(Update, (spawn_arms, hook_players, no_cull, attach_gun, play_events, base_loop, stress_layers, regrips).chain())
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
pub struct GunRoot(&'static str);

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

/// UE additive clips (local-space deltas: fire, regrips), from tools/additive_clips.py.
fn is_additive(clip: &str) -> bool {
    static SET: std::sync::OnceLock<std::collections::HashSet<String>> = std::sync::OnceLock::new();
    SET.get_or_init(|| serde_json::from_str(include_str!("../../assets/additive_clips.json")).expect("additive_clips.json")).contains(clip)
}

/// root -> additive blend [ base blend (full-pose clips, crossfaded by AnimationTransitions), additive clips... ].
fn clip_graph<'a>(gs: impl IntoIterator<Item = &'a Gltf>, graphs: &mut Assets<AnimationGraph>) -> (Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>) {
    let mut graph = AnimationGraph::new();
    let add = graph.add_additive_blend(1.0, graph.root);
    let base = graph.add_blend(1.0, add);
    let nodes = gs
        .into_iter()
        .flat_map(|g| g.named_animations.iter())
        .map(|(n, c)| (n.to_string(), graph.add_clip(c.clone(), 1.0, if is_additive(n) { add } else { base })))
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
    if is_additive(clip) {
        player.start(node); // layered on top of whatever plays; restarts on every shot
        return true;
    }
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
    p: Single<&Player>,
    mut roots: Query<(&mut Anim, Has<ArmsRoot>)>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions, &AnimLink)>,
) {
    // (clip, length, action pitch, also on the arms)
    let mut starts: Vec<(String, f32, f32, bool)> = montages
        .read()
        .map(|m| {
            let a = inv.as_ref().map(|i| &i.slots[i.current].anims);
            let bas = |k: &str| a.and_then(|a| a[k].as_f64()).unwrap_or(0.0) as f32;
            let max = a.and_then(|a| a["ActionPitchAngle"]["x"].as_f64()).unwrap_or(0.0) as f32 * bas("SpineAdditiveActionWeight");
            (clip_of(&m.montage.path), m.montage.length, max * action_weight(m.kind), true)
        })
        .collect();
    if fired.read().count() > 0
        && let Some(inv) = &inv
        && let Some(path) = inv.slots[inv.current].anims["Fire"][0]["Character"]["path"].as_str()
    {
        // UE layers fire additively over hip or aim. Additive fire clips layer here too; the few full-pose ones
        // (M4A1) would drop the aim, so while aiming only the gun plays those and the kick comes from the camera.
        let clip = clip_of(path);
        let on_arms = !p.aiming || is_additive(&clip);
        starts.push((clip, 0.0, 0.0, on_arms));
    }
    let now = time.elapsed_secs();
    for (clip, len, pitch, on_arms) in starts {
        for (mut player, mut tr, link) in &mut players {
            let Ok((mut anim, is_arms)) = roots.get_mut(link.0) else { continue };
            if is_arms && !on_arms {
                continue;
            }
            let name = if is_arms { clip.clone() } else { clip.replacen("A_FP_", "A_W_", 1) };
            if play(&mut anim, &mut player, &mut tr, &name, false) && !is_additive(&name) {
                anim.montage_until = now + if len > 0.0 { len } else { 0.25 };
                anim.action_pitch = pitch;
            }
        }
    }
}

/// AC_BodycamAnimationComponent.StressedSubtleIdleAlpha / StressedStrongIdleAlpha (Stress Tick). Subtle starts at 2.0 (Bodycam_Player default).
#[derive(Resource)]
pub struct StressAnim {
    pub subtle: f32,
    pub strong: f32,
}
impl Default for StressAnim {
    fn default() -> Self {
        StressAnim { subtle: 2.0, strong: 0.0 }
    }
}

/// AC "Stress Tick" + the two BAS stress idle clips (StressedIdleSubtle loop, StressedIdleStrong) layered additively on the
/// arms at those alphas. The clips are full poses in UE; tools/stress_additive.py bakes them into deltas vs their first frame.
// ponytail: A_FP_AK_Shake_Loop isn't in the exported glbs yet (tools/ue-export now exports it on the next full rebuild);
// until then guns that use it fall back to A_FP_Glock17_Shake_Loop.
fn stress_layers(
    time: Res<Time>,
    st: Res<crate::status::SurvivorStatus>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    mut alpha: ResMut<StressAnim>,
    roots: Query<&Anim, With<ArmsRoot>>,
    mut players: Query<(&mut AnimationPlayer, &AnimLink)>,
) {
    let Some(inv) = inv else { return };
    let mr = crate::status::map_range_clamped;
    let speed_cm = Vec2::new(p.vel.x, p.vel.z).length() * 100.0;
    let sub_target = if p.aiming { 0.0 } else { mr(speed_cm, 0.0, 100.0, 0.1, 1.0) };
    alpha.subtle = crate::status::finterp_to(alpha.subtle, sub_target, time.delta_secs(), 10.0);
    alpha.strong = if p.aiming { mr(st.stress, 0.5, 1.0, 0.0, 1.0) } else { mr(st.stress, 0.0, 1.0, 0.0, 4.0) };
    let a = &inv.slots[inv.current].anims;
    let clip = |k: &str| a[k]["path"].as_str().map(clip_of);
    for (mut player, link) in &mut players {
        let Ok(anim) = roots.get(link.0) else { continue };
        for (name, w) in [(clip("StressedIdleSubtle"), alpha.subtle), (clip("StressedIdleStrong"), alpha.strong)] {
            let Some(node) = name.as_ref().and_then(|n| anim.nodes.get(n).or_else(|| anim.nodes.get("A_FP_Glock17_Shake_Loop").filter(|_| n.ends_with("Shake_Loop")))) else { continue };
            if !player.is_playing_animation(*node) {
                player.start(*node).repeat();
            }
            if let Some(act) = player.animation_mut(*node) {
                act.set_weight(w);
            }
        }
    }
}

/// Pending end-fire / aim-end regrip (RetriggerableDelay) and the idle regrip loop timer.
#[derive(Resource, Default)]
struct RegripState {
    pending: Option<f32>,
    idle_in: f32,
}

/// Plays a random BAS ReGripMotions clip (additive) after the delay; AC idle loop re-arms every (len..4)+(5..8) s;
/// HandShake plays A_FP_shakingHand_Montage when its clip exists in the glbs.
fn regrips(
    time: Res<Time>,
    mut rg: ResMut<RegripState>,
    mut req: MessageReader<Regrip>,
    mut shake: MessageReader<HandShake>,
    inv: Option<Single<&Inventory>>,
    mut roots: Query<&mut Anim, With<ArmsRoot>>,
    mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions, &AnimLink)>,
) {
    let Some(inv) = inv else { return };
    let dt = time.delta_secs();
    for r in req.read() {
        rg.pending = Some(r.delay);
    }
    rg.idle_in -= dt;
    if rg.idle_in <= 0.0 {
        let mut rng = rand::rng();
        rg.idle_in = rng.random_range(1.0..4.0) + rng.random_range(5.0..8.0);
        if !inv.busy() {
            rg.pending.get_or_insert(0.0);
        }
    }
    let shaking = shake.read().count() > 0;
    let mut clip = None;
    if let Some(t) = &mut rg.pending {
        *t -= dt;
        if *t <= 0.0 {
            rg.pending = None;
            let list = inv.slots[inv.current].anims["ReGripMotions"].as_array();
            clip = list.filter(|l| !l.is_empty()).and_then(|l| l[rand::rng().random_range(0..l.len())]["path"].as_str().map(clip_of));
        }
    }
    let now = time.elapsed_secs();
    for (mut player, mut tr, link) in &mut players {
        let Ok(mut anim) = roots.get_mut(link.0) else { continue };
        if let Some(c) = &clip {
            play(&mut anim, &mut player, &mut tr, c, false);
        }
        if shaking && play(&mut anim, &mut player, &mut tr, "A_FP_shakingHand", false) {
            anim.montage_until = now + 2.0; // ponytail: montage length unknown until the clip is exported
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
    let moving = p.grounded && Vec2::new(p.vel.x, p.vel.z).length() > 0.1;
    let walk = anims["CurveWalk"]["path"].as_str().map(clip_of);
    let want = match (p.aiming, aim, walk) {
        (true, Some(a), _) => a,
        (false, _, Some(w)) if moving => w,
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

/// Grip point (ik_hand_gun at hip idle, camera space) that BAS PoseOffset rotates about.
const GRIP: Vec3 = Vec3::new(0.09, -0.22, -0.13);

/// BAS ViewmodelOffsets.PoseOffset (UE cm, camera space) as a Bevy camera-space transform pivoting on the grip.
fn pose_offset(anims: &serde_json::Value) -> Transform {
    let o = &anims["ViewmodelOffsets"]["PoseOffset"];
    let f = |v: &serde_json::Value, k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let (t, r) = (&o["Translation"], &o["Rotation"]);
    // UE (X fwd, Y right, Z up, left-handed) -> Bevy (-Z fwd, X right, Y up): p = (y, z, -x), q = (-y, -z, x, w).
    let tr = Vec3::new(f(t, "y"), f(t, "z"), -f(t, "x")) * 0.01;
    let rot = if r.is_object() { Quat::from_xyzw(-f(r, "y"), -f(r, "z"), f(r, "x"), f(r, "w")).normalize() } else { Quat::IDENTITY };
    Transform::from_translation(tr + GRIP) * Transform::from_rotation(rot) * Transform::from_translation(-GRIP)
}

/// Arms root = body view pose * hip PoseOffset * inverse(FP_Camera socket): the socket lands on the un-shaken camera
/// point; the offset fades out while aiming (AimIdlePose already lines the sights up).
fn place_arms(
    time: Res<Time>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    recoil: Res<crate::recoil::Recoil>,
    mut arms: Query<&mut Transform, With<ArmsRoot>>,
    mut hip: Local<f32>,
) {
    *hip = crate::status::finterp_to(*hip, if p.aiming { 0.0 } else { 1.0 }, time.delta_secs(), 1.0 / BLEND.as_secs_f32());
    let off = inv.map(|i| pose_offset(&i.slots[i.current].anims)).unwrap_or_default();
    let off = Transform { translation: off.translation * *hip, rotation: Quat::IDENTITY.slerp(off.rotation, *hip), ..default() };
    for mut t in &mut arms {
        *t = p.view_base * recoil.offset * off * Transform::from_translation(-FP_CAMERA_SOCKET);
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
    if std::env::var("BODYCAM_DEBUG_BONES").is_err() || time.elapsed_secs() - *last < 0.1 {
        return;
    }
    *last = time.elapsed_secs();
    let Ok(r) = arms.single() else { return };
    info!("t={:.1} arms root world {:?}", *last, r.translation());
    let (y, x, z) = cam.rotation().to_euler(EulerRot::YXZ);
    info!("t={:.1} cam yaw {:.1} pitch {:.1} roll {:.1}", *last, y.to_degrees(), x.to_degrees(), z.to_degrees());
    let cam = cam.affine().inverse();
    for (n, gt) in joints.iter().filter(|(n, _)| ["ik_hand_gun", "hand_r", "hand_l"].contains(&n.as_str())) {
        let t = cam.transform_point3(gt.translation());
        let r = cam.to_scale_rotation_translation().1 * gt.rotation();
        // Camera looks down -Z: elevation of the bone relative to view forward (negative = below centre).
        info!("t={:.1} {} cam {:?} elev {:.1} deg rot {:?}", *last, n.as_str(), t, t.y.atan2(-t.z).to_degrees(), r.to_euler(EulerRot::YXZ));
    }
    for (n, vv, iv, gt) in &meshes {
        info!("t={:.1} mesh {} view_visible={} inherited={} world={:?}", *last, n.map(|n| n.as_str()).unwrap_or("?"), vv.get(), iv.get(), gt.translation());
    }
    for (n, w) in &morphs {
        let mx = w.weights().iter().fold(0.0f32, |a, b| a.max(b.abs()));
        info!("t={:.1} morph {} n={} max|w|={}", *last, n.map(|n| n.as_str()).unwrap_or("?"), w.weights().len(), mx);
    }
}
