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
/// Fall speed (m/s) for any landing animation / the heavy one.
const LAND_MIN: f32 = 2.5;
const LAND_HEAVY: f32 = 6.0;

pub struct BodyPlugin;
impl Plugin for BodyPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BodyAnim>().init_resource::<ThirdPerson>().add_systems(Startup, |mut cmd: Commands, a: Res<AssetServer>| cmd.insert_resource(BodyAsset(a.load("models/body.glb"))))
            .add_systems(Update, (spawn, prepare, light_layers, drive).chain())
            .add_systems(PostUpdate, (hide_bones.after(bevy::app::AnimationSystems), third_person_cam).before(bevy::transform::TransformSystems::Propagate));
    }
}

/// Debug third-person orbit view (F8 toggles; BODYCAM_3P="dist:yaw_deg:height" starts it): camera orbits the player, the owner
/// body copy shows its head and arms. For checking the body as a third-person model.
#[derive(Resource)]
pub struct ThirdPerson {
    pub on: bool,
    dist: f32,
    yaw: f32,
    height: f32,
}
impl Default for ThirdPerson {
    fn default() -> Self {
        let v: Vec<f32> = std::env::var("BODYCAM_3P").unwrap_or_default().split(':').filter_map(|x| x.parse().ok()).collect();
        Self { on: !v.is_empty(), dist: v.first().copied().unwrap_or(2.5), yaw: v.get(1).copied().unwrap_or(150.0).to_radians(), height: v.get(2).copied().unwrap_or(1.3) }
    }
}

fn third_person_cam(keys: Res<ButtonInput<KeyCode>>, mut tp: ResMut<ThirdPerson>, mut cam: Single<&mut Transform, With<crate::player::BodycamCamera>>) {
    if keys.just_pressed(KeyCode::F8) {
        tp.on = !tp.on;
    }
    if keys.pressed(KeyCode::Comma) {
        tp.yaw -= 0.03;
    }
    if keys.pressed(KeyCode::Period) {
        tp.yaw += 0.03;
    }
    if tp.on {
        // player space: -Z forward, so yaw 0 = camera in front looking back? yaw 180 = behind.
        **cam = Transform::from_xyz(tp.yaw.sin() * tp.dist, tp.height, -tp.yaw.cos() * tp.dist).looking_at(Vec3::new(0.0, 0.9, 0.0), Vec3::Y);
    }
}

#[derive(Resource)]
struct BodyAsset(Handle<Gltf>);

/// `shadow`: the shadow-only copy (full body); otherwise the owner-visible one (no head/upper arms, no shadow).
#[derive(Component)]
pub struct BodyRoot {
    shadow: bool,
    nodes: HashMap<String, AnimationNodeIndex>,
}

fn spawn(mut cmd: Commands, asset: Res<BodyAsset>, gltfs: Res<Assets<Gltf>>, mut graphs: ResMut<Assets<AnimationGraph>>, clips: Res<Assets<AnimationClip>>, mut st: ResMut<BodyAnim>, player: Single<Entity, With<Player>>, mut done: Local<bool>) {
    if *done {
        return;
    }
    let Some(g) = gltfs.get(&asset.0) else { return };
    *done = true;
    let mut graph = AnimationGraph::new();
    let nodes: HashMap<String, AnimationNodeIndex> = g.named_animations.iter().map(|(n, c)| (n.to_string(), graph.add_clip(c.clone(), 1.0, graph.root))).collect();
    st.dur = g.named_animations.iter().map(|(n, c)| (n.to_string(), clips.get(c).map_or(1.0, |c| c.duration().max(0.01)))).collect();
    let graph = graphs.add(graph);
    for shadow in [false, true] {
        let mut e = cmd.spawn((
            BodyRoot { shadow, nodes: nodes.clone() },
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
            cmd.entity(e).insert(g.clone());
        }
    }
}

/// Lights also light the shadow-only layer.
fn light_layers(mut cmd: Commands, new: Query<Entity, Or<(Added<DirectionalLight>, Added<PointLight>, Added<SpotLight>)>>) {
    for e in &new {
        cmd.entity(e).insert(RenderLayers::from_layers(&[0, SHADOW_LAYER]));
    }
}

/// ALS-style locomotion blend. Every clip plays at once with a smoothed weight (the ALS CycleBlending / BlendListBy* graph):
/// ground = two nearest move directions of the walk/run (gait blend) or crouch set, idle pose, jump/fall loop in the air,
/// land overlay after a fall. The loops share one normalised phase (every clip seeks phase * its length), which stands in for
/// the ALS foot sync markers; the phase rate is CalculateStandingPlayRate (speed over the gait's animated speed, lerped by gait).
// ponytail: normalised-time sync, not the ALS sync markers; no turn-in-place (the local player's actor rotation is the control
// yaw: UpdateGroudedRotation IsLocallyControlled branch) and no Stop transitions.
#[derive(Resource, Default)]
struct BodyAnim {
    w: HashMap<&'static str, f32>,
    dur: HashMap<String, f32>,
    phase: f32,
    gait: f32,
    land_t: f32,
    land_heavy: bool,
    was_air: bool,
    fall: f32,
}

type Dirs = [(&'static str, f32)];
const WALK: &Dirs = &[("ALS_N_Walk_LB", -130.0), ("ALS_N_Walk_LF", -50.0), ("ALS_N_Walk_F", 0.0), ("ALS_N_Walk_RF", 50.0), ("ALS_N_Walk_RB", 130.0), ("ALS_N_Walk_B", 180.0)];
const RUN: &Dirs = &[("ALS_N_Run_LB", -130.0), ("ALS_N_Run_LF", -50.0), ("ALS_N_Run_F", 0.0), ("ALS_N_Run_RF", 50.0), ("ALS_N_Run_RB", 130.0), ("ALS_N_Run_B", 180.0)];
const CROUCH: &Dirs = &[("ALS_CLF_Walk_L", -90.0), ("ALS_CLF_Walk_F", 0.0), ("ALS_CLF_Walk_R", 90.0), ("ALS_CLF_Walk_B", 180.0)];
const AIR: [&str; 2] = ["ALS_N_JumpLoop", "ALS_N_FallLoop"];
const BLEND_RATE: f32 = 9.0;

/// Adds `scale` split between the two set entries bracketing `a` (degrees, 0 = forward, +right; set sorted ascending, cyclic).
fn dir_weights(set: &Dirs, a: f32, scale: f32, out: &mut Vec<(&'static str, f32)>) {
    let n = set.len();
    for i in 0..n {
        let lo = set[i].1;
        let hi = if i + 1 < n { set[i + 1].1 } else { set[0].1 + 360.0 };
        let aa = if a < lo { a + 360.0 } else { a };
        if aa >= lo && aa < hi {
            let t = (aa - lo) / (hi - lo);
            out.push((set[i].0, scale * (1.0 - t)));
            out.push((set[(i + 1) % n].0, scale * t));
            return;
        }
    }
}

/// Target clip weights (sum 1 without the land overlay) and the phase play rate.
fn targets(grounded: bool, vy: f32, crouched: bool, gait: f32, speed: f32, a: f32) -> (Vec<(&'static str, f32)>, f32) {
    let mut w = Vec::new();
    if !grounded {
        w.push((AIR[(vy <= 0.0) as usize], 1.0));
        return (w, 1.0);
    }
    let m = ((speed - 0.1) / 0.3).clamp(0.0, 1.0); // idle <-> move
    w.push((if crouched { "ALS_CLF_Pose" } else { "ALS_N_Pose" }, 1.0 - m));
    if m > 0.0 {
        if crouched {
            dir_weights(CROUCH, a, m, &mut w);
            return (w, speed / CROUCH_REF);
        }
        dir_weights(WALK, a, m * (1.0 - gait), &mut w);
        dir_weights(RUN, a, m * gait, &mut w);
    }
    (w, (speed / WALK_REF) * (1.0 - gait) + (speed / RUN_REF) * gait)
}

fn drive(
    time: Res<Time>,
    player: Single<(&Player, &Transform)>,
    mut st: ResMut<BodyAnim>,
    roots: Query<&BodyRoot>,
    mut players: Query<(Entity, &mut AnimationPlayer)>,
    parents: Query<&ChildOf>,
) {
    let dt = time.delta_secs().min(0.1);
    let (p, tf) = *player;
    let local = tf.rotation.inverse() * Vec3::new(p.vel.x, 0.0, p.vel.z);
    let speed = local.length();
    st.gait += ((p.sprinting as i32 as f32) - st.gait) * (1.0 - (-dt * 6.0).exp());
    // Land overlay: the fall speed seen while airborne decides light/heavy (ALS Land_Light/Land_Heavy).
    if !p.grounded {
        st.was_air = true;
        st.fall = st.fall.min(p.vel.y);
    } else if st.was_air {
        st.was_air = false;
        if st.fall < -LAND_MIN {
            st.land_t = 0.0;
            st.land_heavy = st.fall < -LAND_HEAVY;
        }
        st.fall = 0.0;
    }
    st.land_t += dt;
    let land_clip = if st.land_heavy && st.dur.contains_key("ALS_N_Land_Heavy") { "ALS_N_Land_Heavy" } else { "ALS_N_Land_Light" };
    let land_dur = st.dur.get(land_clip).copied().unwrap_or(0.5);
    let land = if st.land_t < land_dur { (st.land_t / 0.06).min(1.0) * ((land_dur - st.land_t) / 0.25).min(1.0) } else { 0.0 };

    let (tg, rate) = targets(p.grounded, p.vel.y, p.crouched, st.gait, speed, local.x.atan2(-local.z).to_degrees());
    for (n, _) in &tg {
        st.w.entry(n).or_insert(0.0);
    }
    let k = 1.0 - (-dt * BLEND_RATE).exp();
    for (n, w) in st.w.iter_mut() {
        let t = tg.iter().filter(|(c, _)| c == n).map(|c| c.1).sum::<f32>();
        *w += (t - *w) * k;
    }
    // Shared phase: rate / blended loop length over the moving clips.
    let (mut dsum, mut wsum) = (0.0, 0.0);
    for (n, w) in st.w.iter().filter(|(n, _)| n.contains("Walk") || n.contains("Run")) {
        dsum += w * st.dur.get(*n).copied().unwrap_or(1.0);
        wsum += w;
    }
    if wsum > 0.01 {
        st.phase = (st.phase + dt * rate.clamp(0.0, 3.0) * wsum / dsum).rem_euclid(1.0);
    }
    let (phase, land_t) = (st.phase, st.land_t);
    for (pe, mut ap) in &mut players {
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
        let Some(r) = root.and_then(|r| roots.get(r).ok()) else { continue };
        for (name, &node) in &r.nodes {
            let is_land = name.starts_with("ALS_N_Land");
            let w = if is_land { if name == land_clip { land } else { 0.0 } } else { st.w.get(name.as_str()).copied().unwrap_or(0.0) * (1.0 - land) };
            if ap.animation_mut(node).is_none() {
                ap.play(node).repeat().set_weight(0.0);
            }
            let a = ap.animation_mut(node).unwrap();
            a.set_weight(w);
            let dur = st.dur.get(name).copied().unwrap_or(1.0);
            if name.contains("Walk") || name.contains("Run") {
                a.set_speed(0.0).set_seek_time(phase * dur);
            } else if is_land {
                a.set_speed(0.0).set_seek_time(land_t.min(dur));
            }
        }
    }
}

/// HideBoneByName: collapse the bone (and, through the hierarchy, everything under it); then ALS spine rotation (aim pitch,
/// shadow copy) and lean roll. Axes come from last frame's bone GlobalTransform (the pose is about to be propagated).
fn hide_bones(
    player: Single<(&Player, &GlobalTransform)>,
    mut joints: Query<(Entity, &Name, &mut Transform, &GlobalTransform), With<AnimationTargetId>>,
    parents: Query<&ChildOf>,
    roots: Query<&BodyRoot>,
    tp: Res<ThirdPerson>,
) {
    let (p, pg) = *player;
    let (right, fwd) = (pg.right().as_vec3(), pg.forward().as_vec3());
    for (e, n, mut t, g) in &mut joints {
        let Some(shadow) = root_of(e, &parents, &roots).map(|r| roots.get(r).unwrap().shadow) else { continue };
        let name = n.as_str();
        if !tp.on && (if shadow { &SHADOW_HIDDEN_BONES[..] } else { &HIDDEN_BONES[..] }).contains(&name) {
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
    use super::targets;
    #[test]
    fn blend_weights() {
        let get = |w: &[(&str, f32)], n: &str| w.iter().filter(|c| c.0 == n).map(|c| c.1).sum::<f32>();
        let (w, _) = targets(true, 0.0, false, 0.0, 1.25, 0.0);
        assert!((get(&w, "ALS_N_Walk_F") - 1.0).abs() < 1e-5);
        let (w, _) = targets(true, 0.0, false, 0.0, 1.25, 90.0); // strafe right = RF/RB mix
        assert!(get(&w, "ALS_N_Walk_RF") > 0.4 && get(&w, "ALS_N_Walk_RB") > 0.3);
        let (w, _) = targets(true, 0.0, false, 0.5, 2.0, -170.0);
        assert!((w.iter().map(|c| c.1).sum::<f32>() - 1.0).abs() < 1e-4 && get(&w, "ALS_N_Run_B") > 0.39 && get(&w, "ALS_N_Walk_LB") > 0.0);
        assert_eq!(targets(false, 2.0, false, 0.0, 1.0, 0.0).0[0].0, "ALS_N_JumpLoop");
        assert_eq!(targets(true, 0.0, true, 0.0, 0.0, 0.0).0[0].0, "ALS_CLF_Pose");
    }
}
