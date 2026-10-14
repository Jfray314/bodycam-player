//! Full-body player character (Bodycam_Player.Mesh = DefaultOperator on the ALS mannequin skeleton) so the player has legs
//! when looking down and a shadow. UE (SetUpCharacterMeshesVisibility, owner): Mesh hides bones head/UpperArm_L/UpperArm_R
//! (the FP Arms replace them) and CastShadow=false, while a hidden `Shadow` mesh (bCastHiddenShadow) casts the full-body shadow.
//! Same here: a visible copy with those bones scaled to 0 and no shadow, plus a shadow-only copy on RenderLayers 1
//! (the camera only draws layer 0; every light also lights layer 1). Model/clips: tools/build_models.py -> models/body.glb.
use crate::player::Player;
use bevy::animation::AnimationTargetId;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::gltf::Gltf;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use std::collections::HashMap;
use std::time::Duration;

const SHADOW_LAYER: usize = 1;
const HIDDEN_BONES: [&str; 3] = ["head", "upperarm_l", "upperarm_r"];
/// Forward speed (m/s) at which each ALS clip plays at rate 1 (ALS defaults: walk 165, run 375, crouch 150 cm/s).
const WALK_REF: f32 = 1.65;
const RUN_REF: f32 = 3.75;
const CROUCH_REF: f32 = 1.5;

pub struct BodyPlugin;
impl Plugin for BodyPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, |mut cmd: Commands, a: Res<AssetServer>| cmd.insert_resource(BodyAsset(a.load("models/body.glb"))))
            .add_systems(Update, (spawn, prepare, light_layers, drive).chain())
            .add_systems(PostUpdate, hide_bones.after(bevy::app::AnimationSystems).before(bevy::transform::TransformSystems::Propagate));
    }
}

#[derive(Resource)]
struct BodyAsset(Handle<Gltf>);

/// `shadow`: the shadow-only copy (full body); otherwise the owner-visible one (no head/upper arms, no shadow).
#[derive(Component)]
pub struct BodyRoot {
    shadow: bool,
    nodes: HashMap<String, AnimationNodeIndex>,
    current: String,
}

fn spawn(mut cmd: Commands, asset: Res<BodyAsset>, gltfs: Res<Assets<Gltf>>, mut graphs: ResMut<Assets<AnimationGraph>>, player: Single<Entity, With<Player>>, mut done: Local<bool>) {
    if *done {
        return;
    }
    let Some(g) = gltfs.get(&asset.0) else { return };
    *done = true;
    let mut graph = AnimationGraph::new();
    let nodes: HashMap<String, AnimationNodeIndex> = g.named_animations.iter().map(|(n, c)| (n.to_string(), graph.add_clip(c.clone(), 1.0, graph.root))).collect();
    let graph = graphs.add(graph);
    for shadow in [false, true] {
        let mut e = cmd.spawn((
            BodyRoot { shadow, nodes: nodes.clone(), current: String::new() },
            WorldAssetRoot(g.scenes[0].clone()),
            AnimationGraphHandle(graph.clone()),
            Transform::default(),
            crate::player::IgnoreTraces,
        ));
        if shadow {
            e.insert(RenderLayers::layer(SHADOW_LAYER));
        }
        let id = e.id();
        cmd.entity(*player).add_child(id);
    }
}

fn root_of(mut e: Entity, parents: &Query<&ChildOf>, roots: &Query<&BodyRoot>) -> Option<Entity> {
    loop {
        if roots.contains(e) {
            return Some(e);
        }
        e = parents.get(e).ok()?.parent();
    }
}

/// Per new scene node: meshes (skinned bounds go stale under animation; shadow copy off-camera; owner copy casts no shadow) and animation players.
fn prepare(
    mut cmd: Commands,
    meshes: Query<Entity, Added<Mesh3d>>,
    players: Query<Entity, Added<AnimationPlayer>>,
    parents: Query<&ChildOf>,
    roots: Query<&BodyRoot>,
    graphs: Query<&AnimationGraphHandle>,
) {
    for e in &meshes {
        if let Some(r) = root_of(e, &parents, &roots) {
            cmd.entity(e).insert((NoFrustumCulling, crate::player::IgnoreTraces));
            if roots.get(r).unwrap().shadow {
                cmd.entity(e).insert(RenderLayers::layer(SHADOW_LAYER));
            } else {
                cmd.entity(e).insert(NotShadowCaster);
            }
        }
    }
    for e in &players {
        if let Some(r) = root_of(e, &parents, &roots)
            && let Ok(g) = graphs.get(r)
        {
            cmd.entity(e).insert((g.clone(), AnimationTransitions::new()));
        }
    }
}

/// Lights also light the shadow-only layer.
fn light_layers(mut cmd: Commands, new: Query<Entity, Or<(Added<DirectionalLight>, Added<PointLight>, Added<SpotLight>)>>) {
    for e in &new {
        cmd.entity(e).insert(RenderLayers::from_layers(&[0, SHADOW_LAYER]));
    }
}

/// Locomotion clip by state, rate by speed. ponytail: forward-only clips (no strafe/backpedal, turn-in-place, lean, jump/fall,
/// aim offset/spine pitch); the ALS_N_*_B/L/R, Jump*, Fall* clips are in the pak under AnimationExamples/Base if wanted.
fn drive(
    player: Single<&Player>,
    mut roots: Query<(Entity, &mut BodyRoot)>,
    mut players: Query<(Entity, &mut AnimationPlayer, &mut AnimationTransitions)>,
    parents: Query<&ChildOf>,
) {
    let p = *player;
    let speed = Vec2::new(p.vel.x, p.vel.z).length();
    let moving = speed > 0.15 && p.grounded;
    let (clip, rate) = match (p.crouched, moving, p.sprinting) {
        (true, true, _) => ("ALS_CLF_Walk_F", speed / CROUCH_REF),
        (true, false, _) => ("ALS_CLF_Pose", 1.0),
        (false, true, true) => ("ALS_N_Run_F", speed / RUN_REF),
        (false, true, false) => ("ALS_N_Walk_F", speed / WALK_REF),
        _ => ("ALS_N_Pose", 1.0),
    };
    for (pe, mut ap, mut tr) in &mut players {
        let mut e = pe;
        let root = loop {
            if roots.contains(e) {
                break Some(e);
            }
            match parents.get(e) {
                Ok(c) => e = c.parent(),
                Err(_) => break None,
            }
        };
        let Some((_, mut r)) = root.and_then(|r| roots.get_mut(r).ok()) else { continue };
        let Some(&node) = r.nodes.get(clip) else { continue };
        if r.current != clip {
            tr.play(&mut ap, node, Duration::from_millis(250)).repeat();
            r.current = clip.into();
        }
        if let Some(a) = ap.animation_mut(node) {
            a.set_speed(rate.clamp(0.3, 2.0));
        }
    }
}

/// HideBoneByName: collapse the bone (and, through the hierarchy, everything under it) on the owner-visible copy.
fn hide_bones(mut joints: Query<(Entity, &Name, &mut Transform), With<AnimationTargetId>>, parents: Query<&ChildOf>, roots: Query<&BodyRoot>) {
    for (e, n, mut t) in &mut joints {
        if HIDDEN_BONES.contains(&n.as_str()) && root_of(e, &parents, &roots).is_some_and(|r| !roots.get(r).unwrap().shadow) {
            t.scale = Vec3::ZERO;
        }
    }
}
