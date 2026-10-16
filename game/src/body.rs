//! Full-body character (Bodycam_Player.Mesh = DefaultOperator on the ALS mannequin skeleton): the local player's owner view (legs
//! when looking down) and the world copy every other camera sees (character.rs: spawn_character, driven by CharacterState).
//! UE (SetUpCharacterMeshesVisibility, owner): Mesh hides bones head/UpperArm_L/UpperArm_R (the FP Arms replace them) and
//! CastShadow=false, while a hidden `Shadow` mesh (bCastHiddenShadow) casts the full-body shadow.
//! Same here: an owner copy (visible, those bones scaled to 0, no shadow) plus a world copy on RenderLayers 1 (shadow only in the
//! owner's FP view; every light also lights layer 1). When the camera is detached (drone / 3P orbit) the owner copy is hidden and
//! the world copy is drawn on layer 0 with head, arms and gun, exactly like a remote player. Model/clips: tools/build_models.py.
//! Upper body: WEP/BP_EquippableItem.GetOverlay is Default for every weapon, so the owner's FP arms follow the FP animation, not an
//! ALS gun overlay; in the FP view the local world copy hides its upper arms and the FP arms + gun cast that part of the shadow.
use crate::character::{CharGun, CharacterState, NeedsBody};
use bevy::animation::AnimationTargetId;
use bevy::camera::visibility::{NoFrustumCulling, RenderLayers};
use bevy::gltf::Gltf;
use bevy::light::NotShadowCaster;
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use std::collections::HashMap;

const SHADOW_LAYER: usize = 1;
const HIDDEN_BONES: [&str; 3] = ["head", "upperarm_l", "upperarm_r"];
/// Local world copy in the FP view: keeps the head; the FP arms cast the arms' shadow.
const SHADOW_HIDDEN_BONES: [&str; 2] = ["upperarm_l", "upperarm_r"];
/// ALS spine rotation: aim pitch (AimOffset stand-in) shared over spine (total), neck and head on the world copies (the owner copy
/// stays upright so the chest doesn't swing into the camera, which here is not parented to the head). Lean roll (Q/E) on all.
const SPINE: [&str; 3] = ["spine_01", "spine_02", "spine_03"];
const SPINE_SHARE: f32 = 0.45;
const NECK_SHARE: f32 = 0.15;
const HEAD_SHARE: f32 = 0.25;
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
        app.add_plugins(crate::bodyik::IkPlugin)
            .init_resource::<ClipDurs>()
            .init_resource::<MontageNodes>()
            .init_resource::<ThirdPerson>()
            .init_resource::<Detached>()
            .add_systems(Startup, |mut cmd: Commands, a: Res<AssetServer>| cmd.insert_resource(BodyAsset(a.load("models/body.glb"))))
            .add_systems(Update, (detached_view, build_bodies, prepare, light_layers, body_layers, montage_clips, anim_state, drive).chain())
            .add_systems(PostUpdate, (hide_bones.after(bevy::app::AnimationSystems), third_person_cam).before(bevy::transform::TransformSystems::Propagate));
    }
}

/// Debug third-person orbit view (Insert toggles; BODYCAM_3P="dist:yaw_deg:height" starts it): the camera orbits the player (or the
/// test puppet when there is one) and sees the world copy: head, helmet, IK arms and the gun.
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

/// The camera is not at the owner's head (3P orbit / piloting the drone): owner copy hidden, world copy + gun drawn on layer 0,
/// camera without the viewmodel layer.
#[derive(Resource, Default)]
pub struct Detached(pub bool);

fn detached_view(tp: Res<ThirdPerson>, perk: Option<Res<crate::perks::Perk>>, mut det: ResMut<Detached>, mut cam: Single<&mut RenderLayers, With<crate::player::BodycamCamera>>) {
    let d = tp.on || perk.is_some_and(|p| p.phase == crate::perks::Phase::Piloting);
    if det.0 != d {
        det.0 = d;
        **cam = if d { RenderLayers::layer(0) } else { RenderLayers::from_layers(&[0, crate::attach::VM_LAYER]) };
    }
}

fn third_person_cam(
    keys: Res<ButtonInput<KeyCode>>,
    mut tp: ResMut<ThirdPerson>,
    puppet: Query<&GlobalTransform, With<crate::character::Puppet>>,
    player: Single<&GlobalTransform, With<crate::player::Player>>,
    mut cam: Single<&mut Transform, With<crate::player::BodycamCamera>>,
) {
    if keys.just_pressed(KeyCode::Insert) {
        tp.on = !tp.on;
    }
    if keys.pressed(KeyCode::Comma) {
        tp.yaw -= 0.03;
    }
    if keys.pressed(KeyCode::Period) {
        tp.yaw += 0.03;
    }
    if tp.on {
        // Orbit centre + frame (player space): the puppet if there is one, else the player. Facing -Z: yaw 0 = camera in front
        // looking back, 180 = behind.
        let (c, rot) = puppet.single().map_or((Vec3::ZERO, Quat::IDENTITY), |p| {
            let (_, r, t) = (player.affine().inverse() * p.affine()).to_scale_rotation_translation();
            (t, r)
        });
        let o = rot * Vec3::new(tp.yaw.sin() * tp.dist, 0.0, -tp.yaw.cos() * tp.dist);        **cam = Transform::from_translation(c + o + Vec3::Y * tp.height).looking_at(c + Vec3::Y * 0.9, Vec3::Y);
    }
}

#[derive(Resource)]
struct BodyAsset(Handle<Gltf>);

/// Clip name -> length (s).
#[derive(Resource, Default)]
pub(crate) struct ClipDurs(HashMap<String, f32>);

/// `world`: the full-body copy every other camera sees; otherwise the owner-only one (no head/upper arms, no shadow).
/// `local`: belongs to the local player (foot IK from the level; its world copy hides arms and gun in the FP view).
#[derive(Component)]
pub struct BodyRoot {
    pub(crate) world: bool,
    pub(crate) local: bool,
    nodes: HashMap<String, AnimationNodeIndex>,
    /// Bone name -> entity (filled by bodyik).
    pub(crate) bones: HashMap<String, Entity>,
}

/// A mesh of a body copy (or its gun): `root` is the BodyRoot entity, whose kind decides the RenderLayers.
#[derive(Component)]
struct BodyMesh {
    root: Entity,
    gun: bool,
}

#[allow(clippy::too_many_arguments)]
fn build_bodies(
    mut cmd: Commands,
    asset: Res<BodyAsset>,
    gltfs: Res<Assets<Gltf>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    clips: Res<Assets<AnimationClip>>,
    mut durs: ResMut<ClipDurs>,
    mut shared: Local<Option<(Handle<AnimationGraph>, HashMap<String, AnimationNodeIndex>)>>,
    pending: Query<(Entity, &NeedsBody)>,
) {
    if pending.is_empty() {
        return;
    }
    let Some(g) = gltfs.get(&asset.0) else { return };
    let (graph, nodes) = shared
        .get_or_insert_with(|| {
            let mut graph = AnimationGraph::new();
            let nodes: HashMap<String, AnimationNodeIndex> = g.named_animations.iter().map(|(n, c)| (n.to_string(), graph.add_clip(c.clone(), 1.0, graph.root))).collect();
            durs.0 = g.named_animations.iter().map(|(n, c)| (n.to_string(), clips.get(c).map_or(1.0, |c| c.duration().max(0.01)))).collect();
            (graphs.add(graph), nodes)
        })
        .clone();
    for (ch, nb) in &pending {
        cmd.entity(ch).remove::<NeedsBody>();
        for world in [false, true] {
            if !world && !nb.local {
                continue; // remote characters only have the full copy
            }
            let mut e = cmd.spawn((
                BodyRoot { world, local: nb.local, nodes: nodes.clone(), bones: default() },
                BodyAnim::default(),
                WorldAssetRoot(g.scenes[0].clone()),
                AnimationGraphHandle(graph.clone()),
                Transform::default(),
                Visibility::default(),
            ));
            if nb.local {
                e.insert(crate::player::IgnoreTraces);
            }
            if world {
                e.insert(RenderLayers::layer(SHADOW_LAYER));
            }
            let id = e.id();
            cmd.entity(ch).add_child(id);
        }
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

/// `e` is inside a CharGun scene (its AnimationPlayer must not get the body graph, its joints are not the body's).
pub(crate) fn under_gun(mut e: Entity, parents: &Query<&ChildOf>, guns: &Query<(), With<CharGun>>) -> bool {
    while let Ok(c) = parents.get(e) {
        e = c.parent();
        if guns.contains(e) {
            return true;
        }
    }
    false
}

/// Per new scene node: meshes (skinned bounds go stale under animation; the local copies are ignored by traces) and animation players.
fn prepare(
    mut cmd: Commands,
    meshes: Query<Entity, Added<Mesh3d>>,
    players: Query<Entity, Added<AnimationPlayer>>,
    parents: Query<&ChildOf>,
    roots: Query<&BodyRoot>,
    guns: Query<(), With<CharGun>>,
    graphs: Query<&AnimationGraphHandle>,
) {
    for e in &meshes {
        if let Some(r) = root_of(e, &parents, &roots) {
            let b = roots.get(r).unwrap();
            cmd.entity(e).insert((NoFrustumCulling, BodyMesh { root: r, gun: under_gun(e, &parents, &guns) }));
            if b.local {
                cmd.entity(e).insert(crate::player::IgnoreTraces);
            }
            if b.world {
                cmd.entity(e).insert(RenderLayers::layer(SHADOW_LAYER));
            } else {
                cmd.entity(e).insert(NotShadowCaster);
            }
        }
    }
    for e in &players {
        if let Some(r) = root_of(e, &parents, &roots)
            && !under_gun(e, &parents, &guns)
            && let Ok(g) = graphs.get(r)
        {
            cmd.entity(e).insert(g.clone());
        }
    }
}

/// Lights also light the shadow-only layer.
fn light_layers(mut cmd: Commands, new: Query<Entity, Or<(Added<DirectionalLight>, Added<PointLight>, Added<SpotLight>)>>) {
    for e in &new {
        cmd.entity(e).insert(RenderLayers::from_layers(&[0, SHADOW_LAYER, crate::attach::VM_LAYER]));
    }
}

/// Which camera sees which copy: owner copy on layer 0 (hidden when detached); local world copy shadow-only (layer 1) until the
/// camera is detached, its gun not at all (the FP gun casts the shadow); remote world copies always on layers 0 + 1.
fn body_layers(det: Res<Detached>, roots: Query<&BodyRoot>, mut meshes: Query<(&BodyMesh, &mut RenderLayers)>) {
    for (m, mut l) in &mut meshes {
        let Ok(r) = roots.get(m.root) else { continue };
        let want = match (r.world, r.local && !det.0) {
            (false, _) if det.0 => RenderLayers::none(),
            (false, _) => RenderLayers::layer(0),
            (true, true) if m.gun => RenderLayers::none(),
            (true, true) => RenderLayers::layer(SHADOW_LAYER),
            (true, false) => RenderLayers::from_layers(&[0, SHADOW_LAYER]),
        };
        if *l != want {
            *l = want;
        }
    }
}

/// ALS-style locomotion blend, one per body copy. Every clip plays at once with a smoothed weight (the ALS CycleBlending /
/// BlendListBy* graph): ground = two nearest move directions of the walk/run (gait blend) or crouch set, idle pose, jump/fall loop in
/// the air, land overlay after a fall. The loops share one normalised phase (every clip seeks phase * its length), which stands in for
/// the ALS foot sync markers; the phase rate is CalculateStandingPlayRate (speed over the gait's animated speed, lerped by gait).
// ponytail: normalised-time sync, not the ALS sync markers; no turn-in-place (the local player's actor rotation is the control
// yaw: UpdateGroudedRotation IsLocallyControlled branch) and no Stop transitions.
// Montages (reload, mag check, draw, ...) are the owner's A_FP_* clips on the UpperBody slot, as in UE. Kick/hit/death are procedural:
// UE has no character fire/hit montage on remotes here and dies into the ALS ragdoll.
#[derive(Component, Default)]
pub(crate) struct BodyAnim {
    w: HashMap<&'static str, f32>,
    phase: f32,
    gait: f32,
    land_t: f32,
    land_heavy: bool,
    was_air: bool,
    fall: f32,
    /// Smoothed ADS 0..1, recoil kick 1 -> 0, hit flinch 1 -> 0, death fall 0..1.
    pub(crate) ads: f32,
    pub(crate) kick: f32,
    pub(crate) hit: f32,
    death: f32,
    /// Character montage: clip, seconds into it, blend in / out (None = idle); `montage_w` = its current blend weight 0..1.
    montage: Option<(String, f32, (f32, f32))>,
    pub(crate) montage_w: f32,
    /// Event counters of CharacterState already consumed (fired, hits, montage).
    seen: (u32, u32, u32),
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

/// Per body copy: smoothed locomotion weights, event layers (shots, hits, montages), death pose.
fn anim_state(time: Res<Time>, durs: Res<ClipDurs>, mut roots: Query<(&ChildOf, &mut BodyAnim, &mut Transform), With<BodyRoot>>, chars: Query<(&CharacterState, &Transform), Without<BodyRoot>>) {
    let dt = time.delta_secs().min(0.1);
    for (co, mut st, mut rt) in &mut roots {
        let Ok((p, tf)) = chars.get(co.parent()) else { continue };
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
            dsum += w * durs.0.get(*n).copied().unwrap_or(1.0);
            wsum += w;
        }
        if wsum > 0.01 {
            st.phase = (st.phase + dt * rate.clamp(0.0, 3.0) * wsum / dsum).rem_euclid(1.0);
        }
        // Event layers: counters in CharacterState tick up per shot / hit / montage.
        let seen = st.seen;
        if p.fired != seen.0 {
            st.kick = 1.0;
        }
        if p.hits != seen.1 {
            st.hit = 1.0;
        }
        if p.montage != seen.2 && !p.clip.is_empty() {
            st.montage = Some((p.clip.clone(), 0.0, p.blend));
        }
        st.seen = (p.fired, p.hits, p.montage);
        st.kick *= (-dt * 25.0).exp();
        st.hit *= (-dt * 5.0).exp();
        st.ads = crate::status::finterp_to(st.ads, p.aiming as i32 as f32, dt, 12.0);
        // Clip not retargeted yet (its glb still loading): wait up to 10 s at weight 0.
        st.montage = st.montage.take().map(|(c, t, b)| (c, t + dt, b)).filter(|(c, t, _)| *t < durs.0.get(c).copied().unwrap_or(10.0));
        st.montage_w = st.montage.as_ref().map_or(0.0, |(c, t, b)| durs.0.get(c).map_or(0.0, |&l| montage_weight(*t, l, *b)));
        st.death = crate::status::finterp_to(st.death, p.dead as i32 as f32, dt, 3.0);
        // Death: fall backwards about the feet and lie on the back (ponytail: no ragdoll / death montage).
        *rt = Transform { rotation: Quat::from_rotation_x(std::f32::consts::FRAC_PI_2 * st.death), translation: Vec3::Y * 0.12 * st.death, ..default() };
    }
}

/// FAlphaBlend in / out of a montage `t` s into a clip of length `len`.
fn montage_weight(t: f32, len: f32, (bin, bout): (f32, f32)) -> f32 {
    (t / bin.max(1e-3)).min((len - t) / bout.max(1e-3)).clamp(0.0, 1.0)
}

/// Montage clip name -> node in the shared body graph.
#[derive(Resource, Default)]
pub(crate) struct MontageNodes(HashMap<String, AnimationNodeIndex>);

/// Mask group of the bones the UpperBody slot leaves to locomotion: everything not under spine_01, except the ik_hand_* bones the
/// gun hangs on.
const LOWER_BODY: u32 = 0;

/// Retargets each requested montage clip (A_FP_* in arms.glb / arms_anims/*.glb, loaded by viewmodel.rs; same ALS skeleton, glTF
/// root node `arms` instead of `body`) onto the body's joint paths and adds it to the shared graph, masked to the upper body.
fn montage_clips(
    mut nodes: ResMut<MontageNodes>,
    mut durs: ResMut<ClipDurs>,
    gltfs: Res<Assets<Gltf>>,
    mut clips: ResMut<Assets<AnimationClip>>,
    mut graphs: ResMut<Assets<AnimationGraph>>,
    chars: Query<&CharacterState>,
    roots: Query<(&BodyRoot, &AnimationGraphHandle)>,
    joints: Query<(&Name, &AnimationTargetId)>,
    parents: Query<&ChildOf>,
) {
    let Some((r, gh)) = roots.iter().find(|(r, _)| !r.bones.is_empty()) else { return };
    for p in &chars {
        if p.clip.is_empty() || nodes.0.contains_key(&p.clip) {
            continue;
        }
        let Some(src) = gltfs.iter().find_map(|(_, g)| g.named_animations.get(p.clip.as_str())).and_then(|h| clips.get(h)) else { continue };
        let mut map = HashMap::new();
        let mut lower = vec![];
        for &e in r.bones.values() {
            let Ok((_, &id)) = joints.get(e) else { continue };
            // joint path below the glTF root node (whose name differs), e.g. root/pelvis/spine_01
            let (mut path, mut x, mut upper) = (vec![], e, false);
            while let Ok((n, _)) = joints.get(x)
                && n.as_str() != "body"
            {
                upper |= n.as_str() == "spine_01" || n.as_str().starts_with("ik_hand");
                path.push(n.clone());
                x = parents.get(x).map_or(x, |c| c.parent());
            }
            path.push(Name::new("arms"));
            path.reverse();
            map.insert(AnimationTargetId::from_names(path.iter()), id);
            if !upper {
                lower.push(id);
            }
        }
        let mut clip = AnimationClip::default();
        for (id, curves) in src.curves() {
            for c in curves.iter().filter(|_| map.contains_key(id)) {
                clip.add_variable_curve_to_target(map[id], c.clone());
            }
        }
        clip.set_duration(src.duration());
        debug!("body montage {}: {} of {} tracks retargeted", p.clip, clip.curves().len(), src.curves().len());
        durs.0.insert(p.clip.clone(), src.duration().max(0.01));
        let Some(mut g) = graphs.get_mut(&gh.0) else { return };
        for id in lower {
            g.add_target_to_mask_group(id, LOWER_BODY);
        }
        let top = g.root;
        let n = g.add_clip_with_mask(clips.add(clip), 1 << LOWER_BODY, 1.0, top);
        nodes.0.insert(p.clip.clone(), n);
    }
}

pub(crate) fn drive(
    durs: Res<ClipDurs>,
    montages: Res<MontageNodes>,
    roots: Query<(&BodyRoot, &BodyAnim)>,
    mut players: Query<(Entity, &mut AnimationPlayer)>,
    parents: Query<&ChildOf>,
    guns: Query<(), With<CharGun>>,
) {
    for (pe, mut ap) in &mut players {
        if under_gun(pe, &parents, &guns) {
            continue;
        }
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
        let Some((r, st)) = root.and_then(|r| roots.get(r).ok()) else { continue };
        let land_clip = if st.land_heavy && durs.0.contains_key("ALS_N_Land_Heavy") { "ALS_N_Land_Heavy" } else { "ALS_N_Land_Light" };
        let land_dur = durs.0.get(land_clip).copied().unwrap_or(0.5);
        let land = if st.land_t < land_dur { (st.land_t / 0.06).min(1.0) * ((land_dur - st.land_t) / 0.25).min(1.0) } else { 0.0 };
        for (name, &node) in &r.nodes {
            let is_land = name.starts_with("ALS_N_Land");
            let w = if is_land { if name == land_clip { land } else { 0.0 } } else { st.w.get(name.as_str()).copied().unwrap_or(0.0) * (1.0 - land) };
            if ap.animation_mut(node).is_none() {
                ap.play(node).repeat().set_weight(0.0);
            }
            let a = ap.animation_mut(node).unwrap();
            a.set_weight(w);
            let dur = durs.0.get(name).copied().unwrap_or(1.0);
            if name.contains("Walk") || name.contains("Run") {
                a.set_speed(0.0).set_seek_time(st.phase * dur);
            } else if is_land {
                a.set_speed(0.0).set_seek_time(st.land_t.min(dur));
            }
        }
        // Montage over the locomotion: weights blend as a weighted mean (locomotion sums to ~1), so w / (1 - w) gives it share w.
        for (name, &node) in &montages.0 {
            let on = st.montage.as_ref().filter(|m| &m.0 == name);
            if ap.animation_mut(node).is_none() {
                ap.play(node).set_weight(0.0);
            }
            let a = ap.animation_mut(node).unwrap();
            let w = on.map_or(0.0, |_| st.montage_w.min(0.999));
            a.set_weight(w / (1.0 - w));
            a.set_speed(0.0).set_seek_time(on.map_or(0.0, |m| m.1));
        }
    }
}

/// HideBoneByName: collapse the bone (and, through the hierarchy, everything under it); then ALS spine rotation (aim pitch on the
/// world copies, hit flinch) and lean roll. Axes come from last frame's bone GlobalTransform (the pose is about to be propagated).
pub(crate) fn hide_bones(
    det: Res<Detached>,
    chars: Query<(&CharacterState, &GlobalTransform), Without<BodyRoot>>,
    mut joints: Query<(Entity, &Name, &mut Transform, &GlobalTransform), (With<AnimationTargetId>, Without<BodyRoot>)>,
    parents: Query<&ChildOf>,
    roots: Query<(&BodyRoot, &BodyAnim, &ChildOf)>,
    root_only: Query<&BodyRoot>,
    guns: Query<(), With<CharGun>>,
) {
    for (e, n, mut t, g) in &mut joints {
        let name = n.as_str();
        if !(SPINE.contains(&name) || name == "neck_01" || HIDDEN_BONES.contains(&name)) {
            continue;
        }
        let Some((r, an, co)) = root_of(e, &parents, &root_only).and_then(|r| roots.get(r).ok()) else { continue };
        if under_gun(e, &parents, &guns) {
            continue;
        }
        let Ok((p, pg)) = chars.get(co.parent()) else { continue };
        if HIDDEN_BONES.contains(&name) {
            // the camera is at the owner's head: the FP arms replace the body's
            t.scale = if r.local && !det.0 && (if r.world { &SHADOW_HIDDEN_BONES[..] } else { &HIDDEN_BONES[..] }).contains(&name) { Vec3::ZERO } else { Vec3::ONE };
        }
        // ponytail: not UE (its owner mesh is camera-attached at the head); the camera here sits in front of the chest, so the
        // owner copy's torso (spine_01 and up) is scaled away as the view tips down: full at -65 deg, untouched above -40 deg.
        if name == "spine_01" {
            let k = if !r.world && r.local && !det.0 { ((p.pitch.to_degrees() + 40.0) / -25.0).clamp(0.0, 1.0) } else { 0.0 };
            t.scale = Vec3::splat((1.0 - k * k * (3.0 - 2.0 * k)).max(0.001)); // 0 panics the skinned bounds
        }
        // Owner copy stays upright (the chest would swing into the camera, which is not parented to the head).
        let pitch = if r.world { p.pitch } else { 0.0 };
        let (share, flinch, lean) = if SPINE.contains(&name) {
            (SPINE_SHARE / 3.0, an.hit * 0.25 / 3.0, -p.lean * LEAN_ROLL_DEG.to_radians() / 3.0)
        } else if name == "neck_01" {
            (NECK_SHARE + HEAD_SHARE, 0.0, 0.0) // ponytail: the head follows the neck bone (no separate head pass)
        } else {
            continue;
        };
        let (right, fwd) = (pg.right().as_vec3(), pg.forward().as_vec3());
        let inv = g.rotation().inverse();
        t.rotation = t.rotation * Quat::from_axis_angle((inv * right).normalize(), pitch * share - flinch) * Quat::from_axis_angle((inv * fwd).normalize(), lean);
    }
}

#[cfg(test)]
mod tests {
    use super::{montage_weight, targets};
    #[test]
    fn montage_fades() {
        let b = (0.25, 0.5);
        assert_eq!(montage_weight(0.0, 3.0, b), 0.0);
        assert!((montage_weight(0.125, 3.0, b) - 0.5).abs() < 1e-5);
        assert_eq!(montage_weight(1.0, 3.0, b), 1.0);
        assert!((montage_weight(2.75, 3.0, b) - 0.5).abs() < 1e-5);
        assert_eq!(montage_weight(3.0, 3.0, b), 0.0);
    }
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
