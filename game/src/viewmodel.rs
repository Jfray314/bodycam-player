//! First-person viewmodel: Bodycam_Player's `Arms` (SKM_HumanArmsLongSleeves_Morphers, AnimClass Viewmodel_C)
//! with the equipped gun attached at `ik_hand_gun` (BP_EquippableItem.ItemAttachBone = VB ik_hand_gun, identity).
//! Models/clips come from tools/build_models.py (assets-local/models). The arms hang off the body-driven view
//! pose (`Player::view_base`) so the head's FP_Camera socket - where UE attaches the camera - sits on the camera,
//! while camera-only bob/shakes move the view but not the gun, as in UE.
//! Animation: base loop (Idle_Loop01 / Walk_Loop / AimIdlePose) plus the BAS montages from `MontageStarted`
//! and `Fired`, crossfaded; the gun plays the matching A_W_* clip.
use crate::player::Player;
use crate::sway::{Ads, AimOffset, Sway, ue_vec};
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
        ActionKind::Holster | ActionKind::Draw | ActionKind::CameraAdjust => 0.0,
    }
}

pub struct ViewmodelPlugin;
impl Plugin for ViewmodelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load)
            .init_resource::<StressAnim>()
            .init_resource::<RegripState>()
            .add_systems(Update, (spawn_arms, hook_players, no_cull, attach_gun, play_events, base_loop, additive_weights, stress_layers, regrips).chain())
            .add_systems(Last, debug_bones)
            .add_systems(
                PostUpdate,
                (place_arms, (follow_head, left_elbow).after(bevy::app::AnimationSystems)).before(bevy::transform::TransformSystems::Propagate),
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

/// Receiver `AimPoint` socket (UE cm, gun Root bone space; assets-local/models/_work/guns/<gun>/<gun>.json). AnimNode_Ads (C++)
/// puts it on the FP_Camera socket when aimed: rec_1791925656, 6 guns, all within 0.1 cm (position only; the rotation stays the pose's).
/// Returns (bone, socket). A slide part with its own AimPoint wins: Glock17 = Slide socket (0, -6.53, -1.25) on `slide` +
/// Glock17_Default_Slide AimPoint (0, -25, 2.92).
// ponytail: receiver/slide sockets only; a mounted optic's AimPoint would override it (we mount no optics).
fn aim_point(gun: &str) -> Option<(&'static str, Vec3)> {
    if gun == "Glock17" {
        return Some(("slide", Vec3::new(0.0, -31.53, 1.67)));
    }
    Some(("root", match gun {
        "Akm" => Vec3::new(0.0, -5.32, 9.84),
        "Benelli" => Vec3::new(0.0, 2.27, 7.37),
        "Crossbow" => Vec3::new(0.0, -20.87, 10.98),
        "Deagle" => Vec3::new(0.0, -25.0, 6.5),
        "FAL" => Vec3::new(0.0, -9.0, 9.5),
        "M4A1" => Vec3::new(0.1, -13.19, 13.86), // receiver (0, -14.99, 13.06) lands 1.8 cm aft / 0.8 cm low of the recorded (0.1, -11.6, 15.0)
        "MP5" => Vec3::new(0.2, -16.14, 14.1),
        "MP9" => Vec3::new(0.2, -17.0, 8.5),
        "Mossberg590" => Vec3::new(0.1, -17.85, 1.5),
        "Remington700" => Vec3::new(0.0, -27.0, 5.8),
        "SCAR-H" => Vec3::new(0.0, -21.08, 13.14),
        "SVD" => Vec3::new(-0.3, 0.0, 12.07),
        "UZI" => Vec3::new(0.0, -19.48, 8.72),
        "Vaiga" => Vec3::new(0.0, -8.37, 9.12),
        "Vector" => Vec3::new(0.0, -11.0, 7.0),
        _ => return None, // Revolver: socket is rotated on a rotated Root, not mapped yet
    }))
}

fn load(mut cmd: Commands, assets: Res<AssetServer>) {
    let names = ["M4A1", "Glock17", "Akm", "Deagle", "FAL", "Mossberg590", "Remington700", "Revolver", "SCAR-H", "SVD", "UZI", "MP9", "Vector", "MP5", "Benelli", "Crossbow", "Vaiga"];
    // Only the default loadout is preloaded (guns are 40 MB glbs each: all 17 cost ~20 s); the rest load on first equip.
    let guns = names[..2].iter().map(|n| (*n, assets.load(format!("models/guns/{n}.glb")))).collect();
    let arm_anims = names[2..].iter().map(|n| assets.load(format!("models/arms_anims/{n}.glb"))).collect();
    cmd.insert_resource(ViewmodelAssets { arms: assets.load("models/arms.glb"), guns, arm_anims });
}

#[derive(Component)]
pub struct ArmsRoot;
#[derive(Component)]
pub struct GunRoot(&'static str);

/// Arms animation state: clip name -> graph node, and what's playing.
#[derive(Component, Default)]
struct Anim {
    nodes: HashMap<String, AnimationNodeIndex>,
    current: Option<String>,
    /// A montage plays until this time (seconds since startup); afterwards the base loop resumes.
    montage_until: f32,
    /// Camera pitch (deg) the running montage asks for; 0 for fire.
    action_pitch: f32,
    /// The running montage is an action (reload, draw, inspect...), not a fire kick: ActionPoseOffset applies.
    action: bool,
    /// Blend-out time of the last montage, used for the crossfade back to the base loop.
    blend_out: f32,
    after_montage: bool,
    /// Additive clips: (length, blend in, blend out) of the montage that started them.
    add_env: HashMap<AnimationNodeIndex, (f32, f32, f32)>,
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
            Anim { nodes, ..default() },
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
        // the gun is a descendant of the arms: check it first
        let root = root_of(e, &parents, &guns).or_else(|| root_of(e, &parents, &arms));
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
    mut vm: ResMut<ViewmodelAssets>,
    assets: Res<AssetServer>,
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
    let h = vm.guns.entry(want).or_insert_with(|| assets.load(format!("models/guns/{want}.glb")));
    let Some(g) = gltfs.get(&*h) else { return };
    for (e, _) in &guns {
        cmd.entity(e).despawn();
    }
    let (graph, nodes) = clip_graph([g], &mut graphs);
    let gun = cmd
        .spawn((GunRoot(want), WorldAssetRoot(g.scenes[0].clone()), AnimationGraphHandle(graph), Anim { nodes, ..default() }, Transform::default()))
        .id();
    cmd.entity(hand).add_child(gun);
}

fn play(anim: &mut Anim, player: &mut AnimationPlayer, tr: &mut AnimationTransitions, clip: &str, repeat: bool, blend: Duration) -> bool {
    let Some(&node) = anim.nodes.get(clip) else { return false };
    if is_additive(clip) {
        player.start(node); // layered on top of whatever plays; restarts on every shot
        return true;
    }
    let a = tr.play(player, node, blend);
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

/// One montage to start on the arms and/or the gun.
struct Start {
    arms: String,
    /// Gun-side clip from the BAS `Weapon` key.
    gun: Option<String>,
    len: f32,
    pitch: f32,
    on_arms: bool,
    action: bool,
    blend_in: f32,
    blend_out: f32,
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
    let mut starts: Vec<Start> = montages
        .read()
        .map(|m| {
            let a = inv.as_ref().map(|i| &i.slots[i.current].anims);
            let bas = |k: &str| a.and_then(|a| a[k].as_f64()).unwrap_or(0.0) as f32;
            let max = a.and_then(|a| a["ActionPitchAngle"]["x"].as_f64()).unwrap_or(0.0) as f32 * bas("SpineAdditiveActionWeight");
            Start {
                arms: clip_of(&m.montage.path),
                gun: m.montage.weapon.as_deref().map(clip_of),
                len: m.montage.length,
                pitch: max * action_weight(m.kind),
                on_arms: true,
                action: true,
                blend_in: m.montage.blend_in,
                blend_out: m.montage.blend_out,
            }
        })
        .collect();
    if fired.read().count() > 0
        && let Some(inv) = &inv
        && let fire = &inv.slots[inv.current].anims["Fire"][0]
        && let Some(path) = fire["Character"]["path"].as_str()
    {
        // UE layers fire additively over hip or aim. Additive fire clips layer here too; the few full-pose ones
        // (M4A1) would drop the aim, so while aiming only the gun plays those and the kick comes from the camera.
        let clip = clip_of(path);
        let on_arms = !p.aiming || is_additive(&clip);
        let b = |k: &str, d: f64| fire["Character"][k].as_f64().unwrap_or(d) as f32;
        starts.push(Start {
            on_arms,
            gun: fire["Weapon"]["path"].as_str().map(clip_of),
            len: b("length", 0.0),
            blend_in: b("blend_in", 0.25),
            blend_out: b("blend_out", 0.25),
            arms: clip,
            pitch: 0.0,
            action: false,
        });
    }
    let now = time.elapsed_secs();
    for s in starts {
        for (mut player, mut tr, link) in &mut players {
            let Ok((mut anim, is_arms)) = roots.get_mut(link.0) else { continue };
            if is_arms && !s.on_arms {
                continue;
            }
            let Some(name) = (if is_arms { Some(s.arms.clone()) } else { s.gun.clone() }) else { continue };
            if !play(&mut anim, &mut player, &mut tr, &name, false, Duration::from_secs_f32(s.blend_in)) {
                continue;
            }
            if is_additive(&name) {
                let node = anim.nodes[&name];
                anim.add_env.insert(node, (s.len, s.blend_in, s.blend_out));
            } else {
                // The montage blends out over its last BlendOut seconds, so the base loop takes over that early.
                anim.montage_until = now + if s.len > 0.0 { (s.len - s.blend_out).max(0.05) } else { 0.25 };
                anim.action_pitch = s.pitch;
                anim.action = s.action;
                anim.blend_out = s.blend_out;
                anim.after_montage = true;
            }
        }
    }
}

/// Additive clips: montage BlendIn/BlendOut envelope, and AdsAdditiveBlend while aiming (arms only).
fn additive_weights(ads: Res<Ads>, inv: Option<Single<&Inventory>>, roots: Query<(&Anim, Has<ArmsRoot>)>, mut players: Query<(&mut AnimationPlayer, &AnimLink)>) {
    let ads_blend = inv.map(|i| i.slots[i.current].anims["AdsSettings"]["AdsAdditiveBlend"].as_f64().unwrap_or(1.0) as f32).unwrap_or(1.0);
    for (mut player, link) in &mut players {
        let Ok((anim, is_arms)) = roots.get(link.0) else { continue };
        let ads_w = if is_arms { 1.0 + (ads_blend - 1.0) * ads.alpha } else { 1.0 };
        for (&node, &(len, bin, bout)) in &anim.add_env {
            let Some(a) = player.animation_mut(node) else { continue };
            let t = a.seek_time();
            let env = if len > 0.0 { (t / bin.max(1e-3)).min(1.0) * ((len - t) / bout.max(1e-3)).clamp(0.0, 1.0) } else { 1.0 };
            a.set_weight(env * ads_w);
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
        StressAnim { subtle: 0.0, strong: 0.0 }
    }
}

/// AC_BodycamAnimationComponent "Stress Tick" (matches the recordings: tools/probe_sway.py, subtle rms 0.013-0.018 of sd 0.26-0.34).
/// `speed_cm` is VSize(GetVelocity), vertical part included.
pub fn stress_tick(subtle: f32, speed_cm: f32, aiming: bool, stress: f32, dt: f32) -> (f32, f32) {
    (crate::status::stress_subtle_step(subtle, dt, speed_cm, aiming), crate::status::stress_strong(stress, aiming))
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
    (alpha.subtle, alpha.strong) = stress_tick(alpha.subtle, p.vel.length() * 100.0, p.aiming, st.stress, time.delta_secs());
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
            play(&mut anim, &mut player, &mut tr, c, false, BLEND);
        }
        if shaking && play(&mut anim, &mut player, &mut tr, "A_FP_shakingHand", false, BLEND) {
            anim.montage_until = now + 2.0; // ponytail: montage length unknown until the clip is exported
        }
    }
}

/// Base loop when no montage is playing: aim idle, walk loop or idle loop of the current weapon.
fn base_loop(
    time: Res<Time>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    ads: Res<Ads>,
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
    let want = match (p.aiming, aim.clone(), walk) {
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
        // Montage exit uses its BlendOut; hip<->aim uses the weapon's AdsSpeed; everything else the default.
        let dur = if std::mem::take(&mut anim.after_montage) {
            anim.blend_out
        } else if Some(&want) == aim.as_ref() || anim.current.as_ref() == aim.as_ref() {
            ads.duration
        } else {
            BLEND.as_secs_f32()
        };
        let dur = Duration::from_secs_f32(dur);
        if !play(&mut anim, &mut player, &mut tr, &want, true, dur) {
            play(&mut anim, &mut player, &mut tr, &format!("{prefix}Idle_Loop01"), true, dur);
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

/// BAS ViewmodelOffsets.<key> (PoseOffset / ActionPoseOffset; UE cm, mesh component space) as a Bevy camera-space transform
/// pivoting on the grip. The FP mesh faces +Y (yaw -90): component (x, y, z) = camera (fwd y, right -x, up z). Measured:
/// M4A1 hip gun (rec_1791908511) sits 10.2 cm forward, matching only with the 8 cm X offset to the left (as forward it was 18).
fn pose_offset(anims: &serde_json::Value, key: &str) -> Transform {
    let o = &anims["ViewmodelOffsets"][key];
    let f = |v: &serde_json::Value, k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let (t, r) = (&o["Translation"], &o["Rotation"]);
    // component -> UE camera (y, -x, z), then UE (X fwd, Y right, Z up, left-handed) -> Bevy (-Z fwd, X right, Y up):
    // p = (Y, Z, -X) = (-x, z, -y), q = (-Y, -Z, X, w) = (x, -z, y, w).
    let tr = Vec3::new(-f(t, "x"), f(t, "z"), -f(t, "y")) * 0.01;
    let rot = if r.is_object() { Quat::from_xyzw(f(r, "x"), -f(r, "z"), f(r, "y"), f(r, "w")).normalize() } else { Quat::IDENTITY };
    Transform::from_translation(tr + GRIP) * Transform::from_rotation(rot) * Transform::from_translation(-GRIP)
}

/// Arms root = body view pose * recoil * spring sway * aim-in/out additive * (PoseOffset -> ActionPoseOffset during
/// actions, faded out by the ADS alpha) * inverse(FP_Camera socket): the socket lands on the un-shaken camera point.
fn place_arms(
    time: Res<Time>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    recoil: Res<crate::recoil::Recoil>,
    ads: Res<Ads>,
    mut sway: ResMut<Sway>,
    aim: Res<AimOffset>,
    mut arms: Query<(Entity, &mut Transform, &Anim), With<ArmsRoot>>,
    joints: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    mut gun_joint: Local<Option<Entity>>,
    mut action: Local<f32>,
    guns: Query<(Entity, &GunRoot)>,
    mut gun_bone: Local<Option<Entity>>,
) {
    let gun = guns.iter().next();
    let aim_sock = gun.and_then(|(_, g)| aim_point(g.0));
    if gun_bone.is_none_or(|b| globals.get(b).is_err() || !gun.is_some_and(|(g, _)| root_of_entity(b, g, &parents))) {
        *gun_bone = gun.zip(aim_sock).and_then(|((g, _), (bone, _))| joints.iter().find(|(e, n)| n.as_str().eq_ignore_ascii_case(bone) && root_of_entity(*e, g, &parents)).map(|j| j.0));
    }
    // UE socket (x, y, z) cm -> joint frame (x, -y, z) m: the PSK import mirrors Y (as ueblender.attach_parts); verified by the sights centring.
    let gun_aim = aim_sock.map(|(_, v)| Vec3::new(v.x, -v.y, v.z) * 0.01);
    let (pose, act) =inv.map(|i| (pose_offset(&i.slots[i.current].anims, "PoseOffset"), pose_offset(&i.slots[i.current].anims, "ActionPoseOffset"))).unwrap_or_default();
    let hip = 1.0 - ads.alpha;
    let now = time.elapsed_secs();
    for (root, mut t, anim) in &mut arms {
        // ponytail: ActionPoseOffset blend speed (8/s) is a guess; the real blend lives in the compiled ABP.
        *action = crate::status::finterp_to(*action, if now < anim.montage_until && anim.action { 1.0 } else { 0.0 }, time.delta_secs(), 8.0);
        let off = Transform { translation: pose.translation.lerp(act.translation, *action), rotation: pose.rotation.slerp(act.rotation, *action), ..default() };
        let off = Transform { translation: off.translation * hip, rotation: Quat::IDENTITY.slerp(off.rotation, hip), ..default() };
        let aim = Transform::from_translation(GRIP) * aim.0 * Transform::from_translation(-GRIP);
        let post = aim * off * Transform::from_translation(-FP_CAMERA_SOCKET);
        // Sway pivots on the live ik_hand_gun (last frame's pose, arms-root space -> the frame sway.offset applies in).
        if gun_joint.is_none_or(|j| globals.get(j).is_err()) {
            *gun_joint = joints.iter().find(|(e, n)| n.as_str() == "ik_hand_gun" && root_of_entity(*e, root, &parents)).map(|j| j.0);
        }
        // AnimNode_Ads: shift the arms so the gun's AimPoint (last frame's pose, arms-root space) lands on the camera point.
        let mut post = post;
        if let (Some(ap), Some(gt), Ok(rg)) = (gun_aim, gun_bone.and_then(|b| globals.get(b).ok()), globals.get(root)) {
            let p = post.transform_point(rg.affine().inverse().transform_point3(gt.transform_point(ap)));
            post = Transform::from_translation(-p * ads.alpha) * post;
        }
        if let (Some(j), Ok(rg)) = (*gun_joint, globals.get(root))
            && let Ok(gg) = globals.get(j)
        {
            sway.pivot = Some(post.transform_point(rg.affine().inverse().transform_point3(gg.translation())));
        }
        *t = p.view_base * recoil.offset * sway.offset * post;
    }
}

/// BAS ViewmodelOffsets.AimingLeftElbowOffset (M4 (0,0,7.5) cm up, VInterp 5): UE moves the left elbow with an IK pole
/// target. Without IK the upper arm is twisted about the shoulder->hand axis, which moves the elbow and keeps the
/// hand in place; the hand is counter-rotated so the grip orientation stays put.
// ponytail: single-bone twist standing in for the IK pole target; elbow lands on the offset only to first order.
fn left_elbow(
    time: Res<Time>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    arms: Single<(Entity, &GlobalTransform), With<ArmsRoot>>,
    joints: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    globals: Query<&GlobalTransform>,
    mut locals: Query<&mut Transform>,
    mut ids: Local<Option<[Entity; 3]>>,
    mut w: Local<f32>,
    mut prev: Local<f32>,
) {
    let Some(inv) = inv else { return };
    let (root, root_gt) = *arms;
    if ids.is_none() {
        let find = |n: &str| joints.iter().find(|(e, j)| j.as_str() == n && root_of_entity(*e, root, &parents)).map(|j| j.0);
        *ids = (|| Some([find("upperarm_l")?, find("lowerarm_l")?, find("hand_l")?]))();
    }
    let Some([up, low, hand]) = *ids else { return };
    let v = &inv.slots[inv.current].anims["ViewmodelOffsets"]["AimingLeftElbowOffset"];
    let d = Vec3::new(v["x"].as_f64().unwrap_or(0.0) as f32, v["y"].as_f64().unwrap_or(0.0) as f32, v["z"].as_f64().unwrap_or(0.0) as f32);
    *w = crate::status::finterp_to(*w, if p.aiming { 1.0 } else { 0.0 }, time.delta_secs(), 5.0);
    if d == Vec3::ZERO {
        return;
    }
    let (Ok(gu), Ok(gl), Ok(gh)) = (globals.get(up), globals.get(low), globals.get(hand)) else { return };
    let (s, e, h) = (gu.translation(), gl.translation(), gh.translation());
    let a = (h - s).normalize_or_zero();
    // Globals are last frame's, i.e. still twisted by `prev`; undo it to measure the animated pose.
    let undo = Quat::from_axis_angle(a, -*prev);
    let r = undo * ((e - s) - a * (e - s).dot(a));
    if a == Vec3::ZERO || r.length() < 1e-4 {
        return;
    }
    let tang = a.cross(r).normalize();
    let dw = root_gt.rotation() * ue_vec(d) * *w;
    let theta = (tang.dot(dw) / r.length()).clamp(-0.6, 0.6);
    let q = Quat::from_axis_angle(a, theta);
    *prev = theta;
    let Ok(up_parent) = parents.get(up) else { return };
    let Ok(pg) = globals.get(up_parent.parent()) else { return };
    if let Ok(mut t) = locals.get_mut(up) {
        t.rotation = pg.rotation().inverse() * q * pg.rotation() * t.rotation;
    }
    if let Ok(hp) = parents.get(hand)
        && let Ok(hpg) = globals.get(hp.parent())
        && let Ok(mut t) = locals.get_mut(hand)
    {
        // new local = parent_old^-1 * q^-1 * parent_old * local (parent rotated by q, world hand rotation kept)
        let po = undo * hpg.rotation();
        t.rotation = po.inverse() * q.inverse() * po * t.rotation;
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

#[cfg(test)]
mod tests {
    #[test]
    fn stress_tick_matches_recording() {
        let r: serde_json::Value = serde_json::from_str(include_str!("../../assets/sway_reference.json")).unwrap();
        let s = &r["stress"]["series_60hz"];
        let col = |k: &str| -> Vec<f32> { s[k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect() };
        let (speed, aim, stress, sub, strong) = (col("speed_m_s"), col("aiming"), col("stress"), col("subtle"), col("strong"));
        let (mut a, mut es, mut eg, mut ss) = (sub[0], 0.0f32, 0.0f32, 0.0f32);
        for i in 1..sub.len() {
            let (na, g) = super::stress_tick(a, speed[i] * 100.0, aim[i] > 0.5, stress[i], 1.0 / 60.0);
            a = na;
            es += (a - sub[i]).powi(2);
            eg += (g - strong[i]).powi(2);
            ss += (sub[i] - 0.4).powi(2);
        }
        let n = (sub.len() - 1) as f32;
        eprintln!("stress alpha rms: subtle {:.4}, strong {:.4} (subtle signal {:.3})", (es / n).sqrt(), (eg / n).sqrt(), (ss / n).sqrt());
        assert!((es / n).sqrt() < 0.05 && (eg / n).sqrt() < 0.1);
    }
}
