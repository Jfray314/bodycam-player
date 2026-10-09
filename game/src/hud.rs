//! In-game player UI. Bodycam has no permanent HUD (W_PlayerHUD is only a voice-chat icon).
//! Ammo is only seen through HUD_Magazine, a widget on the gun shown during a mag check (IA_InspectAmmos):
//! current rounds in Ki font, colour lerped by MapRangeClamped(ammo / max), with a Fade animation.
//! Visible between the montage's SetMagCheckUIVisibility notifies.
use crate::weapon::{ActionKind, Inventory};
use bevy::prelude::*;

// ponytail: screen-space stand-in until the viewmodel exists; then attach to the gun's widget component.
// ponytail: fade length eyeballed from HUD_Magazine's Fade track.
const FADE_SECS: f32 = 0.2;

#[derive(Component)]
struct MagText;

pub struct HudPlugin;
impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn).add_systems(Update, update);
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

fn update(inv: Option<Single<&Inventory>>, mut q: Single<(&mut Text, &mut TextColor), With<MagText>>) {
    let Some(inv) = inv else { return };
    let w = &inv.slots[inv.current];
    let cap = w.stats.ammo_capacity.unwrap_or(30.0).max(1.0);
    let ratio = (w.ammo as f32 / cap).clamp(0.0, 1.0);
    let alpha = match &inv.action {
        Some(a) if a.kind == ActionKind::MagCheck => a
            .montage
            .mag_ui_window()
            .map_or(0.0, |(show, hide)| ((a.t - show) / FADE_SECS).min((hide - a.t) / FADE_SECS).clamp(0.0, 1.0)),
        _ => 0.0,
    };
    let (text, color) = &mut *q;
    text.0 = w.ammo.to_string();
    color.0 = Color::srgb(1.0, 0.25, 0.2).mix(&Color::WHITE, ratio).with_alpha(alpha);
}
