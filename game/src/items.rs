//! Non-gun inventory items: grenades (GA_Character_Aim_Grenade / GA_ThrowGrenade / Grenade_C and its G67_C / Flash_C / n74_C / RGN_C /
//! MB78_C / Flash_9-Bangers_C children),
//! knife (GA_Character_QuickAttack / GA_TriggerKnife / Knife_C.CloseRangeAttack) and drop/pick up (GA_Character_DropItem).
//! Slots follow Bodycam_Player IA_Slot1..5: Primary, Secondary, Grenade, Melee, Perk.First (drone: perks.rs).
use crate::audio::PlayCue;
use crate::effects::{Explosion, Flashbang};
use crate::player::{BodycamCamera, IgnoreTraces, Player};
use crate::status::PlayerDamage;
use crate::ui::settings::Settings;
use crate::viewmodel::GunRoot;
use crate::weapon::{bone_multiplier, Health, Inventory, ItemKind, Montage, MontageStarted, Weapon};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use serde_json::Value;

// ponytail: underhand grenade throw (the FP BAS only has Overhand; AC_BodycamAnimationComponent.bOverhandGrenade is set by C++), knife stab/stick physics (stuck = pickup at the impact), ReceivePointDamage fuse shortening,
// N67/RGN only collide with the scene (not characters), MB78 P_Explosion emitter / decals / Cubit impact FX traces are the frag FX,
// bots are not flashed (GE_Flashed 18 s), BFL_Explosion falloff formula is UE's ApplyRadialDamageWithFalloff shape.
pub const SLOT_GRENADE: usize = 2;
pub const SLOT_MELEE: usize = 3;
pub const SLOT_PERK: usize = 4;

/// assets/items/<Row>.json (tools/weapon_data.py: DT_PerkItems rows with AnimationData).
pub fn load_item(row: &str) -> Option<Weapon> {
    let kind = if row.starts_with("Grenade") {
        ItemKind::Grenade
    } else if matches!(row, "Kamikaze" | "Sentinel" | "Trataxes" | "Kyoshin") {
        ItemKind::Drone
    } else if row == "Bomb" {
        ItemKind::Bomb
    } else {
        ItemKind::Knife
    };
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/items");
    let v: Value = serde_json::from_str(&std::fs::read_to_string(format!("{dir}/{}.json", row.replace(' ', "_"))).ok()?).ok()?;
    let mut anims = v["anim_set"].clone();
    anims["Extra"] = v["extra"].clone();
    anims["Actor"] = v["actor"].clone();
    Some(Weapon::item(row, kind, anims))
}

/// The three item slots of the saved loadout; missing rows are skipped.
pub fn load_items(cfg: &Settings) -> Vec<Weapon> {
    let explosive = std::env::var("BODYCAM_EXPLOSIVE").unwrap_or_else(|_| cfg.explosive.clone()); // dev: "Grenade G67" / "Grenade Flash"
    [&explosive, &cfg.melee, &cfg.gadget].into_iter().filter_map(|r| load_item(r)).collect()
}

/// Tablet names (the M67 label is the loadout screen's: the row is the G67).
pub fn display_name(row: &str) -> &str {
    match row {
        "Grenade G67" => "M67 Frag Grenade",
        "Grenade Flash" => "Flashbang",
        "Kamikaze" => "Kamikaze",
        "Sentinel" => "Sentinel",
        "Trataxes" => "Trataxes",
        "Kyoshin" => "Kyoshin",
        "Knife" => "Knife",
        "Grenade N67" => "N67 Sticky",
        "Grenade RGN" => "RGN Impact",
        "Grenade G68" => "MB78",
        "Grenade Flash 9-Bangers" => "Flash 9-Bangers",
        r => r,
    }
}

/// BP defaults: Grenade_C (OverhandLaunchSpeed 1600, MaxThrowDistance 1500, ...), G67_C / Flash_C explosion data.
const OVERHAND_SPEED: f32 = 16.0;
const MAX_THROW_DIST: f32 = 15.0;
const G: f32 = 9.8;
const THROW_DELAY: f32 = 0.2; // Grenade_C.Throw: Delay 0.2 before the physics impulse
const FUSE: f32 = 3.0; // DT_GrenadeInfo FuseDelay (counted from the Throw event, which fires before the 0.2 s delay)
pub const KNIFE_RANGE: f32 = 1.25; // Knife_C.CloseAttackRange 125 cm
pub const KNIFE_RADIUS: f32 = 0.5; // sphere trace radius 50 cm
pub const KNIFE_DAMAGE: f32 = 100.0; // GE_KnifeStat_Default Damage
const KNIFE_THROW_DELAY: f32 = 0.3; // Knife_C Server - StartThrowing: Delay 0.3 after the Throw montage starts
const KNIFE_SPEED: f32 = 15.0; // Knife_C ProjectileMovement InitialSpeed 1500 cm/s
const KNIFE_AIM_RANGE: f32 = 8.0; // AimAssistMaxRange 800 cm (sphere trace radius 50 cm)
const KNIFE_STAB_DEPTH: f32 = 0.01; // StabDepth 1 cm

/// RadialDamageWithFalloff(Base, Inner, Outer, Exponent): full damage inside Inner, 0 beyond Outer, power falloff between (cm).
// ponytail: BFL_Explosion.RadialDamageWithFalloff body is not decompiled; this is UE's ApplyRadialDamageWithFalloff shape.
pub fn falloff(d_cm: f32, inner: f32, outer: f32, exp: f32) -> f32 {
    if d_cm <= inner {
        1.0
    } else if d_cm >= outer {
        0.0
    } else {
        (1.0 - (d_cm - inner) / (outer - inner)).powf(exp)
    }
}

/// Low-arc ballistic launch velocity from `from` to `to` at `speed` (UE BlueprintSuggestProjectileVelocity, favor-high-arc off).
pub fn toss_velocity(from: Vec3, to: Vec3, speed: f32) -> Option<Vec3> {
    let d = to - from;
    let flat = Vec3::new(d.x, 0.0, d.z);
    let r = flat.length();
    if r < 1e-3 {
        return None;
    }
    let disc = speed.powi(4) - G * (G * r * r + 2.0 * d.y * speed * speed);
    if disc < 0.0 {
        return None;
    }
    let tan = (speed * speed - disc.sqrt()) / (G * r);
    let dir = flat / r;
    let ang = tan.atan();
    Some((dir * ang.cos() + Vec3::Y * ang.sin()) * speed)
}

/// One radial blast (RadialDamageWithFalloff base x multiplier, Inner/Outer radius cm, falloff exponent).
#[derive(Clone, Copy)]
pub struct BlastSpec {
    pub base: f32,
    pub inner: f32,
    pub outer: f32,
    pub exp: f32,
    /// Applies GE_Flashed (whiteout).
    pub flash: bool,
    /// Applies the radial damage (Flash_C: flash only).
    pub hurt: bool,
    /// Explosion SoundCue ("" = none).
    pub cue: &'static str,
}
const G67_CUE: &str = "/Game/Audio/Grenade/Cue/Bodycam_Grenade_G67";
/// G67_C: (1.0, 350, 1000, 1.3) x GE_GrenadeStat_G67 Damage 200.
pub const G67_BLAST: BlastSpec = BlastSpec { base: 200.0, inner: 350.0, outer: 1000.0, exp: 1.3, flash: false, hurt: true, cue: G67_CUE };
/// Flash_C: (1.0, 250, 1000, 2.0) x GE_GrenadeStat_Flash Damage 50; the flash applies GE_Flashed instead.
pub const FLASH_BLAST: BlastSpec = BlastSpec { base: 50.0, inner: 250.0, outer: 1000.0, exp: 2.0, flash: true, hurt: false, cue: "/Game/BallisticsVFX/SFX/Explodes/Explosions_flash" };
/// GA_ExplodePerk: ExplosionDamage 200, Inner 200, Outer 1000, exponent 2.0.
/// n74_C (N67 Sticky): (1.0, 250, 1000, 2.0) x GE_GrenadeStat_N74 Damage 200.
const N74_BLAST: BlastSpec = BlastSpec { base: 200.0, inner: 250.0, outer: 1000.0, exp: 2.0, flash: false, hurt: true, cue: "/Game/BallisticsVFX/SFX/Explodes/Explosions_fragN74" };
/// RGN_C: (1.0, 80, 500, 2.0) x GE_GrenadeStat_RGN Damage 200.
const RGN_BLAST: BlastSpec = BlastSpec { base: 200.0, inner: 80.0, outer: 500.0, exp: 2.0, flash: false, hurt: true, cue: "/Game/BallisticsVFX/SFX/Explodes/Explosions_fragRGN" };
/// MB78_C (row G68): (1.0, 200, 2000, 2.0) then three (0.4, 100, 2000, 2.0) x GE_GrenadeStat_G68 Damage 50; the fireworks cue plays once.
const MB78_BLAST: BlastSpec = BlastSpec { base: 50.0, inner: 200.0, outer: 2000.0, exp: 2.0, flash: false, hurt: true, cue: "" };
const MB78_TAIL: BlastSpec = BlastSpec { base: 20.0, inner: 100.0, outer: 2000.0, exp: 2.0, flash: false, hurt: true, cue: "" };
/// Flash_9-Bangers_C bang: (0.05, 250, 1000, 2.0) x GE_GrenadeStat_Flash_9-Bangers Damage 150; its ApplyRadialDamageWithFalloff override adds GE_Flashed.
const BANG: BlastSpec = BlastSpec { base: 7.5, inner: 250.0, outer: 1000.0, exp: 2.0, flash: true, hurt: true, cue: "" };
pub const DRONE_BLAST: BlastSpec = BlastSpec { base: 200.0, inner: 200.0, outer: 1000.0, exp: 2.0, flash: false, hurt: true, cue: G67_CUE };

/// The local player threw a grenade (net.rs replicates it).
#[derive(Message)]
pub struct Thrown {
    pub start: Vec3,
    pub vel: Vec3,
    /// Grenade row (DT_PerkItems).
    pub row: String,
}

/// The local player swung the knife from `origin` along `dir` (net.rs: host resolves hits on other players).
#[derive(Message)]
pub struct Melee {
    pub origin: Vec3,
    pub dir: Vec3,
}

/// The local player's drone crashed at `at` (net.rs replicates the blast).
#[derive(Message)]
pub struct DroneCrashed {
    pub at: Vec3,
}

/// Grenade_C children by DT_PerkItems row (ActorClassOverride).
#[derive(Clone, Copy, PartialEq)]
enum GKind {
    Frag,
    Flash,
    /// n74_C: sticks on the first hit, explodes 1.5 s later.
    Sticky,
    /// RGN_C: explodes on the first hit.
    Impact,
    /// MB78_C: four staggered blasts.
    Mb78,
    /// Flash_9-Bangers_C: nine flash bangs.
    Bangers,
}

impl GKind {
    fn of(row: &str) -> Self {
        match row {
            "Grenade Flash" => Self::Flash,
            "Grenade N67" => Self::Sticky,
            "Grenade RGN" => Self::Impact,
            "Grenade G68" => Self::Mb78,
            "Grenade Flash 9-Bangers" => Self::Bangers,
            _ => Self::Frag,
        }
    }
    /// Wep Body mesh (GrenadeInfo.Mesh / the BP's Wep Body override).
    fn glb(self) -> &'static str {
        match self {
            Self::Frag => "G67",
            Self::Flash => "Flashbang",
            Self::Sticky => "N67",
            Self::Impact => "RGN",
            Self::Mb78 => "MB78",
            Self::Bangers => "Flashbang02",
        }
    }
    /// GrenadeExplosion: (seconds after it, blast) and the cue it spawns once.
    fn salvo(self) -> (&'static str, Vec<(f32, BlastSpec)>) {
        match self {
            Self::Frag => ("", vec![(0.0, G67_BLAST)]),
            Self::Flash => ("", vec![(0.0, FLASH_BLAST)]),
            Self::Sticky => ("", vec![(0.0, N74_BLAST)]),
            Self::Impact => ("", vec![(0.0, RGN_BLAST)]),
            // Sequence: blast now; latent Delays 0.2 / 0.5 / 1.0 each fire one more
            Self::Mb78 => ("/Game/Ultimate_SFX/Explosion_Sounds_Pro_HD_Remake/Cues/Explosion_Grenade_fireworks_Cue", vec![(0.0, MB78_BLAST), (0.2, MB78_TAIL), (0.5, MB78_TAIL), (1.0, MB78_TAIL)]),
            // DoN 9: light on, Delay 0.1, light off + bang, Delay 0.2
            Self::Bangers => ("/Game/Ultimate_SFX/Explosion_Sounds_Pro_HD_Remake/Cues/Explosion_Grenade_Flash_9-Bangers_Cue", (0..9).map(|i| (0.1 + 0.3 * i as f32, BANG)).collect()),
        }
    }
}

const STICK_DELAY: f32 = 1.5; // n74_C hit: Delay 1.5 -> Explosion

pub fn spawn_grenade(cmd: &mut Commands, assets: &AssetServer, start: Vec3, vel: Vec3, row: &str, owner: u8) {
    let kind = GKind::of(row);
    let path = format!("models/guns/{}.glb", kind.glb());
    cmd.spawn((
        Grenade { vel, fuse: FUSE - THROW_DELAY, kind, hits: 0, spin: Vec3::new(9.0, 3.0, 7.0), rest: false, owner },
        WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
        Transform::from_translation(start),
    ));
}

/// Ask for a blast (grenade fuse, drone crash).
#[derive(Message)]
pub struct Detonate {
    pub at: Vec3,
    pub spec: BlastSpec,
}

/// A blast owned by another player (remote grenade / drone): same as `Detonate` with `owner` = their net id.
#[derive(Message)]
pub struct DetonateFor {
    pub at: Vec3,
    pub spec: BlastSpec,
    pub owner: u8,
}

/// Radial damage for the networked host to apply to every player (origin = blast point + 50 cm), crediting `owner`.
#[derive(Message)]
pub struct BlastDamage {
    pub origin: Vec3,
    pub spec: BlastSpec,
    pub owner: u8,
}

#[derive(Default, PartialEq, Clone, Copy)]
enum GState {
    #[default]
    Idle,
    /// PullPin montage running (RMB held).
    Pulling,
    /// GameplayEvent.Grenade.HoldReach -> GE_ReadyToThrowGrenade: LMB throws.
    Ready,
    /// Throw montage running; the grenade leaves at THROW_DELAY.
    Throwing,
}

#[derive(Resource, Default)]
struct Items {
    grenade: GState,
    t: f32,
    spawned: bool,
    hold: f32,
    /// LMB tapped while the pin was still being pulled (RMB held): throw as soon as it is ready.
    throw_queued: bool,
    /// Knife combo index (Fire list).
    combo: usize,
    /// Knife_C.bIsAiming (RMB, GA_Character_Aim_Knife): LMB throws instead of stabbing.
    knife_aim: bool,
    /// Throw montage running: seconds since StartThrowing.
    knife_throw: Option<f32>,
    /// GA_Character_QuickAttack: slot to return to after the swing.
    quick: Option<QuickAttack>,
}

#[derive(Clone, Copy)]
struct QuickAttack {
    prev: usize,
    /// None = waiting for the knife draw, Some(t) = swing running.
    swing: Option<f32>,
}

#[derive(Component)]
struct Grenade {
    vel: Vec3,
    fuse: f32,
    kind: GKind,
    hits: u32,
    spin: Vec3,
    rest: bool,
    /// Thrower's net id (kill credit).
    owner: u8,
}

/// A detonated grenade running its GrenadeExplosion sequence (one blast for most, MB78 / 9-Bangers several).
#[derive(Component)]
struct Salvo {
    t: f32,
    kind: GKind,
    owner: u8,
    blasts: Vec<(f32, BlastSpec)>,
    light: Option<Entity>,
}

/// Knife_C in flight (ProjectileMovement + RotatingMovement).
#[derive(Component)]
struct KnifeProj {
    vel: Vec3,
    /// Melee row (the stuck pickup) and its glb.
    row: String,
    model: &'static str,
}

/// A dropped item (GA_Character_DropItem -> DropItem notify -> DropEquippedItem): F picks it up again.
#[derive(Component)]
struct Pickup {
    row: String,
    vel: Vec3,
}

pub struct ItemsPlugin;
impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PostUpdate, hide_thrown);
        app.init_resource::<Items>().add_message::<Detonate>().add_message::<DetonateFor>().add_message::<BlastDamage>().add_message::<Thrown>().add_message::<Melee>().add_message::<DroneCrashed>().add_message::<PlayCue>().add_systems(Update, (grenade_input, knife_input, knife_flight, projectiles, bip_glow, salvos, detonate, tag_meshes, drop_pickup).chain());
    }
}

fn montage_of(w: &Weapon, key: &str) -> Option<Montage> {
    w.montage(key)
}

/// Extra (non-BAS) clip from assets/items `extra`.
fn extra(w: &Weapon, key: &str) -> Option<Montage> {
    Montage::from_value(w.anims["Extra"].get(key)?)
}

#[allow(clippy::too_many_arguments)]
fn grenade_input(
    mut cmd: Commands,
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    assets: Res<AssetServer>,
    mut st: ResMut<Items>,
    mut inv: Single<&mut Inventory>,
    p: Single<&Player>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    hand: Query<&GlobalTransform, With<GunRoot>>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    mut out: MessageWriter<MontageStarted>,
    mut thrown: MessageWriter<Thrown>,
) {
    let dt = time.delta_secs();
    let cur = inv.current;
    if inv.slots[cur].kind != ItemKind::Grenade {
        st.grenade = GState::Idle;
        return;
    }
    if inv.slots[cur].ammo == 0 && st.grenade != GState::Throwing {
        // the thrown grenade's slot is empty: GetBestSlotToSwitch -> primary
        st.grenade = GState::Idle;
        inv.swap_to(0, &mut out);
        return;
    }
    let busy = inv.busy();
    let rmb = mouse.pressed(MouseButton::Right);
    match st.grenade {
        GState::Idle => {
            // GA_Character_Aim_Grenade: RMB -> PullPin montage (BAS MagCheckFull); the notify at its end is HoldReach.
            if mouse.just_pressed(MouseButton::Right) && !busy && !p.dead && !p.sprinting {
                let m = montage_of(&inv.slots[cur], "MagCheckFull");
                inv.use_item(m, &mut out);
                st.grenade = GState::Pulling;
                st.throw_queued = false;
            }
        }
        GState::Pulling => {
            st.throw_queued |= rmb && mouse.just_pressed(MouseButton::Left);
            if !busy {
                if rmb {
                    st.grenade = GState::Ready;
                    st.hold = 0.0;
                } else {
                    // RMB let go: CancelOverhand puts the pin back
                    let m = extra(&inv.slots[cur], "CancelOverhand");
                    inv.use_item(m, &mut out);
                    st.grenade = GState::Idle;
                }
            }
        }
        GState::Ready => {
            if !rmb {
                // GA_Character_Aim_Grenade ends (RMB released) without a throw: CancelOverhand, back to idle
                let m = extra(&inv.slots[cur], "CancelOverhand");
                inv.use_item(m, &mut out);
                st.grenade = GState::Idle;
            } else if mouse.just_pressed(MouseButton::Left) || std::mem::take(&mut st.throw_queued) {
                // GA_ThrowGrenade (needs Character.State.Grenade.ReadyToThrow): Throw montage = BAS Fire[0].
                let m = inv.slots[cur].montage_list(0, "Fire");
                inv.use_item(m, &mut out);
                st.grenade = GState::Throwing;
                st.t = 0.0;
                st.spawned = false;
            } else if let Some(l) = extra(&inv.slots[cur], "OverhandLoop") {
                // Overhand_Loop hold pose: re-trigger the loop clip before it blends out.
                st.hold -= dt;
                if st.hold <= 0.0 {
                    st.hold = l.length - 0.3;
                    out.write(MontageStarted { row: inv.slots[cur].name.clone(), kind: crate::weapon::ActionKind::CameraAdjust, montage: l });
                }
            }
        }
        GState::Throwing => {
            st.t += dt;
            if st.t >= THROW_DELAY && !st.spawned {
                st.spawned = true;
                let row = inv.slots[cur].name.clone();
                let c = cam.compute_transform();
                let fwd = c.forward().as_vec3();
                // Grenade_C.CalculateGrenadeVelocity: target = camera line trace 1500 cm (-100 cm z), launch from the held grenade.
                let start = hand.iter().next().map(|g| g.translation()).filter(|p| p.distance(c.translation) < 1.0).unwrap_or(c.translation + fwd * 0.4 - Vec3::Y * 0.15);
                let end = c.translation + fwd * MAX_THROW_DIST - Vec3::Y;
                let filter = |e: Entity| !ignore.contains(e);
                let target = Dir3::new(end - c.translation)
                    .ok()
                    .and_then(|d| ray.cast_ray(Ray3d::new(c.translation, d), &MeshRayCastSettings::default().with_filter(&filter)).first().map(|(_, h)| h.point))
                    .unwrap_or(end);
                let vel = toss_velocity(start, target, OVERHAND_SPEED).unwrap_or(fwd * OVERHAND_SPEED);
                spawn_grenade(&mut cmd, &assets, start, vel, &row, crate::net::MY_ID.load(std::sync::atomic::Ordering::Relaxed));
                thrown.write(Thrown { start, vel, row });
                // Grenade_C: RemoveItemInSlot(Grenade) -> the slot is empty and the inventory switches to the best other slot.
                inv.slots[cur].ammo = 0;
            }
            if !inv.busy() {
                st.grenade = GState::Idle;
                if inv.slots[cur].ammo == 0 {
                    inv.swap_to(0, &mut out);
                }
            }
        }
    }
}

/// Moves a thrown grenade: gravity + ray-swept bounce (Wep Body physics: restitution of PhysMat MetalDull, +1 linear damping per hit > 300 cm/s),
/// fuse, explosion.
// ponytail: rays against the scene meshes instead of a physics engine; no angular response, spin is cosmetic.
fn projectiles(
    mut cmd: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Transform, &mut Grenade)>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    targets: Query<(), With<Health>>,
    mut snd: MessageWriter<PlayCue>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
) {
    let dt = time.delta_secs().min(0.05);
    for (e, mut tf, mut g) in &mut q {
        g.fuse -= dt;
        if g.fuse <= 0.0 {
            let (cue, blasts) = g.kind.salvo();
            if !cue.is_empty() {
                snd.write(PlayCue { bools: vec![], cue: cue.into(), dist_cm: cam.translation().distance(tf.translation) * 100.0 });
            }
            cmd.spawn((Salvo { t: 0.0, kind: g.kind, owner: g.owner, blasts, light: None }, Transform::from_translation(tf.translation)));
            cmd.entity(e).despawn();
            continue;
        }
        if g.rest {
            continue;
        }
        g.vel.y -= G * dt;
        let damping = 0.1 + g.hits as f32 * 1.0;
        g.vel *= (-damping * dt).exp();
        let step = g.vel * dt;
        let len = step.length();
        let filter = |e: Entity| !ignore.contains(e) && !targets.contains(e);
        let hit = Dir3::new(step).ok().and_then(|d| {
            ray.cast_ray(Ray3d::new(tf.translation, d), &MeshRayCastSettings::default().with_filter(&filter)).first().map(|(_, h)| (h.point, h.normal, h.distance)).filter(|h| h.2 <= len + 0.04)
        });
        match hit {
            Some((pt, n, _)) => {
                let n = n.normalize_or_zero();
                match g.kind {
                    // RGN_C hit (bIsThrown) -> Explosion
                    GKind::Impact => g.fuse = 0.0,
                    // n74_C hit: SetSimulatePhysics(false), Grenade_ImpactStick bip, BipGlow sphere on, Delay 1.5 -> Explosion
                    GKind::Sticky => {
                        tf.translation = pt + n * 0.02;
                        g.vel = Vec3::ZERO;
                        g.rest = true;
                        g.fuse = g.fuse.min(STICK_DELAY);
                        snd.write(PlayCue { bools: vec![], cue: "/Game/BallisticsVFX/SFX/Impacts/Grenade/Grenade_ImpactStick".into(), dist_cm: cam.translation().distance(pt) * 100.0 });
                        cmd.entity(e).insert(Bip);
                        continue;
                    }
                    _ => {}
                }
                let vn = g.vel.dot(n) * n;
                let vt = g.vel - vn;
                if g.vel.length() > 3.0 {
                    g.hits += 1; // Hit event only for speed > 300 cm/s
                    snd.write(PlayCue { bools: vec![], cue: "/Game/BallisticsVFX/SFX/Impacts/Grenade/Grenade_Impact".into(), dist_cm: cam.translation().distance(pt) * 100.0 });
                }
                g.vel = vt * 0.7 - vn * 0.35;
                tf.translation = pt + n * 0.04;
                if g.vel.length() < 0.4 && n.y > 0.5 {
                    g.vel = Vec3::ZERO;
                    g.rest = true;
                }
            }
            None => tf.translation += step,
        }
        let s = g.spin * (g.vel.length() / 8.0).min(1.0) * dt;
        tf.rotate(Quat::from_euler(EulerRot::XYZ, s.x, s.y, s.z));
    }
}

/// n74_C stuck: its BipGlow sphere.
#[derive(Component)]
struct Bip;

// ponytail: BipGlow's material graph is not decompiled; a small red emissive sphere at the BP's Sphere offset.
fn bip_glow(mut cmd: Commands, q: Query<Entity, Added<Bip>>, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    for e in &q {
        cmd.entity(e).with_child((Mesh3d(meshes.add(Sphere::new(0.035))), MeshMaterial3d(mats.add(StandardMaterial { base_color: Color::BLACK, emissive: LinearRgba::rgb(40.0, 0.0, 0.0), ..default() })), Transform::from_xyz(0.0, 0.0458, 0.001)));
    }
}

/// Fires each Salvo blast at its time. 9-Bangers: Flash_9-Bangers_C PointLight (30000 cd, 1800 K) on for the 0.1 s before each bang,
/// bangs scattered 0..150 cm in X/Y around the grenade.
fn salvos(mut cmd: Commands, time: Res<Time>, mut q: Query<(Entity, &Transform, &mut Salvo)>, mut out: MessageWriter<DetonateFor>) {
    let dt = time.delta_secs();
    for (e, tf, mut s) in &mut q {
        let t0 = s.t;
        s.t += dt;
        let t = s.t;
        let at = tf.translation;
        for (_, spec) in s.blasts.iter().filter(|(bt, _)| (*bt >= t0 && *bt < t) || (*bt == 0.0 && t0 == 0.0)) {
            let off = if s.kind == GKind::Bangers { Vec3::new(rand::random::<f32>() * 1.5, 0.0, rand::random::<f32>() * 1.5) } else { Vec3::ZERO };
            out.write(DetonateFor { at: at + off, spec: *spec, owner: s.owner });
        }
        if s.kind == GKind::Bangers {
            let on = t < 2.6 && t % 0.3 < 0.1;
            match (on, s.light) {
                (true, None) => s.light = Some(cmd.spawn((PointLight { intensity: 377_000.0, range: 10.0, color: Color::srgb(1.0, 0.56, 0.17), ..default() }, Transform::from_translation(at))).id()),
                (false, Some(l)) => {
                    cmd.entity(l).despawn();
                    s.light = None;
                }
                _ => {}
            }
        }
        if s.blasts.iter().all(|(bt, _)| *bt < t) && s.light.is_none() {
            cmd.entity(e).despawn();
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn detonate(
    mut cmd: Commands,
    mut ev: MessageReader<Detonate>,
    mut ev_for: MessageReader<DetonateFor>,
    mut bd: MessageWriter<BlastDamage>,
    net: Option<Res<crate::net::Net>>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    mut targets: Query<(Entity, &GlobalTransform, &mut Health)>,
    player: Single<&GlobalTransform, With<Player>>,
    mut boom: MessageWriter<Explosion>,
    mut flash: MessageWriter<Flashbang>,
    mut dmg: MessageWriter<PlayerDamage>,
    mut snd: MessageWriter<PlayCue>,
    mut fxm: MessageWriter<crate::explosion::ExplosionFx>,
) {
    let me = crate::net::MY_ID.load(std::sync::atomic::Ordering::Relaxed);
    let all: Vec<_> = ev.read().map(|d| (d.at, d.spec, me)).chain(ev_for.read().map(|d| (d.at, d.spec, d.owner))).collect();
    for (at, spec, owner) in all {
        explode(&mut cmd, at, spec, owner, net.is_some(), &mut ray, &ignore, &mut targets, player.translation() + Vec3::Y * 1.6, &mut boom, &mut flash, &mut dmg, &mut bd, &mut snd, &mut fxm);
    }
}

#[allow(clippy::too_many_arguments)]
fn explode(
    cmd: &mut Commands,
    at: Vec3,
    spec: BlastSpec,
    owner: u8,
    networked: bool,
    ray: &mut MeshRayCast,
    ignore: &Query<(), With<IgnoreTraces>>,
    targets: &mut Query<(Entity, &GlobalTransform, &mut Health)>,
    head: Vec3,
    boom: &mut MessageWriter<Explosion>,
    flash: &mut MessageWriter<Flashbang>,
    dmg: &mut MessageWriter<PlayerDamage>,
    bd: &mut MessageWriter<BlastDamage>,
    snd: &mut MessageWriter<PlayCue>,
    fxm: &mut MessageWriter<crate::explosion::ExplosionFx>,
) {
    let BlastSpec { base, inner, outer, exp, flash: flash_gren, hurt, cue } = spec;
    let origin = at + Vec3::Y * 0.5; // BFL call uses ActorLocation + (0,0,50)
    let blocked = |ray: &mut MeshRayCast, to: Vec3, ignore: &Query<(), With<IgnoreTraces>>, targets: &Query<(Entity, &GlobalTransform, &mut Health)>| {
        let d = to - origin;
        let filter = |e: Entity| !ignore.contains(e) && !targets.contains(e);
        Dir3::new(d).ok().is_some_and(|dir| ray.cast_ray(Ray3d::new(origin, dir), &MeshRayCastSettings::default().with_filter(&filter)).first().is_some_and(|(_, h)| h.distance < d.length() - 0.15))
    };
    if hurt {
        let hit: Vec<(Entity, f32)> = targets.iter().map(|(e, t, _)| (e, t.translation().distance(origin) * 100.0)).filter(|(_, d)| *d < outer).collect();
        for (e, d) in hit {
            let to = targets.get(e).unwrap().1.translation();
            if blocked(ray, to, ignore, targets) {
                continue;
            }
            let mut dead = false;
            if let Ok((_, _, mut hp)) = targets.get_mut(e) {
                hp.0 -= base * falloff(d, inner, outer, exp);
                dead = hp.0 <= 0.0;
            }
            if dead {
                cmd.entity(e).despawn();
            }
        }
        boom.write(Explosion { at });
    }
    if flash_gren {
        flash.write(Flashbang { at });
    }
    let dp = head.distance(origin) * 100.0; // the body, not the (drone) camera
    if networked {
        // players: the host applies it to everyone (LOS + falloff) and credits the owner
        if hurt {
            bd.write(BlastDamage { origin, spec, owner });
        }
    } else if !blocked(ray, head, ignore, targets) {
        let f = falloff(dp, inner, outer, exp);
        if f > 0.0 && hurt {
            dmg.write(PlayerDamage(base * f, None));
        }
    } else if flash_gren {
        // flash through geometry is still occluded by walls in the game's visibility check; here: no whiteout when blocked is not modelled (message already sent)
    }
    // Visuals: SpawnEmitterAtLocation Explosion_Frag_small / Explosion_Flash_small (explosion.rs).
    fxm.write(crate::explosion::ExplosionFx { at, flash: !hurt });
    // G67_C: AC_Detonation.DetonationSound = Bodycam_Grenade_G67; Flash_C: Explosions_flash.
    if !cue.is_empty() {
        snd.write(PlayCue { bools: vec![], cue: cue.into(), dist_cm: dp });
    }
}

/// Thrown grenades are skinned meshes with stale bounds: never cull them.
fn tag_meshes(mut cmd: Commands, new: Query<Entity, Added<Mesh3d>>, parents: Query<&ChildOf>, roots: Query<(), Or<(With<Grenade>, With<Pickup>)>>) {
    for e in &new {
        let mut x = e;
        while !roots.contains(x) {
            let Ok(p) = parents.get(x) else { break };
            x = p.parent();
        }
        if roots.contains(x) {
            cmd.entity(e).insert(bevy::camera::visibility::NoFrustumCulling);
        }
    }
}

/// Knife: LMB with the knife drawn, or X (IA_Melee -> GA_Character_QuickAttack: swap to Melee, swing, back).
#[allow(clippy::too_many_arguments)]
fn knife_input(
    time: Res<Time>,
    mouse: Res<ButtonInput<MouseButton>>,
    keys: Res<ButtonInput<KeyCode>>,
    mut st: ResMut<Items>,
    mut inv: Single<&mut Inventory>,
    p: Single<&Player>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    perk: Res<crate::perks::Perk>,
    mut cmd: Commands,
    mut targets: Query<(Entity, &GlobalTransform, &mut Health, Option<&Name>)>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    mut out: MessageWriter<MontageStarted>,
    mut snd: MessageWriter<PlayCue>,
    mut melee: MessageWriter<Melee>,
    (assets, hand): (Res<AssetServer>, Query<&GlobalTransform, With<GunRoot>>), // grouped: Bevy's 16-param limit
) {
    let dt = time.delta_secs();
    if perk.locked() {
        return;
    }
    let cur = inv.current;
    let on_knife = cur == SLOT_MELEE && inv.slots.get(cur).is_some_and(|w| w.kind == ItemKind::Knife);
    let knife_slot = inv.slots.get(SLOT_MELEE).is_some_and(|w| w.kind == ItemKind::Knife && w.ammo > 0);
    if p.dead || !knife_slot {
        return;
    }
    if !on_knife {
        st.knife_aim = false;
        st.knife_throw = None;
    }
    // Knife_C.TriggerKnife: bIsAiming -> Server - StartThrowing (Throw montage, 0.3 s later InternalStartThrowing + RemoveItemFromInventory); else CloseRangeAttack.
    if on_knife && st.quick.is_none() {
        if let Some(t) = &mut st.knife_throw {
            *t += dt;
            if *t >= KNIFE_THROW_DELAY {
                st.knife_throw = None;
                // InternalStartThrowing: SphereTraceSingleForObjects 800 cm from the camera; a hit -> UpdateProjectileParams (suggested arc), else straight at InitialSpeed.
                let c = cam.compute_transform();
                let fwd = c.forward().as_vec3();
                let start = hand.iter().next().map(|g| g.translation()).filter(|p| p.distance(c.translation) < 1.0).unwrap_or(c.translation + fwd * 0.4 - Vec3::Y * 0.15);
                let aim = targets
                    .iter()
                    .filter_map(|(e, t, _, _)| {
                        let v = t.translation() - c.translation;
                        let along = v.dot(fwd);
                        ((0.0..KNIFE_AIM_RANGE).contains(&along) && (v - fwd * along).length() <= KNIFE_RADIUS).then_some((e, t.translation(), along))
                    })
                    .min_by(|a, b| a.2.total_cmp(&b.2));
                let vel = aim.and_then(|(_, at, _)| toss_velocity(start, at, KNIFE_SPEED)).unwrap_or(fwd * KNIFE_SPEED);
                let w = &inv.slots[SLOT_MELEE];
                let model = crate::viewmodel::gun_model(&w.name, &w.anims).unwrap_or("Knife");
                cmd.spawn((KnifeProj { vel, row: w.name.clone(), model }, WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/guns/{model}.glb")))), Transform::from_translation(start).with_rotation(knife_rot(vel))));
                inv.slots[SLOT_MELEE].ammo = 0; // RemoveItemInSlot(Melee)
                inv.swap_to(0, &mut out);
            }
            return;
        }
        if mouse.just_pressed(MouseButton::Right) && !st.knife_aim && !inv.busy() {
            // ponytail: no Windup/WindDown clips (the AnimBP holds the aim pose), only the flag.
            st.knife_aim = true;
        } else if st.knife_aim && !mouse.pressed(MouseButton::Right) {
            st.knife_aim = false;
        } else if st.knife_aim && mouse.just_pressed(MouseButton::Left) {
            st.knife_aim = false;
            st.knife_throw = Some(0.0);
            let m = extra(&inv.slots[SLOT_MELEE], "Throw");
            inv.use_item(m, &mut out);
            return;
        }
    }
    let mut swing = false;
    if keys.just_pressed(KeyCode::KeyX) && st.quick.is_none() && !inv.busy() {
        if on_knife {
            swing = true;
        } else if st.grenade == GState::Idle {
            // SwapItemToSlot(Melee, speed 2.0) then WaitGameplayEvent(Swap) -> TriggerKnife.
            st.quick = Some(QuickAttack { prev: cur, swing: None });
            inv.swap_to(SLOT_MELEE, &mut out);
        }
    } else if on_knife && st.quick.is_none() && !st.knife_aim && mouse.just_pressed(MouseButton::Left) && !inv.busy() {
        swing = true;
    }
    if let Some(mut q) = st.quick {
        if q.swing.is_none() && inv.current == SLOT_MELEE && !inv.busy() {
            swing = true;
            q.swing = Some(0.0);
        } else if let Some(t) = &mut q.swing {
            *t += dt;
            if !inv.busy() {
                inv.swap_to(q.prev, &mut out);
                st.quick = None;
                return;
            }
        }
        if st.quick.is_some() {
            st.quick = Some(q);
        }
    }
    if swing {
        // Fire list = Attack01..03 montages; cycled.
        let list = inv.slots[SLOT_MELEE].anims["Fire"].as_array().map_or(1, |l| l.len().max(1));
        let m = inv.slots[SLOT_MELEE].montage_list(st.combo % list, "Fire");
        st.combo += 1;
        inv.use_item(m, &mut out);
        // Knife_C.CloseRangeAttack: SphereTrace 50 cm from the camera 125 cm ahead -> GE_Knife_Damage (100, bone multiplier).
        let (o, d) = (cam.translation(), cam.forward().as_vec3());
        melee.write(Melee { origin: o, dir: d });
        let best = targets
            .iter()
            .filter_map(|(e, t, _, n)| {
                let v = t.translation() - o;
                let along = v.dot(d);
                let perp = (v - d * along.clamp(0.0, KNIFE_RANGE)).length();
                (along > -KNIFE_RADIUS && perp <= KNIFE_RADIUS).then_some((e, along, n.map_or(1.0, |n| bone_multiplier(n.as_str()))))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((e, along, mult)) = best {
            let filter = |x: Entity| !ignore.contains(x) && !targets.contains(x);
            let wall = Dir3::new(d).ok().is_some_and(|dir| ray.cast_ray(Ray3d::new(o, dir), &MeshRayCastSettings::default().with_filter(&filter)).first().is_some_and(|(_, h)| h.distance < along.max(0.0) - 0.05));
            if !wall && let Ok((_, _, mut hp, _)) = targets.get_mut(e) {
                hp.0 -= KNIFE_DAMAGE * mult;
                snd.write(PlayCue { bools: vec![], cue: "/Game/Audio/Knife/Cue/Bodycam_SFX_Weapon_Knife_Attack_Gore".into(), dist_cm: 60.0 }); // MultiCast-PlayHitSound
                if hp.0 <= 0.0 {
                    cmd.entity(e).despawn();
                }
            }
        }
    }
}

/// Knife mesh long axis (Knife.glb: +Z = blade tip) along the velocity; ponytail: roll about the blade is fixed.
fn knife_rot(vel: Vec3) -> Quat {
    Quat::from_rotation_arc(Vec3::Z, vel.normalize_or_zero())
}

/// Thrown knife: ProjectileMovement (gravity, 15 m/s), RotatingMovement while flying; on a hit the knife sticks (bIsStuck, StabDepth 1 cm) and is
/// a pickup again; a hit on a target applies GE_Knife_Damage (100 x bone multiplier) + Throw_Impact_Gore, else Throw_Impact.
// ponytail: swept ray (no sphere body), stuck knives never attach to moving targets, spin about X at 20 rad/s.
fn knife_flight(
    mut cmd: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Transform, &mut KnifeProj, Option<&Children>)>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    mut targets: Query<(Entity, &GlobalTransform, &mut Health, Option<&Name>)>,
    mut snd: MessageWriter<PlayCue>,
    assets: Res<AssetServer>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
) {
    let dt = time.delta_secs().min(0.05);
    for (e, mut tf, mut k, _) in &mut q {
        k.vel.y -= G * dt;
        let step = k.vel * dt;
        let len = step.length();
        let dir = step.normalize_or_zero();
        // target hit: the nearest Health entity within 0.4 m of the step
        let hit_t = targets
            .iter()
            .filter_map(|(t, g, _, n)| {
                let v = g.translation() - tf.translation;
                let along = v.dot(dir);
                ((-0.2..len + 0.2).contains(&along) && (v - dir * along).length() <= 0.4).then_some((t, along.max(0.0), n.map_or(1.0, |n| bone_multiplier(n.as_str()))))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1));
        let filter = |x: Entity| !ignore.contains(x) && !targets.contains(x);
        let wall = Dir3::new(step).ok().and_then(|d| ray.cast_ray(Ray3d::new(tf.translation, d), &MeshRayCastSettings::default().with_filter(&filter)).first().map(|(_, h)| h.distance).filter(|d| *d <= len + 0.02));
        let hit = match (hit_t, wall) {
            (Some((t, along, mult)), w) if w.is_none_or(|w| along <= w) => Some((tf.translation + dir * along, Some((t, mult)))),
            (_, Some(w)) => Some((tf.translation + dir * w, None)),
            _ => None,
        };
        let Some((pt, tgt)) = hit else {
            tf.translation += step;
            tf.rotate_local_z(20.0 * dt);
            continue;
        };
        let mut cue = "/Game/Audio/Knife/Cue/Bodycam_SFX_Weapon_Knife_Throw_Impact";
        if let Some((t, mult)) = tgt
            && let Ok((_, _, mut hp, _)) = targets.get_mut(t)
        {
            hp.0 -= KNIFE_DAMAGE * mult;
            cue = "/Game/Audio/Knife/Cue/Bodycam_SFX_Weapon_Knife_Throw_Impact_Gore";
            if hp.0 <= 0.0 {
                cmd.entity(t).despawn();
            }
        }
        snd.write(PlayCue { bools: vec![], cue: cue.into(), dist_cm: cam.translation().distance(pt) * 100.0 });
        // bIsStuck: the knife stays where it hit (StabDepth into the surface) and can be picked up again.
        cmd.entity(e).despawn();
        cmd.spawn((
            Pickup { row: k.row.clone(), vel: Vec3::ZERO },
            WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/guns/{}.glb", k.model)))),
            Transform::from_translation(pt + dir * KNIFE_STAB_DEPTH).with_rotation(knife_rot(k.vel)),
        ));
    }
}

/// G (IA_Drop -> GA_Character_DropItem): the equipped non-primary item is dropped as a pickup; F picks the nearest back up.
// ponytail: Drop montage + DropItem notify timing not honoured (instant); only the grenade/knife slots drop.
fn drop_pickup(
    mut cmd: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    assets: Res<AssetServer>,
    mut inv: Single<&mut Inventory>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    perk: Res<crate::perks::Perk>,
    mut pickups: Query<(Entity, &mut Transform, &mut Pickup)>,
    mut out: MessageWriter<MontageStarted>,
) {
    let dt = time.delta_secs();
    let cur = inv.current;
    if perk.locked() {
        return;
    }
    if keys.just_pressed(KeyCode::KeyG) && !inv.busy() && matches!(inv.slots[cur].kind, ItemKind::Grenade | ItemKind::Knife) && inv.slots[cur].ammo > 0 {
        let row = inv.slots[cur].name.clone();
        inv.slots[cur].ammo = 0;
        let path = format!("models/guns/{}.glb", crate::viewmodel::gun_model(&row, &inv.slots[cur].anims).unwrap_or("G67"));
        let c = cam.compute_transform();
        cmd.spawn((Pickup { row, vel: c.forward().as_vec3() * 2.0 + Vec3::Y }, WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))), Transform::from_translation(c.translation + c.forward().as_vec3() * 0.4 - Vec3::Y * 0.3)));
        inv.swap_to(0, &mut out);
    }
    // fall to the floor y=0 plane of whatever is below (simple: stop at the first y<=0.05 or after 1.2 s of motion)
    for (_, mut tf, mut p) in &mut pickups {
        if p.vel != Vec3::ZERO {
            p.vel.y -= G * dt;
            tf.translation += p.vel * dt;
            if tf.translation.y <= 0.05 {
                tf.translation.y = 0.05;
                p.vel = Vec3::ZERO;
            }
        }
    }
    if keys.just_pressed(KeyCode::KeyF) {
        let at = cam.translation();
        let near = pickups.iter().filter(|(_, t, _)| t.translation.distance(at) < 3.0).min_by(|a, b| a.1.translation.distance(at).total_cmp(&b.1.translation.distance(at)));
        if let Some((e, _, p)) = near {
            if let Some(w) = inv.slots.iter_mut().find(|w| w.name == p.row) {
                w.ammo = 1;
            }
            cmd.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radial_falloff_shape() {
        assert_eq!(falloff(300.0, 350.0, 1000.0, 1.3), 1.0);
        assert_eq!(falloff(1000.0, 350.0, 1000.0, 1.3), 0.0);
        let mid = falloff(675.0, 350.0, 1000.0, 1.3);
        assert!(mid > 0.3 && mid < 0.5, "{mid}");
    }

    #[test]
    fn toss_hits_target() {
        let (from, to) = (Vec3::new(0.0, 1.6, 0.0), Vec3::new(0.0, 0.0, -12.0));
        let v = toss_velocity(from, to, OVERHAND_SPEED).expect("reachable");
        // integrate until it passes the target's horizontal distance
        let (mut p, mut v, dt) = (from, v, 0.001);
        while p.z > to.z {
            v.y -= G * dt;
            p += v * dt;
        }
        assert!((p.y - to.y).abs() < 0.1, "{p}");
        assert!(toss_velocity(from, Vec3::new(0.0, 0.0, -100.0), OVERHAND_SPEED).is_none());
    }
}


/// Grenade_C: the FP grenade leaves the hand on release (RemoveItemInSlot). The empty slot's viewmodel model stays until the holster
/// montage ends: hide it, or it hangs in front of the camera and sinks to the feet with the holster pose.
fn hide_thrown(inv: Option<Single<&Inventory>>, mut guns: Query<&mut Visibility, With<GunRoot>>) {
    let Some(inv) = inv else { return };
    let slot = &inv.slots[inv.current];
    if slot.kind == ItemKind::Grenade && slot.ammo == 0 {
        for mut v in &mut guns {
            *v = Visibility::Hidden;
        }
    }
}
