//! Non-gun inventory items: M67/G67 frag + flashbang (GA_Character_Aim_Grenade / GA_ThrowGrenade / Grenade_C / G67_C / Flash_C),
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

pub const SLOT_GRENADE: usize = 2;
pub const SLOT_MELEE: usize = 3;
pub const SLOT_PERK: usize = 4;

/// assets/items/<Row>.json (tools/weapon_data.py: DT_PerkItems rows with AnimationData).
pub fn load_item(row: &str) -> Option<Weapon> {
    let kind = if row.starts_with("Grenade") {
        ItemKind::Grenade
    } else if row == "Kamikaze" {
        ItemKind::Drone
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
    [&cfg.explosive, &cfg.melee, &cfg.gadget].into_iter().filter_map(|r| load_item(r)).collect()
}

/// Tablet names (the M67 label is the loadout screen's: the row is the G67).
pub fn display_name(row: &str) -> &str {
    match row {
        "Grenade G67" => "M67 Frag Grenade",
        "Grenade Flash" => "Flashbang",
        "Kamikaze" => "Kamikaze",
        "Knife" => "Knife",
        r => r,
    }
}

/// BP defaults: Grenade_C (OverhandLaunchSpeed 1600, MaxThrowDistance 1500, ...), G67_C / Flash_C explosion data.
const OVERHAND_SPEED: f32 = 16.0;
const MAX_THROW_DIST: f32 = 15.0;
const G: f32 = 9.8;
const THROW_DELAY: f32 = 0.2; // Grenade_C.Throw: Delay 0.2 before the physics impulse
const FUSE: f32 = 3.0; // DT_GrenadeInfo FuseDelay (counted from the Throw event, which fires before the 0.2 s delay)
const KNIFE_RANGE: f32 = 1.25; // Knife_C.CloseAttackRange 125 cm
const KNIFE_RADIUS: f32 = 0.5; // sphere trace radius 50 cm
const KNIFE_DAMAGE: f32 = 100.0; // GE_KnifeStat_Default Damage

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
    pub flash: bool,
}
/// G67_C: (1.0, 350, 1000, 1.3) x GE_GrenadeStat_G67 Damage 200.
pub const G67_BLAST: BlastSpec = BlastSpec { base: 200.0, inner: 350.0, outer: 1000.0, exp: 1.3, flash: false };
/// Flash_C: (1.0, 250, 1000, 2.0) x GE_GrenadeStat_Flash Damage 50; the flash applies GE_Flashed instead.
pub const FLASH_BLAST: BlastSpec = BlastSpec { base: 50.0, inner: 250.0, outer: 1000.0, exp: 2.0, flash: true };
/// GA_ExplodePerk: ExplosionDamage 200, Inner 200, Outer 1000, exponent 2.0.
pub const DRONE_BLAST: BlastSpec = BlastSpec { base: 200.0, inner: 200.0, outer: 1000.0, exp: 2.0, flash: false };

/// Ask for a blast (grenade fuse, drone crash).
#[derive(Message)]
pub struct Detonate {
    pub at: Vec3,
    pub spec: BlastSpec,
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
    /// Knife combo index (Fire list).
    combo: usize,
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
    flash: bool,
    hits: u32,
    spin: Vec3,
    rest: bool,
}

#[derive(Component)]
struct Blast {
    t: f32,
    life: f32,
    size: f32,
}

/// A dropped item (GA_Character_DropItem -> DropItem notify -> DropEquippedItem): F picks it up again.
#[derive(Component)]
struct Pickup {
    row: String,
    vel: Vec3,
}

#[derive(Resource)]
struct BlastMesh(Handle<Mesh>);

pub struct ItemsPlugin;
impl Plugin for ItemsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Items>().add_message::<Detonate>().add_message::<PlayCue>().add_systems(Startup, |mut cmd: Commands, mut meshes: ResMut<Assets<Mesh>>| cmd.insert_resource(BlastMesh(meshes.add(Sphere::new(0.5).mesh().ico(3).unwrap())))).add_systems(Update, (grenade_input, knife_input, projectiles, detonate, blasts, tag_meshes, drop_pickup).chain());
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
            }
        }
        GState::Pulling => {
            if !busy {
                if rmb {
                    st.grenade = GState::Ready;
                    st.hold = 0.0;
                } else {
                    let m = extra(&inv.slots[cur], "CancelOverhand");
                    inv.use_item(m, &mut out);
                    st.grenade = GState::Idle;
                }
            }
        }
        GState::Ready => {
            if !rmb {
                let m = extra(&inv.slots[cur], "CancelOverhand");
                inv.use_item(m, &mut out);
                st.grenade = GState::Idle;
            } else if mouse.just_pressed(MouseButton::Left) {
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
                let flash = inv.slots[cur].name.contains("Flash");
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
                let path = if flash { "models/guns/Flashbang.glb" } else { "models/guns/G67.glb" };
                cmd.spawn((
                    Grenade { vel, fuse: FUSE - THROW_DELAY, flash, hits: 0, spin: Vec3::new(9.0, 3.0, 7.0), rest: false },
                    WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(path))),
                    Transform::from_translation(start),
                ));
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
    mut out: MessageWriter<Detonate>,
    mut snd: MessageWriter<PlayCue>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
) {
    let dt = time.delta_secs().min(0.05);
    for (e, mut tf, mut g) in &mut q {
        g.fuse -= dt;
        if g.fuse <= 0.0 {
            out.write(Detonate { at: tf.translation, spec: if g.flash { FLASH_BLAST } else { G67_BLAST } });
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
                let vn = g.vel.dot(n) * n;
                let vt = g.vel - vn;
                if g.vel.length() > 3.0 {
                    g.hits += 1; // Hit event only for speed > 300 cm/s
                    snd.write(PlayCue { cue: "/Game/BallisticsVFX/SFX/Impacts/Grenade/Grenade_Impact".into(), dist_cm: cam.translation().distance(pt) * 100.0 });
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

#[allow(clippy::too_many_arguments)]
fn detonate(
    mut cmd: Commands,
    mut ev: MessageReader<Detonate>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    mut targets: Query<(Entity, &GlobalTransform, &mut Health)>,
    player: Single<&GlobalTransform, With<Player>>,
    mut boom: MessageWriter<Explosion>,
    mut flash: MessageWriter<Flashbang>,
    mut dmg: MessageWriter<PlayerDamage>,
    mut snd: MessageWriter<PlayCue>,
    blast: Res<BlastMesh>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    for d in ev.read() {
        explode(&mut cmd, d.at, d.spec, &mut ray, &ignore, &mut targets, player.translation() + Vec3::Y * 1.6, &mut boom, &mut flash, &mut dmg, &mut snd, &blast.0, &mut mats);
    }
}

#[allow(clippy::too_many_arguments)]
fn explode(
    cmd: &mut Commands,
    at: Vec3,
    spec: BlastSpec,
    ray: &mut MeshRayCast,
    ignore: &Query<(), With<IgnoreTraces>>,
    targets: &mut Query<(Entity, &GlobalTransform, &mut Health)>,
    head: Vec3,
    boom: &mut MessageWriter<Explosion>,
    flash: &mut MessageWriter<Flashbang>,
    dmg: &mut MessageWriter<PlayerDamage>,
    snd: &mut MessageWriter<PlayCue>,
    sphere: &Handle<Mesh>,
    mats: &mut Assets<StandardMaterial>,
) {
    let BlastSpec { base, inner, outer, exp, flash: flash_gren } = spec;
    let origin = at + Vec3::Y * 0.5; // BFL call uses ActorLocation + (0,0,50)
    let blocked = |ray: &mut MeshRayCast, to: Vec3, ignore: &Query<(), With<IgnoreTraces>>, targets: &Query<(Entity, &GlobalTransform, &mut Health)>| {
        let d = to - origin;
        let filter = |e: Entity| !ignore.contains(e) && !targets.contains(e);
        Dir3::new(d).ok().is_some_and(|dir| ray.cast_ray(Ray3d::new(origin, dir), &MeshRayCastSettings::default().with_filter(&filter)).first().is_some_and(|(_, h)| h.distance < d.length() - 0.15))
    };
    if !flash_gren {
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
    } else {
        flash.write(Flashbang { at });
    }
    let dp = head.distance(origin) * 100.0; // the body, not the (drone) camera
    if !blocked(ray, head, ignore, targets) {
        let f = falloff(dp, inner, outer, exp);
        if f > 0.0 && !flash_gren {
            dmg.write(PlayerDamage(base * f));
        }
    } else if flash_gren {
        // flash through geometry is still occluded by walls in the game's visibility check; here: no whiteout when blocked is not modelled (message already sent)
    }
    // Visuals + sound: SpawnEmitterAtLocation Explosion_Frag_small / Explosion_Flash_small (particle systems not ported: flash ball + light).
    let (col, size, life) = if flash_gren { (Color::srgb(1.0, 1.0, 0.95), 3.0, 0.12) } else { (Color::srgb(1.0, 0.55, 0.15), 2.2, 0.4) };
    cmd.spawn((
        Mesh3d(sphere.clone()),
        MeshMaterial3d(mats.add(StandardMaterial { base_color: col, emissive: LinearRgba::from(col) * 40.0, unlit: true, alpha_mode: AlphaMode::Add, ..default() })),
        Transform::from_translation(at + Vec3::Y * 0.2).with_scale(Vec3::splat(0.2)),
        Blast { t: 0.0, life, size },
        IgnoreTraces,
    ));
    cmd.spawn((PointLight { intensity: 8_000_000.0, range: 25.0, color: col, ..default() }, Transform::from_translation(at + Vec3::Y * 0.5), Blast { t: 0.0, life: life + 0.1, size: 0.0 }));
    // G67_C: AC_Detonation.DetonationSound = Bodycam_Grenade_G67; Flash_C: Explosions_flash.
    let cue = if flash_gren { "/Game/BallisticsVFX/SFX/Explodes/Explosions_flash" } else { "/Game/Audio/Grenade/Cue/Bodycam_Grenade_G67" };
    snd.write(PlayCue { cue: cue.into(), dist_cm: dp });
}

fn blasts(mut cmd: Commands, time: Res<Time>, mut q: Query<(Entity, &mut Blast, &mut Transform, Option<&mut PointLight>)>) {
    for (e, mut b, mut tf, light) in &mut q {
        b.t += time.delta_secs();
        let k = (b.t / b.life).clamp(0.0, 1.0);
        if b.size > 0.0 {
            tf.scale = Vec3::splat(0.2 + (b.size - 0.2) * (1.0 - (1.0 - k).powi(2)));
        }
        if let Some(mut l) = light {
            l.intensity = 8_000_000.0 * (1.0 - k);
        }
        if k >= 1.0 {
            cmd.entity(e).despawn();
        }
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
) {
    let dt = time.delta_secs();
    if perk.locked() {
        return;
    }
    let cur = inv.current;
    let on_knife = cur == SLOT_MELEE && inv.slots.get(cur).is_some_and(|w| w.kind == ItemKind::Knife);
    let knife_slot = inv.slots.get(SLOT_MELEE).is_some_and(|w| w.kind == ItemKind::Knife);
    if p.dead || !knife_slot {
        return;
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
    } else if on_knife && st.quick.is_none() && mouse.just_pressed(MouseButton::Left) && !inv.busy() {
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
                snd.write(PlayCue { cue: "/Game/Audio/Knife/Cue/Bodycam_SFX_Weapon_Knife_Attack_Gore".into(), dist_cm: 60.0 }); // MultiCast-PlayHitSound
                if hp.0 <= 0.0 {
                    cmd.entity(e).despawn();
                }
            }
        }
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
        let path = match row.as_str() {
            "Knife" => "models/guns/Knife.glb",
            r if r.contains("Flash") => "models/guns/Flashbang.glb",
            _ => "models/guns/G67.glb",
        };
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
