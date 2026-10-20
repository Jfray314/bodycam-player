//! Death ragdoll: UE WW_RagdollSystem simulates the character Mesh with AnimMan_PhysicsAsset when the player dies
//! (EventRagdollStart). Port: assets/ragdoll.json (tools/ragdoll.py: the asset's 18 bodies + joint limits, converted to
//! body.glb bone space) becomes rapier bodies + joints spawned at the bones' poses on the death frame; each frame the bodies'
//! poses are written back onto the bones (after animation and IK, before transform propagation). Map meshes near the corpse
//! get trimesh colliders on demand. The local player's camera follows the ragdoll's neck (effects.rs).
use crate::body::BodyRoot;
use crate::character::CharacterState;
use bevy::animation::AnimationTargetId;
use bevy::math::Affine3A;
use bevy::prelude::*;
use bevy_rapier3d::prelude::*;
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Shape {
    kind: String,
    center: [f32; 3],
    rot: [f32; 4],
    #[serde(default)]
    radius: f32,
    #[serde(default)]
    half_len: f32,
    #[serde(default)]
    half: [f32; 3],
}
#[derive(Deserialize)]
struct BodyDef {
    bone: String,
    shapes: Vec<Shape>,
}
#[derive(Deserialize)]
struct JointDef {
    child: String,
    parent: String,
    pos1: [f32; 3],
    pri1: [f32; 3],
    sec1: [f32; 3],
    pos2: [f32; 3],
    pri2: [f32; 3],
    sec2: [f32; 3],
    twist: Option<f32>,
    swing1: Option<f32>,
    swing2: Option<f32>,
}
#[derive(Deserialize)]
struct Def {
    bodies: Vec<BodyDef>,
    joints: Vec<JointDef>,
}

/// Ragdoll bodies collide with the map, not with each other (UE only disables jointed pairs).
// ponytail: whole-ragdoll self-collision off; per-pair filtering if limbs pass through each other visibly.
const RAGDOLL: Group = Group::GROUP_2;
/// Map trimeshes within this distance (m) of the corpse get colliders.
const MAP_RADIUS: f32 = 6.0;
// ponytail: no shot direction reaches here yet (CharacterState carries no hit vector); UE ApplyPointDamage pushes the hit
// bone along ShotFromDirection * RagdollForce. A random horizontal knock on the chest stands in.
const KNOCK: f32 = 25.0; // N s

pub struct RagdollPlugin;
impl Plugin for RagdollPlugin {
    fn build(&self, app: &mut App) {
        let def: Def = serde_json::from_str(include_str!("../../assets/ragdoll.json")).expect("ragdoll.json");
        app.add_plugins(RapierPhysicsPlugin::<NoUserData>::default())
            .insert_resource(DefRes(def))
            .add_systems(Update, (start_stop, map_colliders).chain())
            .add_systems(
                PostUpdate,
                writeback
                    .after(PhysicsSet::Writeback)
                    .after(crate::bodyik::apply_ik)
                    .after(crate::body::hide_bones)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

#[derive(Resource)]
struct DefRes(Def);

/// On a BodyRoot (world copy) while its character is dead: bone name -> body entity.
#[derive(Component)]
pub struct Ragdoll {
    pub bodies: HashMap<String, Entity>,
    floor: Entity,
}

/// A ragdoll body: the bone it drives (rapier bodies are unscaled; the bone keeps its own scale).
#[derive(Component)]
struct Part {
    bone: Entity,
}

/// Map mesh that already has its baked collider.
#[derive(Component)]
struct HasCollider;

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::from_array(a)
}

/// Joint frame from UE's primary (twist, X) and secondary (Y) axes, already mirrored to glb space.
fn basis(pri: [f32; 3], sec: [f32; 3]) -> Quat {
    let x = v3(pri).normalize();
    let z = x.cross(v3(sec)).normalize();
    Quat::from_mat3(&Mat3::from_cols(x, z.cross(x), z))
}

fn start_stop(
    mut cmd: Commands,
    def: Res<DefRes>,
    chars: Query<&CharacterState>,
    roots: Query<(Entity, &BodyRoot, &ChildOf, Option<&Ragdoll>)>,
    globals: Query<&GlobalTransform>,
    locals: Query<(&Transform, &ChildOf)>,
) {
    for (root, r, co, rag) in &roots {
        let Ok(st) = chars.get(co.parent()) else { continue };
        if !r.world {
            continue;
        }
        match (st.dead, rag) {
            (true, None) => {
                // FP-hidden bones (arms on the world copy) are still scaled to 0 on the death frame (collapsed onto the
                // shoulder): wait until hide_bones has shown them. The own corpse's head stays hidden (death cam).
                let hidden = def.0.bodies.iter().filter(|b| b.bone != "head").filter_map(|b| r.bones.get(&b.bone)).filter_map(|&e| globals.get(e).ok());
                if hidden.into_iter().any(|g| g.to_scale_rotation_translation().0.min_element() < 0.5) {
                    continue;
                }
                let mut bodies = HashMap::new();
                let knock = {
                    let a = rand::random::<f32>() * std::f32::consts::TAU;
                    Vec3::new(a.cos(), 0.0, a.sin()) * KNOCK * (0.3 + 0.7 * rand::random::<f32>())
                };
                for b in &def.0.bodies {
                    let (Some(&bone), true) = (r.bones.get(&b.bone), b.bone != "root") else { continue };
                    let Ok(g) = globals.get(bone) else { continue };
                    // Rotation via the parent: the own corpse's head is scaled to 0 (death cam), its global rotation is garbage.
                    let pos = g.translation();
                    let rot = locals
                        .get(bone)
                        .ok()
                        .and_then(|(t, co)| globals.get(co.parent()).ok().map(|pg| pg.to_scale_rotation_translation().1 * t.rotation))
                        .unwrap_or_else(|| g.to_scale_rotation_translation().1);
                    let shapes = b
                        .shapes
                        .iter()
                        .map(|s| {
                            let c = match s.kind.as_str() {
                                "capsule" => Collider::capsule_z(s.half_len, s.radius),
                                "box" => Collider::cuboid(s.half[0], s.half[1], s.half[2]),
                                _ => Collider::ball(s.radius),
                            };
                            (v3(s.center), Quat::from_array(s.rot), c)
                        })
                        .collect();
                    let mut e = cmd.spawn((
                        RigidBody::Dynamic,
                        Collider::compound(shapes),
                        ColliderMassProperties::Density(1000.0),
                        CollisionGroups::new(RAGDOLL, Group::ALL ^ RAGDOLL),
                        Damping { linear_damping: 0.1, angular_damping: 0.8 },
                        Velocity { linear: st.vel, angular: Vec3::ZERO },
                        Ccd::enabled(),
                        Transform::from_translation(pos).with_rotation(rot),
                        Part { bone },
                    ));
                    if b.bone == "spine_03" {
                        e.insert(ExternalImpulse { impulse: knock, torque_impulse: Vec3::ZERO });
                    }
                    bodies.insert(b.bone.clone(), e.id());
                }
                for j in &def.0.joints {
                    let (Some(&child), Some(&parent)) = (bodies.get(&j.child), bodies.get(&j.parent)) else { continue };
                    let mut mask = JointAxesMask::LIN_AXES;
                    let mut joint = GenericJointBuilder::new(JointAxesMask::LIN_AXES)
                        // ImpulseJoint frame 1 = parent (UE Bone2 / Pos2), frame 2 = child (UE Bone1 / Pos1)
                        .local_anchor1(v3(j.pos2))
                        .local_basis1(basis(j.pri2, j.sec2))
                        .local_anchor2(v3(j.pos1))
                        .local_basis2(basis(j.pri1, j.sec1));
                    for (axis, lim, m) in [(JointAxis::AngX, j.twist, JointAxesMask::ANG_X), (JointAxis::AngZ, j.swing1, JointAxesMask::ANG_Z), (JointAxis::AngY, j.swing2, JointAxesMask::ANG_Y)] {
                        match lim {
                            Some(0.0) => mask |= m,
                            // ponytail: per-axis limits approximate UE's swing cone; rapier's angle limits go singular at 90 deg
                            Some(a) => joint = joint.limits(axis, [-a.min(1.4), a.min(1.4)]),
                            None => {}
                        }
                    }
                    let joint = joint.locked_axes(mask).contacts_enabled(false).build();
                    cmd.entity(child).insert(ImpulseJoint::new(parent, joint));
                }
                // Floor at the feet: the test map has no map colliders.
                let feet = globals.get(root).map(|g| g.translation()).unwrap_or_default();
                let floor = cmd.spawn((Collider::cuboid(50.0, 0.05, 50.0), Transform::from_translation(feet - Vec3::Y * 0.05))).id();
                cmd.entity(root).insert(Ragdoll { bodies, floor });
            }
            (false, Some(rag)) => {
                for &e in rag.bodies.values() {
                    cmd.entity(e).despawn();
                }
                cmd.entity(rag.floor).despawn();
                cmd.entity(root).remove::<Ragdoll>();
            }
            _ => {}
        }
    }
}

/// Trimesh colliders for the map meshes around each ragdoll (built once per mesh, kept).
fn map_colliders(
    mut cmd: Commands,
    rags: Query<&GlobalTransform, With<Ragdoll>>,
    geo: Query<(Entity, &Mesh3d, &GlobalTransform, &bevy::camera::primitives::Aabb), (With<crate::map::MapGeometry>, Without<HasCollider>)>,
    meshes: Res<Assets<Mesh>>,
) {
    for at in &rags {
        let c = at.translation();
        for (e, m, g, aabb) in &geo {
            let centre = g.transform_point(aabb.center.into());
            let half = (g.affine().matrix3.abs() * Vec3::from(aabb.half_extents)).length();
            if centre.distance(c) - half > MAP_RADIUS {
                continue;
            }
            cmd.entity(e).insert(HasCollider);
            // World transform baked into the vertices: map meshes can be mirrored (negative scale), which a scaled rapier
            // collider turns into NaN contacts.
            if let Some(col) = meshes.get(&m.0).and_then(|mesh| baked_trimesh(mesh, g)) {
                cmd.spawn((col, RigidBody::Fixed, Transform::IDENTITY));
            }
        }
    }
}

fn baked_trimesh(mesh: &Mesh, g: &GlobalTransform) -> Option<Collider> {
    let verts: Vec<Vec3> = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?.as_float3()?.iter().map(|&p| g.transform_point(p.into())).collect();
    let idx: Vec<u32> = mesh.indices()?.iter().map(|i| i as u32).collect();
    let tris = idx.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect();
    Collider::trimesh(verts, tris).ok()
}

/// Bodies -> bones. A bone's new local = (parent bone's pose this frame)^-1 * body pose; the parent's pose is its body when it
/// has one, else its last-frame GlobalTransform (root: the corpse doesn't move, so that is exact).
fn writeback(
    rags: Query<&Ragdoll>,
    parts: Query<(&Transform, &Part), Without<AnimationTargetId>>,
    mut bones: Query<(&mut Transform, &ChildOf), With<AnimationTargetId>>,
    globals: Query<&GlobalTransform>,
) {
    for rag in &rags {
        let pose: HashMap<Entity, Affine3A> = rag
            .bodies
            .values()
            .filter_map(|&e| parts.get(e).ok())
            .filter(|(t, _)| t.translation.is_finite() && t.rotation.is_finite())
            .map(|(t, p)| (p.bone, Affine3A::from_rotation_translation(t.rotation, t.translation)))
            .collect();
        for (&bone, &world) in &pose {
            let Ok((_, co)) = bones.get(bone) else { continue };
            let parent = pose.get(&co.parent()).copied().or_else(|| {
                globals.get(co.parent()).ok().map(|g| {
                    let (_, r, t) = g.to_scale_rotation_translation();
                    Affine3A::from_rotation_translation(r, t)
                })
            });
            if let (Some(p), Ok((mut t, _))) = (parent, bones.get_mut(bone)) {
                let (_, r, tr) = (p.inverse() * world).to_scale_rotation_translation();
                (t.rotation, t.translation) = (r, tr);
            }
        }
    }
}
