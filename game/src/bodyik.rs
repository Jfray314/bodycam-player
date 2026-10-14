//! Body IK on the ALS mannequin skeleton: ALS foot IK (trace down from each foot, lift/lower the feet, drop the pelvis to the lower
//! one: AnimBP SetFootOffsets / SetPelvisIKOffset) and third-person arms (two-bone IK of the body's own arms onto the FP arms'
//! hands, which are placed relative to the gun by the FP animation, so the body holds the current gun in any pose/montage).
//! Poses are computed from the local Transforms the animation just wrote (GlobalTransforms are last frame's), in BodyRoot space.
use crate::body::{BodyRoot, ThirdPerson};
use crate::player::{IgnoreTraces, Player};
use bevy::animation::AnimationTargetId;
use bevy::math::Affine3A;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;

/// Foot offsets (m, + = ground above the player's feet plane) and pelvis drop, smoothed (AnimBP FInterpTo 15 / 10).
#[derive(Resource, Default)]
struct FootIk {
    off: [f32; 2],
    pelvis: f32,
}
const FOOT_RANGE: f32 = 0.45;

pub struct IkPlugin;
impl Plugin for IkPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FootIk>()
            .add_systems(Update, (fill_bones, foot_trace).chain().after(crate::body::drive))
            .add_systems(PostUpdate, (apply_ik, fp_arms_visibility).after(crate::body::hide_bones).before(bevy::transform::TransformSystems::Propagate));
    }
}

fn fill_bones(mut roots: Query<(Entity, &mut BodyRoot)>, joints: Query<(Entity, &Name), With<AnimationTargetId>>, parents: Query<&ChildOf>) {
    for (r, mut root) in &mut roots {
        if !root.bones.is_empty() {
            continue;
        }
        for (e, n) in &joints {
            if crate::viewmodel::root_of_entity(e, r, &parents) {
                root.bones.insert(n.to_string(), e);
            }
        }
    }
}

fn foot_trace(
    time: Res<Time>,
    mut ik: ResMut<FootIk>,
    player: Single<(&Player, &GlobalTransform)>,
    roots: Query<&BodyRoot>,
    globals: Query<&GlobalTransform>,
    ignore: Query<(), With<IgnoreTraces>>,
    mut ray: MeshRayCast,
) {
    let (p, pg) = *player;
    let Some(owner) = roots.iter().find(|r| !r.shadow) else { return };
    let dt = time.delta_secs().min(0.1);
    let filter = |e: Entity| !ignore.contains(e);
    let settings = MeshRayCastSettings::default().with_filter(&filter);
    let feet_y = pg.translation().y;
    let mut target = [0.0f32; 2];
    for (i, n) in ["foot_l", "foot_r"].iter().enumerate() {
        let Some(g) = owner.bones.get(*n).and_then(|e| globals.get(*e).ok()) else { return };
        if !p.grounded {
            continue;
        }
        let o = Vec3::new(g.translation().x, feet_y + 0.7, g.translation().z);
        if let Some((_, h)) = ray.cast_ray(Ray3d::new(o, Dir3::NEG_Y), &settings).first() {
            let off = o.y - h.distance - feet_y;
            if off.abs() <= FOOT_RANGE {
                target[i] = off;
            }
        }
    }
    for i in 0..2 {
        ik.off[i] = crate::status::finterp_to(ik.off[i], target[i], dt, 15.0);
    }
    ik.pelvis = crate::status::finterp_to(ik.pelvis, ik.off[0].min(ik.off[1]).min(0.0), dt, 10.0);
}

type Bones<'w, 's> = Query<'w, 's, (&'static mut Transform, &'static ChildOf)>;

/// Pose (translation, rotation) of `e` in `root` space (root's own transform excluded), from the local Transforms.
fn pose(mut e: Entity, root: Entity, q: &Bones) -> (Vec3, Quat) {
    let mut m = Affine3A::IDENTITY;
    while e != root {
        let Ok((t, c)) = q.get(e) else { break };
        m = t.compute_affine() * m;
        e = c.parent();
    }
    let (_, r, t) = m.to_scale_rotation_translation();
    (t, r)
}

fn parent_rot(e: Entity, root: Entity, q: &Bones) -> Quat {
    q.get(e).map_or(Quat::IDENTITY, |(_, c)| pose(c.parent(), root, q).1)
}

/// Rotates bone `e` by `delta` (a rotation in root space) about its own origin.
fn rotate(e: Entity, root: Entity, delta: Quat, q: &mut Bones) {
    let p = parent_rot(e, root, q);
    if let Ok((mut t, _)) = q.get_mut(e) {
        t.rotation = p.inverse() * delta * p * t.rotation;
    }
}

/// Sets bone `e`'s root-space rotation.
fn set_rot(e: Entity, root: Entity, rot: Quat, q: &mut Bones) {
    let p = parent_rot(e, root, q);
    if let Ok((mut t, _)) = q.get_mut(e) {
        t.rotation = p.inverse() * rot;
    }
}

fn shift(e: Entity, root: Entity, v: Vec3, q: &mut Bones) {
    let p = parent_rot(e, root, q);
    if let Ok((mut t, _)) = q.get_mut(e) {
        t.translation += p.inverse() * v;
    }
}

/// Two-bone IK: bring the end of chain [a, b, c] to `target` (root space), bending the middle joint toward `pole`.
/// `end_rot`: the end bone's wanted root-space rotation (None = keep what the animation gave it).
fn two_bone(root: Entity, [a, b, c]: [Entity; 3], target: Vec3, pole: Vec3, end_rot: Option<Quat>, q: &mut Bones) {
    let (pa, _) = pose(a, root, q);
    let (pb, _) = pose(b, root, q);
    let (pc, rc) = pose(c, root, q);
    let (l1, l2) = ((pb - pa).length(), (pc - pb).length());
    let to = target - pa;
    let d = to.length().clamp(0.01, (l1 + l2) * 0.999);
    let u = to.normalize_or_zero();
    if u == Vec3::ZERO || l1 < 1e-4 || l2 < 1e-4 {
        return;
    }
    let x = (l1 * l1 - l2 * l2 + d * d) / (2.0 * d);
    let h = (l1 * l1 - x * x).max(0.0).sqrt();
    let n = (pole - u * pole.dot(u)).try_normalize().unwrap_or(Vec3::NEG_Z);
    let pb2 = pa + u * x + n * h;
    rotate(a, root, Quat::from_rotation_arc((pb - pa).normalize(), (pb2 - pa).normalize()), q);
    let (pb2, _) = pose(b, root, q);
    let (pc1, _) = pose(c, root, q);
    let end = pa + u * d;
    rotate(b, root, Quat::from_rotation_arc((pc1 - pb2).normalize(), (end - pb2).normalize()), q);
    set_rot(c, root, end_rot.unwrap_or(rc), q);
}

fn apply_ik(
    ik: Res<FootIk>,
    tp: Res<ThirdPerson>,
    roots: Query<(Entity, &BodyRoot, &GlobalTransform)>,
    mut q: Bones,
    fp: Query<(Entity, &Name, &GlobalTransform)>,
    arms: Query<Entity, With<crate::viewmodel::ArmsRoot>>,
    parents: Query<&ChildOf>,
) {
    let hands: Vec<(String, GlobalTransform)> = match (tp.on, arms.single()) {
        (true, Ok(ar)) => fp.iter().filter(|(e, n, _)| matches!(n.as_str(), "hand_r" | "hand_l") && crate::viewmodel::root_of_entity(*e, ar, &parents)).map(|(_, n, g)| (n.to_string(), *g)).collect(),
        _ => vec![],
    };
    for (root, r, rg) in &roots {
        let b = |n: &str| r.bones.get(n).copied();
        if (ik.off.iter().any(|o| o.abs() >= 0.003) || ik.pelvis.abs() >= 0.003)
            && let (Some(pelvis), Some(tl), Some(cl), Some(fl), Some(tr), Some(cr), Some(fr)) = (b("pelvis"), b("thigh_l"), b("calf_l"), b("foot_l"), b("thigh_r"), b("calf_r"), b("foot_r"))
        {
            let wanted = [fl, fr].map(|f| pose(f, root, &q).0);
            shift(pelvis, root, Vec3::Y * ik.pelvis, &mut q);
            for (i, chain) in [[tl, cl, fl], [tr, cr, fr]].into_iter().enumerate() {
                two_bone(root, chain, wanted[i] + Vec3::Y * ik.off[i], Vec3::NEG_Z, None, &mut q);
            }
        }
        // Third-person arms: hands to the FP arms' hands (gun-relative placement from the FP animation), elbows down and out.
        let inv = rg.affine().inverse();
        for (name, g) in &hands {
            let side = if name.ends_with('r') { "r" } else { "l" };
            let (Some(u), Some(l), Some(h)) = (b(&format!("upperarm_{side}")), b(&format!("lowerarm_{side}")), b(&format!("hand_{side}"))) else { continue };
            let m = inv * g.affine();
            let (_, rot, t) = m.to_scale_rotation_translation();
            let sx = if side == "r" { 1.0 } else { -1.0 };
            two_bone(root, [u, l, h], t, Vec3::new(sx * 0.6, -1.0, 0.3), Some(rot), &mut q);
        }
    }
}

/// Third-person view: hide the FP arms' skin (the body's own arms hold the gun); the gun stays.
fn fp_arms_visibility(
    tp: Res<ThirdPerson>,
    arms: Query<Entity, With<crate::viewmodel::ArmsRoot>>,
    mut meshes: Query<(Entity, &mut Visibility), With<Mesh3d>>,
    parents: Query<&ChildOf>,
    guns: Query<Entity, With<crate::viewmodel::GunRoot>>,
    mut last: Local<Option<bool>>,
) {
    if *last == Some(tp.on) {
        return;
    }
    let Ok(ar) = arms.single() else { return };
    *last = Some(tp.on);
    for (e, mut v) in &mut meshes {
        if crate::viewmodel::root_of_entity(e, ar, &parents) && !guns.iter().any(|g| crate::viewmodel::root_of_entity(e, g, &parents)) {
            *v = if tp.on { Visibility::Hidden } else { Visibility::Inherited };
        }
    }
}
