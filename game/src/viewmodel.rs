//! First-person viewmodel: Bodycam_Player's `Arms` (SKM_HumanArmsLongSleeves_Morphers, AnimClass Viewmodel_C)
//! with the equipped gun attached at `ik_hand_gun` (BP_EquippableItem.ItemAttachBone = VB ik_hand_gun, identity).
//! Models/clips come from tools/build_models.py (assets-local/models). The arms hang off the body-driven view
//! pose (`Player::view_base`) so the head's FP_Camera socket - where UE attaches the camera - sits on the camera,
//! while camera-only bob/shakes move the view but not the gun, as in UE.
//! Animation: base loop (Idle_Loop01 / Walk_Loop / AimIdlePose) plus the BAS montages from `MontageStarted`
//! and `Fired`, crossfaded; the gun plays the matching A_W_* clip.
use crate::player::Player;
use crate::weapon::{Fired, Inventory, MontageStarted};
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

pub struct ViewmodelPlugin;
impl Plugin for ViewmodelPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, load)
            .add_systems(Update, (spawn_arms, hook_players, attach_gun, play_events, base_loop).chain())
            .add_systems(PostUpdate, place_arms.before(bevy::transform::TransformSystems::Propagate));
    }
}

#[derive(Resource)]
struct ViewmodelAssets {
    arms: Handle<Gltf>,
    guns: HashMap<&'static str, Handle<Gltf>>,
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
    cmd.insert_resource(ViewmodelAssets { arms: assets.load("models/arms.glb"), guns });
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
}

fn clip_graph(g: &Gltf, graphs: &mut Assets<AnimationGraph>) -> (Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>) {
    let mut graph = AnimationGraph::new();
    let nodes = g.named_animations.iter().map(|(n, c)| (n.to_string(), graph.add_clip(c.clone(), 1.0, graph.root))).collect();
    (graphs.add(graph), nodes)
}

fn spawn_arms(
    mut cmd: Commands,
    vm: Res<ViewmodelAssets>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    player: Single<Entity, With<Player>>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(g) = gltfs.get(&vm.arms) else { return };
    *done = true;
    let (graph, nodes) = clip_graph(g, &mut graphs);
    let arms = cmd
        .spawn((
            ArmsRoot,
            WorldAssetRoot(g.scenes[0].clone()),
            AnimationGraphHandle(graph),
            Anim { nodes, current: None, montage_until: 0.0 },
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
    let (graph, nodes) = clip_graph(g, &mut graphs);
    let gun = cmd
        .spawn((GunRoot(want), WorldAssetRoot(g.scenes[0].clone()), AnimationGraphHandle(graph), Anim { nodes, current: None, montage_until: 0.0 }, Transform::default()))
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
    let mut starts: Vec<(String, f32)> = montages.read().map(|m| (clip_of(&m.montage.path), m.montage.length)).collect();
    if fired.read().count() > 0
        && let Some(inv) = &inv
        && let Some(path) = inv.slots[inv.current].anims["Fire"][0]["Character"]["path"].as_str()
    {
        starts.push((clip_of(path), 0.0)); // fire is additive-ish: play, but let the base loop resume right away
    }
    let now = time.elapsed_secs();
    for (clip, len) in starts {
        for (mut player, mut tr, link) in &mut players {
            let Ok((mut anim, is_arms)) = roots.get_mut(link.0) else { continue };
            let name = if is_arms { clip.clone() } else { clip.replacen("A_FP_", "A_W_", 1) };
            if play(&mut anim, &mut player, &mut tr, &name, false) {
                anim.montage_until = now + if len > 0.0 { len } else { 0.25 };
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
    let moving = Vec2::new(p.vel.x, p.vel.z).length() > 0.1;
    let aim = anims["AimIdlePose"].as_str().map(clip_of);
    let want = match (p.aiming, aim) {
        (true, Some(a)) => a,
        _ if moving => format!("{prefix}Walk_Loop"),
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

/// Arms root = body view pose * inverse(FP_Camera socket): the socket lands on the un-shaken camera point.
fn place_arms(p: Single<&Player>, mut arms: Query<&mut Transform, With<ArmsRoot>>) {
    for mut t in &mut arms {
        *t = p.view_base * Transform::from_translation(-FP_CAMERA_SOCKET);
    }
}
