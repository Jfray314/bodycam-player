//! In-game player UI. Bodycam has no permanent HUD (W_PlayerHUD is only a voice-chat icon).
//! Ammo is only seen through HUD_Magazine, a widget on the gun shown during a mag check (IA_InspectAmmos):
//! current rounds in Ki font, colour lerped by MapRangeClamped(ammo / max), with a Fade animation.
use crate::weapon::Inventory;
use bevy::prelude::*;

// ponytail: screen-space stand-in until the viewmodel exists; then attach to the gun's widget component.
// ponytail: duration/fade eyeballed; take from A_FP_<gun>_MagCheck_* length and HUD_Magazine Fade track.
const SHOW_SECS: f32 = 2.4;
const FADE_SECS: f32 = 0.3;

#[derive(Component)]
struct MagText;

/// Seconds left on the mag-check display.
#[derive(Resource, Default)]
pub struct MagCheck(pub f32);

pub struct HudPlugin;
impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MagCheck>().add_systems(Startup, spawn).add_systems(Update, (input, update).chain());
    }
}

fn spawn(mut cmd: Commands, assets: Res<AssetServer>) {
    cmd.spawn((
        Text::new(""),
        TextFont { font: assets.load("fonts/Ki-Bold.otf").into(), font_size: 34.0.into(), ..default() },
        TextColor(Color::NONE),
        MagText,
        Node { position_type: PositionType::Absolute, bottom: percent(22), left: percent(56), ..default() },
    ));
}

// ponytail: T until IMC_KBM_CharacterOnFoot's key for IA_InspectAmmos is decoded by the asset reader.
fn input(keys: Res<ButtonInput<KeyCode>>, inv: Option<Single<&Inventory>>, mut mc: ResMut<MagCheck>) {
    if keys.just_pressed(KeyCode::KeyT) && inv.is_some_and(|i| i.reloading <= 0.0) {
        mc.0 = SHOW_SECS;
    }
}

fn update(time: Res<Time>, mut mc: ResMut<MagCheck>, inv: Option<Single<&Inventory>>, mut q: Single<(&mut Text, &mut TextColor), With<MagText>>) {
    mc.0 = (mc.0 - time.delta_secs()).max(0.0);
    let Some(inv) = inv else { return };
    let w = &inv.slots[inv.current];
    let cap = w.stats.ammo_capacity.unwrap_or(30.0).max(1.0);
    let ratio = (w.ammo as f32 / cap).clamp(0.0, 1.0);
    let alpha = (mc.0 / FADE_SECS).min((SHOW_SECS - mc.0) / FADE_SECS).clamp(0.0, 1.0);
    let (text, color) = &mut *q;
    text.0 = w.ammo.to_string();
    color.0 = Color::srgb(1.0, 0.25, 0.2).mix(&Color::WHITE, ratio).with_alpha(alpha);
}
