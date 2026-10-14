//! Full-body player character (Bodycam_Player.Mesh = DefaultOperator on the ALS mannequin skeleton) so the player has legs
//! when looking down and a shadow. UE (SetUpCharacterMeshesVisibility, owner): Mesh hides bones head/UpperArm_L/UpperArm_R
//! (the FP Arms replace them) and CastShadow=false, while a hidden `Shadow` mesh (bCastHiddenShadow) casts the full-body shadow.
//! Same here: a visible copy with those bones scaled to 0 and no shadow, plus a shadow-only copy on RenderLayers 1
//! (the camera only draws layer 0; every light also lights layer 1). Model/clips: tools/build_models.py -> models/body.glb.
//! Upper body: WEP/BP_EquippableItem.GetOverlay is Default for every weapon, so the body's arms follow the FP animation, not an
//! ALS gun overlay. The shadow copy therefore also hides its upper arms and the FP arms + gun cast that part of the shadow
//! (same pose, every gun, every montage).
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
/// Shadow copy: keeps the head; the FP arms cast the arms' shadow.
const SHADOW_HIDDEN_BONES: [&str; 2] = ["upperarm_l", "upperarm_r"];
/// ALS spine rotation: aim pitch shared over the three spine bones (shadow copy; the owner copy stays upright so the chest
/// doesn't swing into the camera, which here is not parented to the head). Lean roll (Q/E) on both copies.
const SPINE: [&str; 3] = ["spine_01", "spine_02", "spine_03"];
const AIM_PITCH_SHARE: f32 = 0.5;
const LEAN_ROLL_DEG: f32 = 18.0;
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

/// ALS locomotion: clip by stance, gait and local move direction (N walk/run F, LF, RF, LB, RB, B; crouch F/L/R/B), in-air
/// jump/fall loops, rate by speed.
// ponytail: nearest-direction clip with a 0.2 s crossfade (no ALS sync-marker blending, so feet can skip on a direction
// change); no turn-in-place, landing additive or stop transitions.
fn drive(
    player: Single<(&Player, &Transform)>,
    mut roots: Query<(Entity, &mut BodyRoot)>,
    mut players: Query<(Entity, &mut AnimationPlayer, &mut AnimationTransitions)>,
    parents: Query<&ChildOf>,
) {
    let (p, tf) = *player;
    let local = tf.rotation.inverse() * Vec3::new(p.vel.x, 0.0, p.vel.z);
    let speed = local.length();
    let (clip, rate) = pick_clip(p.grounded, p.vel.y, p.crouched, p.sprinting, speed, (-local.z).atan2(local.x).to_degrees());
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
            tr.play(&mut ap, node, Duration::from_millis(200)).repeat();
            r.current = clip.into();
        }
        if let Some(a) = ap.animation_mut(node) {
            a.set_speed(rate.clamp(0.3, 2.0));
        }
    }
}

/// `heading`: move direction in degrees, 90 = forward, 0 = right, 180 = left, -90 = back.
fn pick_clip(grounded: bool, vy: f32, crouched: bool, sprinting: bool, speed: f32, heading: f32) -> (&'static str, f32) {
    if !grounded {
        return (if vy > 0.0 { "ALS_N_JumpLoop" } else { "ALS_N_FallLoop" }, 1.0);
    }
    if speed < 0.15 {
        return (if crouched { "ALS_CLF_Pose" } else { "ALS_N_Pose" }, 1.0);
    }
    // Angle from forward: 0 = forward, +90 = right, -90 = left, +-180 = back.
    let a = 90.0 - heading;
    let a = (a + 180.0).rem_euclid(360.0) - 180.0;
    if crouched {
        let c = match a {
            a if a.abs() <= 45.0 => "ALS_CLF_Walk_F",
            a if a.abs() >= 135.0 => "ALS_CLF_Walk_B",
            a if a > 0.0 => "ALS_CLF_Walk_R",
            _ => "ALS_CLF_Walk_L",
        };
        return (c, speed / CROUCH_REF);
    }
    let (set, reference) = if sprinting { (["ALS_N_Run_F", "ALS_N_Run_RF", "ALS_N_Run_RB", "ALS_N_Run_B", "ALS_N_Run_LB", "ALS_N_Run_LF"], RUN_REF) } else { (["ALS_N_Walk_F", "ALS_N_Walk_RF", "ALS_N_Walk_RB", "ALS_N_Walk_B", "ALS_N_Walk_LB", "ALS_N_Walk_LF"], WALK_REF) };
    let c = match a {
        a if a.abs() <= 30.0 => set[0],
        a if a.abs() >= 150.0 => set[3],
        a if a > 0.0 => if a <= 100.0 { set[1] } else { set[2] },
        a => if a >= -100.0 { set[5] } else { set[4] },
    };
    (c, speed / reference)
}

/// HideBoneByName: collapse the bone (and, through the hierarchy, everything under it); then ALS spine rotation (aim pitch,
/// shadow copy) and lean roll. Axes come from last frame's bone GlobalTransform (the pose is about to be propagated).
fn hide_bones(
    player: Single<(&Player, &GlobalTransform)>,
    mut joints: Query<(Entity, &Name, &mut Transform, &GlobalTransform), With<AnimationTargetId>>,
    parents: Query<&ChildOf>,
    roots: Query<&BodyRoot>,
) {
    let (p, pg) = *player;
    let (right, fwd) = (pg.right().as_vec3(), pg.forward().as_vec3());
    for (e, n, mut t, g) in &mut joints {
        let Some(shadow) = root_of(e, &parents, &roots).map(|r| roots.get(r).unwrap().shadow) else { continue };
        let name = n.as_str();
        if (if shadow { &SHADOW_HIDDEN_BONES[..] } else { &HIDDEN_BONES[..] }).contains(&name) {
            t.scale = Vec3::ZERO;
        }
        if SPINE.contains(&name) {
            let inv = g.rotation().inverse();
            let pitch = if shadow { p.pitch * AIM_PITCH_SHARE / 3.0 } else { 0.0 };
            let roll = -p.lean * LEAN_ROLL_DEG.to_radians() / 3.0;
            t.rotation = t.rotation * Quat::from_axis_angle((inv * right).normalize(), pitch) * Quat::from_axis_angle((inv * fwd).normalize(), roll);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::pick_clip;
    #[test]
    fn clip_by_direction() {
        let walk = |h: f32| pick_clip(true, 0.0, false, false, 1.25, h).0;
        assert_eq!(walk(90.0), "ALS_N_Walk_F");
        assert_eq!(walk(-90.0), "ALS_N_Walk_B");
        assert_eq!(walk(0.0), "ALS_N_Walk_RF");
        assert_eq!(walk(180.0), "ALS_N_Walk_LF");
        assert_eq!(walk(-45.0), "ALS_N_Walk_RB");
        assert_eq!(pick_clip(true, 0.0, true, false, 1.0, 180.0).0, "ALS_CLF_Walk_L");
        assert_eq!(pick_clip(false, 2.0, false, false, 1.0, 90.0).0, "ALS_N_JumpLoop");
        assert_eq!(pick_clip(true, 0.0, false, false, 0.0, 90.0).0, "ALS_N_Pose");
    }
}
