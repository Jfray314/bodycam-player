//! Bomb mode (`BODYCAM_MODE=bomb`): GM_BodyBomb + AC_BombRuleComponent (the player is handed a Bombe_C in the Special slot),
//! BombZone boxes, Bombe_C / GA_TriggerBomb plant + defuse + fuse + explosion. Offline, one round at a time.
//! Plant: LMB held with the C4 drawn inside a zone plays A_FP_C4_Plant_Montage; its SendGameplayEvent notify (3.4887 s) is
//! GameplayEvent.Bomb.Initiated -> Start Fuse; releasing earlier cancels (K2_OnEndAbility bWasCancelled -> MontageStop).
//! Defuse: F held next to the armed bomb plays A_FP_C4_Defuse_Montage, notify at 6.4613 s -> Defuse. The fuse keeps counting.
//! Explode: Bombe_C Explode (mutlicast): Explosion_Flash_small + C4_Explosion_2, RadialDamageWithFalloff(1000, inner 250, outer 2000, exp 2).
// ponytail: no replication (net.rs would need plant/defuse/explode events), no teams/rounds beyond restart 6 s after the outcome, no beep/announcer
// audio (Ui_Hover_over_button_16 beeps, BombPlantedB announcer: cues not baked), zones only on BODYCAM_MAP=test (the lobby map has no BombZone),
// no W_Bomb tablet widget, the planted bomb stays where the player stood (Bombe_C physics drop not modelled).
use crate::items::{load_item, BlastSpec, Detonate};
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
const BOMB_BLAST: BlastSpec = BlastSpec { base: 1000.0, inner: 250.0, outer: 2000.0, exp: 2.0, flash: false };
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
        if !wanted() {
            return;
        }
        app.init_resource::<Round>().add_systems(Startup, spawn).add_systems(Update, (give_bomb, plant, defuse, fuse, finish, hud).chain());
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

/// AC_BombRuleComponent.AssignPlayerToObjective: SpawnSpecialItem(Bombe) (also again after each round).
fn give_bomb(mut r: ResMut<Round>, inv: Option<Single<&mut Inventory>>) {
    let Some(mut inv) = inv else { return };
    if r.given {
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
            r.planting = None;
            let pos = Vec3::new(tr.translation.x, tr.translation.y, tr.translation.z);
            cmd.spawn((PlantedBomb, WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset("models/guns/Bomb.glb"))), Transform::from_translation(pos + Vec3::Y * 0.03)))
                .with_children(|c| {
                    c.spawn((BombLight, PointLight { color: Color::srgb(1.0, 0.26, 0.0), intensity: 60_000.0, range: 5.0, ..default() }, Transform::from_xyz(0.0, 0.1, 0.0), Visibility::Hidden));
                });
            inv.slots[slot].ammo = 0;
            let left = std::env::var("BODYCAM_BOMBFUSE").ok().and_then(|v| v.parse().ok()).unwrap_or(FUSE); // dev: shorter fuse
            r.armed = Some(Armed { left, pos, defusing: None });
            info!("bomb planted at {pos:?}");
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

/// Initiate/Defuse: the interaction key (F here) held within reach of the armed bomb.
fn defuse(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut r: ResMut<Round>, mut inv: Single<&mut Inventory>, p: Single<(&Player, &Transform)>, mut out: MessageWriter<MontageStarted>) {
    let Some(slot) = bomb_slot(&inv) else { return };
    let (pl, tr) = *p;
    let Some(a) = &mut r.armed else { return };
    let near = tr.translation.distance(a.pos) <= DEFUSE_REACH && !pl.dead;
    if let Some(t) = &mut a.defusing {
        *t += time.delta_secs();
        if !near || !keys.pressed(KeyCode::KeyF) || inv.current != slot {
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
fn fuse(
    time: Res<Time>,
    mut cmd: Commands,
    mut r: ResMut<Round>,
    mut det: MessageWriter<Detonate>,
    bombs: Query<Entity, With<PlantedBomb>>,
    mut lights: Query<&mut Visibility, With<BombLight>>,
) {
    let Some(a) = &mut r.armed else { return };
    a.left -= time.delta_secs();
    // PointLight blink: RetriggerableDelay MapRangeClamped(FuzeTimer, 0..FuseDelay 30 -> 0.0001..1.0) s, lit 0.2 s max.
    let period = (a.left / 30.0).clamp(0.05, 1.0);
    for mut v in &mut lights {
        *v = if (time.elapsed_secs() % period) < 0.12 { Visibility::Inherited } else { Visibility::Hidden };
    }
    if a.left <= 0.0 {
        det.write(Detonate { at: a.pos + Vec3::Y * 0.1, spec: BOMB_BLAST });
        for e in &bombs {
            cmd.entity(e).despawn();
        }
        r.armed = None;
        r.over = Some(("BOMB DETONATED - ATTACKERS WIN", RESTART));
        info!("bomb exploded");
    }
}

/// Round over: after a pause the carrier gets a fresh Bombe_C.
fn finish(time: Res<Time>, mut r: ResMut<Round>) {
    if let Some((_, t)) = &mut r.over {
        *t -= time.delta_secs();
        if *t <= 0.0 {
            r.over = None;
            r.given = false;
        }
    }
}

fn hud(r: Res<Round>, inv: Option<Single<&Inventory>>, p: Single<&Transform, With<Player>>, zones: Query<&Transform, With<BombZone>>, mut q: Single<&mut Text, With<BombText>>) {
    q.0 = if let Some((msg, _)) = r.over {
        msg.into()
    } else if let Some(a) = &r.armed {
        match a.defusing {
            Some(t) if t >= 0.0 => format!("DEFUSING {:.0}%   BOMB {:.0}", t / DEFUSE_EVENT * 100.0, a.left.max(0.0).ceil()),
            _ => format!("BOMB PLANTED {:.0}   (hold F next to it to defuse)", a.left.max(0.0).ceil()),
        }
    } else if let Some(t) = r.planting {
        format!("PLANTING {:.0}%", t / PLANT_EVENT * 100.0)
    } else if let Some(inv) = inv.filter(|i| bomb_slot(i).is_some()) {
        let n = bomb_slot(&inv).unwrap() + 1;
        if in_zone(p.translation, &zones) { format!("In bomb zone: draw C4 (key {n}), hold LMB to plant") } else { format!("Carrying C4 (key {n}): reach a bomb zone") }
    } else {
        String::new()
    };
}
