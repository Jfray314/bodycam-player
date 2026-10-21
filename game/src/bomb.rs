//! Bomb mode (`BODYCAM_MODE=bomb`): GM_BodyBomb + AC_BombRuleComponent (the player is handed a Bombe_C in the Special slot),
//! BombZone boxes, Bombe_C / GA_TriggerBomb plant + defuse + fuse + explosion. Offline, one round at a time.
//! Plant: LMB held with the C4 drawn inside a zone plays A_FP_C4_Plant_Montage; its SendGameplayEvent notify (3.4887 s) is
//! GameplayEvent.Bomb.Initiated -> Start Fuse; releasing earlier cancels (K2_OnEndAbility bWasCancelled -> MontageStop).
//! Defuse: F held next to the armed bomb plays A_FP_C4_Defuse_Montage, notify at 6.4613 s -> Defuse. The fuse keeps counting.
//! Explode: Bombe_C Explode (mutlicast): Explosion_Flash_small + C4_Explosion_2, RadialDamageWithFalloff(1000, inner 250, outer 2000, exp 2).
//! Net: teams by player id parity (team 0 = Bombe_C.TeamHasBomb attackers, the host carries the bomb; odd ids defend). Plant and
//! defuse go out as `BombMsg` (net.rs Ev::Bomb); every machine runs its own fuse and blast (only the host's blast deals damage).
//! Offline both plant and defuse are yours. Beeps: Bombe_C loop (Ui_Hover_over_button_16 per tick, Access_Granted_8 in the last 2 s).
// ponytail: team = id parity (GM_BodyBomb/GameState objective team not ported), rounds restart 6 s after the outcome, the fuse runs
// per machine (drift = latency), beep volume ramp (MapRange 0.45..0.7) and zone PlantSound announcers not ported, zones only on
// BODYCAM_MAP=test (the lobby map has no BombZone), no W_Bomb tablet widget, the bomb stays where it was planted (no physics drop).
use crate::audio::PlayCue;
use crate::items::{load_item, BlastSpec, DetonateFor};
use crate::net::MY_ID;
use std::sync::atomic::Ordering::Relaxed;
use crate::player::Player;
use crate::weapon::{Inventory, ItemKind, Montage, MontageStarted};
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;

/// Bombe_C.FuzeTimer default (counted down once a second).
const FUSE: f32 = 60.0;
/// A_FP_C4_Plant_Montage / A_FP_C4_Defuse_Montage SendGameplayEvent notify times.
const PLANT_EVENT: f32 = 3.4887;
const DEFUSE_EVENT: f32 = 6.4613;
const DEFUSE_REACH: f32 = 2.0;
const RESTART: f32 = 6.0;
/// Bombe_C.Explode: damage 1000, RadialDamageWithFalloff inner 250 / outer 2000 cm, exponent 2.
const BOMB_BLAST: BlastSpec = BlastSpec { base: 1000.0, inner: 250.0, outer: 2000.0, exp: 2.0, flash: false, hurt: true, cue: "/Game/Ultimate_SFX/Survival_Sound_Kit_HD_Remake/Cues/C4_Explosion_2_Cue" };
/// BombZone Box half extents (m) on the test map.
const ZONE_HALF: f32 = 3.0;

pub fn wanted() -> bool {
    std::env::var("BODYCAM_MODE").is_ok_and(|v| v == "bomb")
}

#[derive(Component)]
struct BombZone;
#[derive(Component)]
struct PlantedBomb;
#[derive(Component)]
struct BombLight;
#[derive(Component)]
struct BombText;

struct Armed {
    left: f32,
    pos: Vec3,
    defusing: Option<f32>,
    /// Planter (kill credit for the blast).
    by: u8,
    /// Seconds to the next fuse beep / until the light goes off.
    next_beep: f32,
    lit: f32,
}

/// Bombe_C.FuseDelay: below it the beeps speed up.
const FUSE_DELAY: f32 = 30.0;
const BEEP: &str = "/Game/Ultimate_SFX/Ultimate_Retro_Sounds/Cues/Ui_Hover_over_button_-_10/Ui_Hover_over_button_16_Cue";
const BEEP_LAST: &str = "/Game/Ultimate_SFX/Ultimate_Retro_Sounds/Cues/Access_Granted_-_10/Access_Granted_8_Cue";
const PLANTED_CUE: &str = "/Game/Ultimate_SFX/Ultimate_Retro_Sounds/Cues/Ui_Hover_over_button_-_10/Ui_Hover_over_button_9_Cue";
const DEFUSE_ANNOUNCER: &str = "/Game/Ultimate_SFX/Announcer/BombDefuseWarning/BombDefuseWarning-01_Cue";
const ATTACKERS: u8 = 0;

/// Bomb events on the wire (net.rs Ev::Bomb).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub enum BombMsg {
    Planted { at: [f32; 3], by: u8 },
    Defused,
}
/// Local plant/defuse -> net.rs.
#[derive(Message)]
pub struct BombOut(pub BombMsg);
/// Another player's plant/defuse <- net.rs.
#[derive(Message)]
pub struct BombIn(pub BombMsg);

fn team(id: u8) -> u8 {
    id % 2
}

#[derive(Resource, Default)]
struct Round {
    given: bool,
    planting: Option<f32>,
    armed: Option<Armed>,
    /// Outcome text + seconds until the round restarts.
    over: Option<(&'static str, f32)>,
}

pub struct BombPlugin;
impl Plugin for BombPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<BombOut>().add_message::<BombIn>();
        if !wanted() {
            return;
        }
        app.init_resource::<Round>().add_systems(Startup, spawn).add_systems(Update, (give_bomb, bomb_in, plant, defuse, fuse, finish, hud).chain());
    }
}

fn spawn(mut cmd: Commands, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>, assets: Res<AssetServer>) {
    if crate::testmap::wanted() {
        // BombZone: Box overlap + the MI_BombZone floor decal -> a flat translucent orange pad.
        let m = mats.add(StandardMaterial { base_color: Color::srgba(1.0, 0.45, 0.05, 0.35), alpha_mode: AlphaMode::Blend, unlit: true, ..default() });
        for x in [-7.0, 7.0] {
            cmd.spawn((BombZone, Mesh3d(meshes.add(Cuboid::new(2.0 * ZONE_HALF, 0.02, 2.0 * ZONE_HALF))), MeshMaterial3d(m.clone()), Transform::from_xyz(x, 0.012, -4.0)));
        }
    }
    cmd.spawn((
        Text::new(""),
        TextFont { font: assets.load("fonts/Ki-Bold.otf").into(), font_size: 26.0.into(), ..default() },
        TextColor(Color::srgb(1.0, 0.55, 0.2)),
        BombText,
        Node { position_type: PositionType::Absolute, top: percent(6), left: percent(30), ..default() },
    ));
}

fn in_zone(p: Vec3, zones: &Query<&Transform, With<BombZone>>) -> bool {
    zones.iter().any(|z| (p.x - z.translation.x).abs() <= ZONE_HALF && (p.z - z.translation.z).abs() <= ZONE_HALF)
}

fn bomb_slot(inv: &Inventory) -> Option<usize> {
    inv.slots.iter().position(|w| w.kind == ItemKind::Bomb)
}

fn extra(inv: &Inventory, slot: usize, key: &str) -> Option<Montage> {
    Montage::from_value(inv.slots[slot].anims["Extra"].get(key)?)
}

/// AC_BombRuleComponent.AssignPlayerToObjective: SpawnSpecialItem(Bombe) (also again after each round) -> the host / offline player.
fn give_bomb(mut r: ResMut<Round>, inv: Option<Single<&mut Inventory>>, net: Option<Res<crate::net::Net>>) {
    let Some(mut inv) = inv else { return };
    if r.given || (net.is_some() && MY_ID.load(Relaxed) != 0) {
        return;
    }
    match bomb_slot(&inv) {
        Some(s) => inv.slots[s].ammo = 1,
        None => {
            if let Some(w) = load_item("Bomb") {
                inv.slots.push(w);
            }
        }
    }
    r.given = true;
}

/// Bombe_C Start Fuse (planted at `pos` by `by`): the bomb actor with its blinking light.
fn arm(cmd: &mut Commands, assets: &AssetServer, r: &mut Round, pos: Vec3, by: u8) {
    cmd.spawn((PlantedBomb, WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/guns/Bomb.glb"))), Transform::from_translation(pos + Vec3::Y * 0.03))).with_children(|c| {
        c.spawn((BombLight, PointLight { color: Color::srgb(1.0, 0.26, 0.0), intensity: 60_000.0, range: 5.0, ..default() }, Transform::from_xyz(0.0, 0.1, 0.0), Visibility::Hidden));
    });
    let left = std::env::var("BODYCAM_BOMBFUSE").ok().and_then(|v| v.parse().ok()).unwrap_or(FUSE); // dev: shorter fuse
    r.armed = Some(Armed { left, pos, defusing: None, by, next_beep: 0.0, lit: 0.0 });
    r.planting = None;
    info!("bomb planted at {pos:?} by player {by}");
}

/// Another player planted / defused. Bombe_C OnRep: non-owners hear the plant cue, the defenders the defuse announcer; defenders get
/// a spent C4 slot so they can play the defuse montage.
fn bomb_in(mut ev: MessageReader<BombIn>, mut cmd: Commands, assets: Res<AssetServer>, mut r: ResMut<Round>, mut inv: Option<Single<&mut Inventory>>, mut snd: MessageWriter<PlayCue>) {
    for BombIn(m) in ev.read() {
        match m {
            BombMsg::Planted { at, by } => {
                if r.armed.is_some() {
                    continue;
                }
                arm(&mut cmd, &assets, &mut r, Vec3::from(*at), *by);
                snd.write(PlayCue { bools: vec![], cue: PLANTED_CUE.into(), dist_cm: 0.0 });
                if team(MY_ID.load(Relaxed)) != team(*by) {
                    snd.write(PlayCue { bools: vec![], cue: DEFUSE_ANNOUNCER.into(), dist_cm: 0.0 });
                    if let Some(inv) = inv.as_mut().filter(|i| bomb_slot(i).is_none())
                        && let Some(mut w) = load_item("Bomb")
                    {
                        w.ammo = 0;
                        inv.slots.push(w);
                    }
                }
            }
            BombMsg::Defused => {
                if r.armed.take().is_some() {
                    r.over = Some(("BOMB DEFUSED - DEFENDERS WIN", RESTART));
                    info!("bomb defused (remote)");
                }
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn plant(
    time: Res<Time>,
    keys: Res<ButtonInput<MouseButton>>,
    mut r: ResMut<Round>,
    mut cmd: Commands,
    assets: Res<AssetServer>,
    mut inv: Single<&mut Inventory>,
    p: Single<(&Player, &Transform)>,
    zones: Query<&Transform, With<BombZone>>,
    mut out: MessageWriter<MontageStarted>,
    mut net_out: MessageWriter<BombOut>,
) {
    let Some(slot) = bomb_slot(&inv) else { return };
    let (pl, tr) = *p;
    if let Some(t) = &mut r.planting {
        *t += time.delta_secs();
        if pl.dead || !keys.pressed(MouseButton::Left) || inv.current != slot {
            r.planting = None;
            if !pl.dead && inv.current == slot {
                inv.cancel_to_draw(&mut out); // GA_TriggerBomb cancelled: MontageStop
            }
        } else if *t >= PLANT_EVENT {
            // GameplayEvent.Bomb.Initiated -> Bombe_C.Start Fuse (StartObjective, BombeActive).
            let (pos, by) = (tr.translation, MY_ID.load(Relaxed));
            arm(&mut cmd, &assets, &mut r, pos, by);
            inv.slots[slot].ammo = 0;
            net_out.write(BombOut(BombMsg::Planted { at: pos.to_array(), by }));
        }
        return;
    }
    if r.armed.is_none() && r.over.is_none() && inv.current == slot && inv.slots[slot].ammo > 0 && !inv.busy() && !pl.dead && keys.just_pressed(MouseButton::Left) && in_zone(tr.translation, &zones) {
        let m = extra(&inv, slot, "Plant");
        inv.use_item(m, &mut out);
        r.planting = Some(0.0);
    }
    // After the plant montage ends the carrier puts the (spent) item away.
    if r.armed.is_some() && inv.current == slot && !inv.busy() && inv.slots[slot].ammo == 0 && r.armed.as_ref().is_some_and(|a| a.defusing.is_none()) {
        inv.swap_to(0, &mut out);
    }
}

/// Initiate/Defuse: the interaction key (F here) held within reach of the armed bomb (Bombe_C.CanInterract: the other team only).
#[allow(clippy::too_many_arguments)]
fn defuse(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut r: ResMut<Round>,
    mut inv: Single<&mut Inventory>,
    p: Single<(&Player, &Transform)>,
    mut out: MessageWriter<MontageStarted>,
    mut net_out: MessageWriter<BombOut>,
    net: Option<Res<crate::net::Net>>,
) {
    if net.is_some() && team(MY_ID.load(Relaxed)) == ATTACKERS {
        return;
    }
    let Some(slot) = bomb_slot(&inv) else { return };
    let (pl, tr) = *p;
    let Some(a) = &mut r.armed else { return };
    let near = tr.translation.distance(a.pos) <= DEFUSE_REACH && !pl.dead;
    if let Some(t) = &mut a.defusing {
        *t += time.delta_secs();
        // (t < 0: still drawing the C4, the slot isn't current yet)
        if !near || !keys.pressed(KeyCode::KeyF) || (inv.current != slot && *t >= 0.0) {
            a.defusing = None;
            inv.slots[slot].ammo = 0;
            if !pl.dead && inv.current == slot {
                inv.cancel_to_draw(&mut out);
            }
        } else if *t >= DEFUSE_EVENT {
            // GameplayEvent.Bomb.Defused -> Bombe_C.Defuse (BombeActive = false).
            r.armed = None;
            inv.slots[slot].ammo = 0;
            r.over = Some(("BOMB DEFUSED - DEFENDERS WIN", RESTART));
            net_out.write(BombOut(BombMsg::Defused));
            info!("bomb defused");
        }
        return;
    }
    if near && keys.pressed(KeyCode::KeyF) && !inv.busy() && a.defusing.is_none() {
        inv.slots[slot].ammo = 1; // the interacting player holds the C4 for the defuse montage
        if inv.current != slot {
            inv.swap_to(slot, &mut out);
        }
        a.defusing = Some(-1.0); // starts once the draw is done
    }
    if a.defusing.is_some_and(|t| t < 0.0) && inv.current == slot && !inv.busy() {
        let m = extra(&inv, slot, "Defuse");
        inv.use_item(m, &mut out);
        a.defusing = Some(0.0);
    }
}

/// Bombe_C Loop: FuzeTimer - 1 every second; <= 0 -> Explode (Server) -> multicast.
/// Beep loop on the whole-second FuzeTimer: light on + Ui_Hover_over_button_16, light off after MapRangeClamped(FuzeTimer,
/// 0..FuseDelay -> 0..0.2) s, next beep after MapRangeClamped(.. -> 0.0001..1.0) s more (fastest 0.12 s at 3 s left). At
/// FuzeTimer <= 2 the loop plays Access_Granted_8 once and ends (pop_flow): no beeps in the last 2 s.
#[allow(clippy::too_many_arguments)]
fn fuse(
    time: Res<Time>,
    mut cmd: Commands,
    mut r: ResMut<Round>,
    mut det: MessageWriter<DetonateFor>,
    mut snd: MessageWriter<PlayCue>,
    cam: Single<&GlobalTransform, With<crate::player::BodycamCamera>>,
    bombs: Query<Entity, With<PlantedBomb>>,
    mut lights: Query<&mut Visibility, With<BombLight>>,
) {
    let Some(a) = &mut r.armed else { return };
    let dt = time.delta_secs();
    a.left -= dt;
    a.next_beep -= dt;
    a.lit -= dt;
    if a.next_beep <= 0.0 && a.left > 0.0 {
        let fuze = a.left.ceil(); // replicated FuzeTimer counts whole seconds
        let dist_cm = cam.translation().distance(a.pos) * 100.0;
        a.lit = 0.2;
        if fuze <= 2.0 {
            snd.write(PlayCue { bools: vec![], cue: BEEP_LAST.into(), dist_cm });
            a.next_beep = f32::INFINITY; // the loop ends here
        } else {
            let k = (fuze / FUSE_DELAY).clamp(0.0, 1.0);
            a.lit = 0.2 * k;
            a.next_beep = a.lit + k.max(0.0001);
            snd.write(PlayCue { bools: vec![], cue: BEEP.into(), dist_cm });
        }
    }
    for mut v in &mut lights {
        *v = if a.lit > 0.0 { Visibility::Inherited } else { Visibility::Hidden };
    }
    if a.left <= 0.0 {
        det.write(DetonateFor { at: a.pos + Vec3::Y * 0.1, spec: BOMB_BLAST, owner: a.by });
        for e in &bombs {
            cmd.entity(e).despawn();
        }
        r.armed = None;
        r.over = Some(("BOMB DETONATED - ATTACKERS WIN", RESTART));
        info!("bomb exploded");
    }
}

/// Round over: after a pause the (defused) bomb goes away and the carrier gets a fresh Bombe_C.
fn finish(time: Res<Time>, mut cmd: Commands, mut r: ResMut<Round>, bombs: Query<Entity, With<PlantedBomb>>) {
    if let Some((_, t)) = &mut r.over {
        *t -= time.delta_secs();
        if *t <= 0.0 {
            r.over = None;
            r.given = false;
            for e in &bombs {
                cmd.entity(e).despawn();
            }
        }
    }
}

fn hud(net: Option<Res<crate::net::Net>>, r: Res<Round>, inv: Option<Single<&Inventory>>, p: Single<&Transform, With<Player>>, zones: Query<&Transform, With<BombZone>>, mut q: Single<&mut Text, With<BombText>>) {
    q.0 = if let Some((msg, _)) = r.over {
        msg.into()
    } else if let Some(a) = &r.armed {
        match a.defusing {
            Some(t) if t >= 0.0 => format!("DEFUSING {:.0}%   BOMB {:.0}", t / DEFUSE_EVENT * 100.0, a.left.max(0.0).ceil()),
            _ if net.is_some() && team(MY_ID.load(Relaxed)) == ATTACKERS => format!("BOMB PLANTED {:.0}   (defend it)", a.left.max(0.0).ceil()),
            _ => format!("BOMB PLANTED {:.0}   (hold F next to it to defuse)", a.left.max(0.0).ceil()),
        }
    } else if let Some(t) = r.planting {
        format!("PLANTING {:.0}%", t / PLANT_EVENT * 100.0)
    } else if let Some(inv) = inv.filter(|i| bomb_slot(i).is_some_and(|s| i.slots[s].ammo > 0)) {
        let n = bomb_slot(&inv).unwrap() + 1;
        if in_zone(p.translation, &zones) { format!("In bomb zone: draw C4 (key {n}), hold LMB to plant") } else { format!("Carrying C4 (key {n}): reach a bomb zone") }
    } else {
        String::new()
    };
}
