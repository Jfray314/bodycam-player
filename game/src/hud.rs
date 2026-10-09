//! Player HUD (BP_BodycamGameHUD in-game part only): ammo / mags / weapon name, REC + timestamp overlay.
use crate::weapon::Inventory;
use bevy::prelude::*;

#[derive(Component)]
struct AmmoText;
#[derive(Component)]
struct RecText;

pub struct HudPlugin;
impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
    }
}

fn spawn(mut cmd: Commands) {
    let font = TextFont { font_size: 22.0.into(), ..default() };
    cmd.spawn((Text::new(""), font.clone(), RecText, Node { position_type: PositionType::Absolute, top: px(16), left: px(16), ..default() }));
    cmd.spawn((Text::new(""), font, AmmoText, Node { position_type: PositionType::Absolute, bottom: px(16), right: px(16), ..default() }));
}

fn update(
    time: Res<Time>,
    inv: Option<Single<&Inventory>>,
    mut ammo: Single<&mut Text, (With<AmmoText>, Without<RecText>)>,
    mut rec: Single<&mut Text, With<RecText>>,
) {
    let t = time.elapsed_secs();
    let dot = if (t * 2.0) as u32 % 2 == 0 { "\u{25CF}" } else { " " };
    rec.0 = format!("{dot} REC  {:02}:{:02}:{:02}", (t / 3600.0) as u32, (t / 60.0) as u32 % 60, t as u32 % 60);
    if let Some(inv) = inv {
        let w = &inv.slots[inv.current];
        let state = if inv.reloading > 0.0 { "  RELOADING" } else { "" };
        ammo.0 = format!("{}  {} / {} mags{}", w.name, w.ammo, w.mags, state);
    }
}
