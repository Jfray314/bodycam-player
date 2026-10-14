//! In-game player UI. Bodycam has no permanent HUD (W_PlayerHUD is only a voice-chat icon).
//! Ammo is only seen through HUD_Magazine, a widget on the gun shown during a mag check (IA_InspectAmmos):
//! current rounds in Ki font, colour lerped by MapRangeClamped(ammo / max), with a Fade animation.
//! Visible between the montage's SetMagCheckUIVisibility notifies.
use crate::viewmodel::{root_of_entity, GunRoot};
use crate::weapon::{ActionKind, Inventory};
use bevy::prelude::*;

// HUD_Magazine is a WidgetComponent (Space = Screen, NODE_AddWidgetComponent-0 in WEP) attached to the Magazine attachment's scene component
// (SetMagCheckHudVisibility): a screen-space widget drawn at the projected magazine position; with no magazine it hangs on the gun body.
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
        Node { position_type: PositionType::Absolute, bottom: percent(22), left: percent(56), ..default() }, // fallback until the first projection
    ));
}

fn update(
    inv: Option<Single<&Inventory>>,
    mut q: Single<(&mut Text, &mut TextColor, &mut Node), With<MagText>>,
    cam: Single<(&Camera, &GlobalTransform), With<crate::player::BodycamCamera>>,
    guns: Query<(Entity, &GunRoot)>,
    joints: Query<(Entity, &Name, &GlobalTransform)>,
    parents: Query<&ChildOf>,
) {
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
    let (text, color, node) = &mut *q;
    // widget anchor: the Magazine bone (the mag part rides it through reloads), else the gun root
    if alpha > 0.0
        && let Some((gun, _)) = guns.iter().next()
    {
        let pick = |n: &str| joints.iter().find(|(e, nm, _)| nm.as_str().eq_ignore_ascii_case(n) && root_of_entity(*e, gun, &parents));
        if let Some((_, _, gt)) = pick("magazine").or_else(|| pick("root"))
            && let Ok(p) = cam.0.world_to_viewport(cam.1, gt.translation())
        {
            node.left = px(p.x - 24.0);
            node.top = px(p.y - 20.0);
            node.bottom = Val::Auto;
        }
    }
    text.0 = w.ammo.to_string();
    color.0 = Color::srgb(1.0, 0.25, 0.2).mix(&Color::WHITE, ratio).with_alpha(alpha);
}
