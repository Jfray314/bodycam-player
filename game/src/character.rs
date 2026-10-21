//! Third-person character, driven purely by replicable state, for every camera that is not the owner's FP view (drone camera, 3P
//! orbit, and the remote players networking spawns).
//!   let e = spawn_character(&mut cmd, Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)), CharacterState::default());
//! Afterwards the net layer only writes the entity's `Transform` (feet position + yaw) and `CharacterState` (aim pitch, velocity,
//! stance, lean, equipped DT_WeaponItems row, aiming, event counters). body.rs builds the ALS body from it (locomotion blend, spine /
//! head aim pitch, lean, hit flinch, death fall, shot kick, montages), this file the gun in the hands (grips measured from the FP arms
//! in assets/grips.json via BODYCAM_GRIPDUMP=1) and bodyik.rs the IK that puts both hands on it.
//! The local player is the same thing: its entity gets a CharacterState synced from `Player`/`Inventory`, plus the owner copy.
//! Dev: BODYCAM_PUPPET="dx:dz:dyaw_deg" spawns a puppet mirroring the local player's state and motion 1 s late at that offset
//! (default 1.5:-2.5:180: in front, facing the player); with BODYCAM_3P the orbit camera circles the puppet.
use crate::player::Player;
use crate::status::PlayerDamage;
use crate::viewmodel::{ArmsRoot, GunRoot, ads_axes, gun_model, root_of_entity};
use crate::weapon::{Fired, Inventory, MontageStarted};
use bevy::prelude::*;
use bevy::world_serialization::{WorldAsset, WorldAssetRoot};
use std::collections::{HashMap, VecDeque};
use std::sync::OnceLock;

/// Everything another player needs to draw this character (position/yaw are the entity's Transform).
#[derive(Component, Clone, Debug)]
pub struct CharacterState {
    /// Aim pitch (rad, + = looking up): ALS AimOffset / spine + head bend, gun elevation.
    pub pitch: f32,
    /// World velocity (m/s): locomotion blend direction and speed relative to the facing.
    pub vel: Vec3,
    pub grounded: bool,
    pub crouched: bool,
    pub sprinting: bool,
    /// -1 left .. 1 right.
    pub lean: f32,
    /// Equipped DT_WeaponItems row ("" = nothing in hand).
    pub item: String,
    pub aiming: bool,
    /// Event counters: bump `fired` per shot, `hits` per hit taken, `montage` per character montage (`clip` = its A_FP_* clip,
    /// `blend` = montage BlendIn/BlendOut s). UE multicasts the same FP montage onto the full-body Mesh (AC_BodycamAnimationComponent
    /// Multicast PlayCharacterWeaponMontage, slot UpperBody), so other players see the owner's reload / mag check / draw ... 1:1.
    pub fired: u32,
    pub hits: u32,
    pub montage: u32,
    pub clip: String,
    pub blend: (f32, f32),
    pub dead: bool,
    /// Last shot's ragdoll impulse (N s), world hit point, time (s): WW_RagdollSystem PendingImpulse, applied on Ragdoll Start
    /// within 0.5 s (ClearPendingImpulse timer).
    pub push: Option<(Vec3, Vec3, f32)>,
}
impl Default for CharacterState {
    fn default() -> Self {
        Self { pitch: 0.0, vel: Vec3::ZERO, grounded: true, crouched: false, sprinting: false, lean: 0.0, item: String::new(), aiming: false, fired: 0, hits: 0, montage: 0, clip: String::new(), blend: (0.25, 0.25), dead: false, push: None }
    }
}

/// Spawns a remote-style character (full body copy only). The body appears once models/body.glb has loaded.
pub fn spawn_character(cmd: &mut Commands, tf: Transform, state: CharacterState) -> Entity {
    cmd.spawn((tf, Visibility::default(), state, NeedsBody { local: false })).id()
}

/// body.rs builds the BodyRoot copies for entities with this.
#[derive(Component)]
pub struct NeedsBody {
    pub local: bool,
}

/// The gun in a world copy's hands: `joint` = its `root` bone (found once the scene spawned).
#[derive(Component)]
pub struct CharGun {
    pub model: &'static str,
    pub joint: Option<Entity>,
}

#[derive(Component)]
struct MuzzleFlash(Entity);

/// Test puppet (BODYCAM_PUPPET).
#[derive(Component)]
pub struct Puppet;

pub struct CharacterPlugin;
impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (mark_local, sync_local, sync_guns, gun_joints, flash, death_scream).chain());
        if let Ok(v) = std::env::var("BODYCAM_PUPPET") {
            let n: Vec<f32> = v.split(':').filter_map(|x| x.parse().ok()).collect();
            let off = Vec3::new(n.first().copied().unwrap_or(0.0), 0.0, n.get(1).copied().unwrap_or(-2.5));
            let dyaw = n.get(2).copied().unwrap_or(180.0).to_radians();
            app.insert_resource(PuppetCfg { off, dyaw }).init_resource::<Feed>().add_systems(Startup, puppet_spawn).add_systems(Update, puppet_feed);
        }
        if std::env::var("BODYCAM_GRIPDUMP").is_ok() {
            app.add_systems(Update, grip_dump);
        }
    }
}

// ---- the local player is a character too ----

fn mark_local(mut cmd: Commands, p: Query<Entity, (With<Player>, Without<CharacterState>)>) {
    for e in &p {
        cmd.entity(e).insert((CharacterState::default(), NeedsBody { local: true }));
    }
}

fn sync_local(
    mut p: Query<(&Player, Option<&Inventory>, &mut CharacterState)>,
    mut fired: MessageReader<Fired>,
    mut montages: MessageReader<MontageStarted>,
    mut dmg: MessageReader<PlayerDamage>,
    time: Res<Time>,
) {
    let Ok((pl, inv, mut s)) = p.single_mut() else { return };
    s.pitch = pl.pitch;
    s.vel = pl.vel;
    (s.grounded, s.crouched, s.sprinting, s.lean, s.aiming, s.dead) = (pl.grounded, pl.crouched, pl.sprinting, pl.lean, pl.aiming, pl.dead);
    if let Some(i) = inv {
        // Grenade_C RemoveItemInSlot on release: a thrown grenade leaves the hand at once (the body copy kept it until the holster).
        let slot = &i.slots[i.current];
        s.item = if slot.kind == crate::weapon::ItemKind::Grenade && slot.ammo == 0 { String::new() } else { slot.name.clone() };
    }
    s.fired += fired.read().count() as u32;
    for d in dmg.read() {
        s.hits += 1;
        if let Some(h) = d.1 {
            s.push = Some((h.impulse, h.at, time.elapsed_secs()));
        }
    }
    for m in montages.read().filter(|m| m.kind.on_body()) {
        s.montage += 1;
        s.clip = crate::viewmodel::clip_of(&m.montage.path);
        s.blend = (m.montage.blend_in, m.montage.blend_out);
    }
}

// ---- gun in the hands ----

#[derive(serde::Deserialize, Clone, Copy)]
struct Grip {
    r: [f32; 7],
    l: [f32; 7],
}

fn grips() -> &'static HashMap<String, Grip> {
    static G: OnceLock<HashMap<String, Grip>> = OnceLock::new();
    G.get_or_init(|| serde_json::from_str(include_str!("../../assets/grips.json")).expect("assets/grips.json"))
}

fn xf(v: [f32; 7]) -> (Vec3, Quat) {
    (Vec3::new(v[0], v[1], v[2]), Quat::from_xyzw(v[3], v[4], v[5], v[6]).normalize())
}

/// Item row -> gun glb (guns, grenades, knife, drone; None = nothing to draw).
fn model_of(row: &str) -> Option<&'static str> {
    static M: OnceLock<HashMap<String, &'static str>> = OnceLock::new();
    let m = M.get_or_init(|| crate::weapon::load_weapons().iter().filter_map(|w| Some((w.name.clone(), gun_model(&w.name, &w.anims)?))).collect());
    m.get(row).copied().or_else(|| gun_model(row, &serde_json::Value::Null))
}

/// Where the gun goes (all in BodyRoot space) and where the hands hold it.
pub struct Hold {
    /// Gun `root` joint: orientation and origin.
    pub rot: Quat,
    pub joint: Vec3,
    pub hand_r: (Vec3, Quat),
    pub hand_l: (Vec3, Quat),
}

/// Right-hand point relative to the chest (spine_03) at hip / aimed, in a yaw-forward frame (x right, y up, -z forward).
const HAND_HIP: Vec3 = Vec3::new(0.09, -0.03, -0.30);
const HAND_ADS: Vec3 = Vec3::new(0.035, 0.19, -0.34);

/// Third-person gun hold between montages: the hands pivot about the chest by the aim pitch, rise to the sight line when aiming and
/// kick back per shot. Grips come from the FP arms (assets/grips.json). During a montage bodyik blends to the clip's own arms + gun.
// ponytail: no per-gun hold offsets beyond the measured grips.
pub fn hold(model: &str, chest: Vec3, pitch: f32, ads: f32, kick: f32) -> Hold {
    let g = grips().get(model).or_else(|| grips().get("Glock17")).expect("Glock17 grip");
    let ((rp, rq), (lp, lq)) = (xf(g.r), xf(g.l));
    let rx = Quat::from_rotation_x(pitch);
    let hand = chest + rx * (HAND_HIP.lerp(HAND_ADS, ads) + Vec3::Z * 0.03 * kick);
    let rot = rx * Quat::from_rotation_x(kick * 0.07) * Quat::from_mat3(&ads_axes(model));
    let joint = hand - rot * rp;
    Hold { rot, joint, hand_r: (hand, rot * rq), hand_l: (joint + rot * lp, rot * lq) }
}

/// Keeps one CharGun on every world body copy matching its character's equipped item.
fn sync_guns(
    mut cmd: Commands,
    assets: Res<AssetServer>,
    roots: Query<(Entity, &crate::body::BodyRoot, &ChildOf)>,
    chars: Query<&CharacterState>,
    guns: Query<(Entity, &CharGun, &ChildOf)>,
) {
    for (root, r, co) in &roots {
        let (true, Ok(st)) = (r.world, chars.get(co.parent())) else { continue };
        let want = if st.dead { None } else { model_of(&st.item) }; // the gun leaves the hands on death
        let cur = guns.iter().find(|(_, _, c)| c.parent() == root);
        if cur.map(|c| c.1.model) == want {
            continue;
        }
        if let Some((e, _, _)) = cur {
            cmd.entity(e).despawn();
        }
        if let Some(m) = want {
            let scene: Handle<WorldAsset> = assets.load(bevy::gltf::GltfAssetLabel::Scene(0).from_asset(format!("models/guns/{m}.glb")));
            cmd.spawn((CharGun { model: m, joint: None }, Name::new("char_gun"), WorldAssetRoot(scene), Transform::default(), Visibility::default(), ChildOf(root)));
        }
    }
}

/// Finds each world gun's `root` joint once its scene exists and hangs the muzzle flash (FireLocation socket) on it.
fn gun_joints(
    mut cmd: Commands,
    mut guns: Query<(Entity, &mut CharGun, &ChildOf)>,
    names: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    static FIRE: OnceLock<HashMap<String, (String, f32, f32, f32)>> = OnceLock::new();
    let fire = FIRE.get_or_init(|| serde_json::from_str(include_str!("../../assets/fire_sockets.json")).unwrap_or_default());
    for (ge, mut g, co) in &mut guns {
        if g.joint.is_some() {
            continue;
        }
        // Guns have a `root` joint; items name theirs b_grenade_Root / RootNode / root_goggles. Without a joint the CharGun is never placed and
        // sits at the body's feet (the "second grenade at the player's feet").
        let in_gun = |e: Entity| root_of_entity(e, ge, &parents);
        let Some((j, _)) = names.iter().find(|(e, n)| n.as_str().eq_ignore_ascii_case("root") && in_gun(*e)).or_else(|| names.iter().find(|(e, n)| n.to_lowercase().contains("root") && in_gun(*e))) else { continue };
        g.joint = Some(j);
        let at = fire.get(g.model).map_or(Vec3::new(0.0, -0.4, 0.05), |&(_, x, y, z)| Vec3::new(x, -y, z) * 0.01);
        let m = mats.add(StandardMaterial { base_color: Color::srgb(1.0, 0.85, 0.5), emissive: LinearRgba::rgb(40.0, 24.0, 6.0), unlit: true, alpha_mode: AlphaMode::Add, ..default() });
        cmd.spawn((
            MuzzleFlash(co.parent()),
            Mesh3d(meshes.add(Sphere::new(0.025))),
            MeshMaterial3d(m),
            PointLight { intensity: 30_000.0, range: 6.0, color: Color::srgb(1.0, 0.8, 0.5), shadow_maps_enabled: false, ..default() },
            Transform::from_translation(at).with_scale(Vec3::new(1.0, 3.0, 1.0)),
            Visibility::Hidden,
            ChildOf(j),
        ));
    }
}

/// Muzzle flash for the first ~30 ms of a shot.
fn flash(mut f: Query<(&MuzzleFlash, &mut Visibility)>, anims: Query<&crate::body::BodyAnim>) {
    for (m, mut v) in &mut f {
        let on = anims.get(m.0).is_ok_and(|a| a.kick > 0.45);
        *v = if on { Visibility::Visible } else { Visibility::Hidden };
    }
}

/// WW_SurvivorStatus on death: after RandomFloatInRange(0.1, 1.0) s, ScreamSound (multicast) plays ScreamBlood_Cue at the head with
/// `suffer` = RandomBoolWithWeight(Default Suffer Proba 0.2) (long agony scream + death breaths instead of a short cry).
// ponytail: the Health < -350 instant-death branch (no scream) and StopScream on respawn are not ported (remotes carry no health).
const SCREAM: &str = "/Game/Ultimate_SFX/DeathScream/ScreamBlood_Cue";
const SUFFER_PROBA: f32 = 0.2;
fn death_scream(
    time: Res<Time>,
    chars: Query<(Entity, &CharacterState, &GlobalTransform)>,
    cam: Single<&GlobalTransform, With<crate::player::BodycamCamera>>,
    mut pending: Local<HashMap<Entity, (bool, Option<f32>)>>,
    mut out: MessageWriter<crate::audio::PlayCue>,
) {
    for (e, st, tf) in &chars {
        let (was, timer) = pending.entry(e).or_insert((st.dead, None));
        if st.dead && !*was {
            *timer = Some(rand::random_range(0.1..1.0));
        }
        *was = st.dead;
        if let Some(t) = timer {
            *t -= time.delta_secs();
            if *t <= 0.0 {
                *timer = None;
                let head = tf.translation() + Vec3::Y * 0.3; // the body lies on the ground by now
                out.write(crate::audio::PlayCue { cue: SCREAM.into(), dist_cm: cam.translation().distance(head) * 100.0, bools: vec![("suffer", rand::random::<f32>() < SUFFER_PROBA)] });
            }
        }
    }
    pending.retain(|e, _| chars.contains(*e));
}

// ---- test puppet ----

#[derive(Resource)]
struct PuppetCfg {
    off: Vec3,
    dyaw: f32,
}
#[derive(Resource, Default)]
struct Feed(VecDeque<(f32, Transform, CharacterState)>);

const PUPPET_DELAY: f32 = 1.0;

fn puppet_spawn(mut cmd: Commands) {
    let e = spawn_character(&mut cmd, Transform::from_xyz(0.0, 0.0, -2.5), CharacterState::default());
    cmd.entity(e).insert((Puppet, Name::new("puppet")));
}

/// The puppet shows the local player's snapshot from PUPPET_DELAY seconds ago, at the configured offset / facing.
fn puppet_feed(
    time: Res<Time>,
    cfg: Res<PuppetCfg>,
    mut feed: ResMut<Feed>,
    me: Single<(&Transform, &CharacterState), (With<Player>, Without<Puppet>)>,
    mut pup: Single<(&mut Transform, &mut CharacterState), With<Puppet>>,
) {
    let now = time.elapsed_secs();
    feed.0.push_back((now, *me.0, me.1.clone()));
    while feed.0.get(1).is_some_and(|s| s.0 <= now - PUPPET_DELAY) {
        feed.0.pop_front();
    }
    let Some((_, tf, st)) = feed.0.front() else { return };
    let turn = Quat::from_rotation_y(cfg.dyaw);
    *pup.0 = Transform { translation: tf.translation + tf.rotation * cfg.off, rotation: tf.rotation * turn, ..default() };
    *pup.1 = CharacterState { vel: turn * st.vel, ..st.clone() };
}

// ---- dev: grip measurement ----

/// Dev: BODYCAM_GRIPDUMP=1 equips every gun in turn and prints where the FP arms' hands sit in the gun's `root` joint frame
/// (hip idle) -> assets/grips.json (the third-person hands hold the gun there).
fn grip_dump(
    time: Res<Time>,
    mut inv: Single<&mut Inventory>,
    guns: Query<(Entity, &GunRoot)>,
    arms: Query<Entity, With<ArmsRoot>>,
    names: Query<(Entity, &Name, &GlobalTransform)>,
    parents: Query<&ChildOf>,
    mut st: Local<(usize, f32, Vec<String>)>,
) {
    let now = time.elapsed_secs();
    if st.2.is_empty() {
        st.2 = inv.slots.iter().take(2).chain(inv.pool.iter()).filter(|w| w.kind == crate::weapon::ItemKind::Gun).map(|w| w.name.clone()).collect();
        st.1 = now + 8.0;
    }
    if now < st.1 || st.0 >= st.2.len() {
        return;
    }
    let row = st.2[st.0].clone();
    let model = inv.slots.iter().chain(inv.pool.iter()).find(|w| w.name == row).and_then(|w| gun_model(&w.name, &w.anims));
    if inv.slots[inv.current].name != row {
        let other = if row == "Glock17" { "M4A1" } else { "Glock17" };
        inv.equip(&row, other);
        st.1 = now + 8.0;
        return;
    }
    st.0 += 1;
    st.1 = now + 1.5;
    let (Some(model), Some((gun, _))) = (model, guns.iter().find(|(_, g)| Some(g.0) == model)) else { return };
    let (Ok(arms), Some(root)) = (arms.single(), names.iter().find(|(e, n, _)| n.as_str().eq_ignore_ascii_case("root") && root_of_entity(*e, gun, &parents))) else { return };
    let inv_root = root.2.affine().inverse();
    let mut out = format!("GRIP \"{model}\": {{");
    for side in ["r", "l"] {
        let Some((_, _, g)) = names.iter().find(|(e, n, _)| n.as_str() == format!("hand_{side}") && root_of_entity(*e, arms, &parents)) else { return };
        let (_, r, t) = (inv_root * g.affine()).to_scale_rotation_translation();
        out += &format!("\"{side}\": [{:.4},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}], ", t.x, t.y, t.z, r.x, r.y, r.z, r.w);
    }
    info!("{}}},", out.trim_end_matches(", "));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gun_points_where_aimed() {
        for m in ["M4A1", "Glock17", "Akm", "Revolver"] {
            let h = hold(m, Vec3::new(0.0, 1.3, 0.0), 0.4, 0.0, 0.0);
            // muzzle = joint -Y (Revolver: +Z) pitched up with the aim
            let muzzle = h.rot * if m == "Revolver" { Vec3::Z } else { Vec3::NEG_Y };
            let want = Quat::from_rotation_x(0.4) * Vec3::NEG_Z;
            assert!(muzzle.dot(want) > 0.999, "{m}: {muzzle} vs {want}");
            assert!((h.hand_r.0 - Vec3::new(0.0, 1.3, 0.0)).length() < 0.5);
        }
        assert!(grips().len() >= 17 && model_of("M4A1") == Some("M4A1"));
    }
}
