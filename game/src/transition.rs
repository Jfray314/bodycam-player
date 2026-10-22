//! Camera transitions (the C++ CameraTransitionSubsystem broadcasts Camera.Transition.Connected / .Disconnected) and the
//! loading screen, rebuilt from W_ConnectionMenu and W_LoadingMenu (UI/Game/Player/Submenus, UI/Menus/Loading):
//! - Disconnected (death): DeactivatePanel, 1.5 s at rate 1 (DeactivatePanelInstant on InstantKill): at 1.0 s the VHS
//!   background, BackgroundBlurNormal (BlurStrength 5) and red "CAM DISCONNECTED" become visible and stay.
//! - Connected (respawn, and after loading): ActivatePanel, 1.5 s: "CAM DISCONNECTED" until 0.75 s, green "CAM CONNECTED"
//!   0.75..1.35 s, blur 5 -> 1 at 1.35 s -> 0 at 1.5 s, VHS background until 1.5 s.
//! - Loading: VHS background at ColorAndOpacity 0.094, blur 5, "CAM DISCONNECTED", battery + "BODY EARLY ACCESS" + date +
//!   versions + REC on top, tip + "WAITING FOR CONNECTION..." + loading bar at the bottom (1920x1080 design px).
//! No sounds: neither widget has an audio track on these animations and the VHS MediaTexture carries no audio.
//! VHS background = the real movie's frames (vhs.wesl).
use bevy::asset::embedded_asset;
use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;
use bevy::shader::ShaderRef;

/// Camera.Transition.Connected (true) / Disconnected (false); `instant` = Camera.Transition.Data.InstantKill.
#[derive(Message)]
pub struct CamTransition {
    pub connected: bool,
    pub instant: bool,
}

/// A new level is loading (level.rs): the loading screen comes back up until it is ready.
#[derive(Message)]
pub struct Reload;

#[derive(Resource)]
enum Mode {
    Loading(f32),
    Disconnected { t: f32, instant: bool },
    Connected(f32),
    Live,
}

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct VhsMat {
    #[uniform(0)]
    p: Vec4,
    #[texture(1)]
    #[sampler(2)]
    frames: Handle<Image>,
}
impl UiMaterial for VhsMat {
    fn fragment_shader() -> ShaderRef {
        "embedded://game/vhs.wesl".into()
    }
}

#[derive(Component)]
struct Root;
#[derive(Component)]
struct Disc;
#[derive(Component)]
struct Conn;
#[derive(Component)]
struct LoadingUi;
#[derive(Component)]
struct RecDot;
#[derive(Component)]
struct Bar(usize);
/// "CAM DISCONNECTED" (Ki 34) and the tip (Ki 18) draw larger in the game than their design sizes (measured on its 1080p
/// loading screen: 27 and 13.8 px per glyph); the 24 / 32 px texts match as they are.
const CAM_PX: f32 = 45.0;
const TIP_PX: f32 = 24.0;
const SEGMENTS: usize = 28;
#[derive(Component)]
struct BlurCopy;
#[derive(Component)]
struct BlurMain;
/// Font size in 1920x1080 design px.
#[derive(Component)]
struct Fs(f32);

pub struct TransitionPlugin;
impl Plugin for TransitionPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "vhs.wesl");
        // dev screenshots are timed from viewmodel-ready: skip the loading screen unless asked for
        let skip = std::env::var_os("BODYCAM_SCREENSHOT").is_some() && std::env::var_os("BODYCAM_LOADING").is_none();
        app.add_plugins(UiMaterialPlugin::<VhsMat>::default())
            .add_message::<CamTransition>()
            .add_message::<Reload>()
            .insert_resource(if skip { Mode::Live } else { Mode::Loading(0.0) })
            .add_systems(Startup, spawn)
            .add_systems(Update, (drive, font_scale));
    }
}

/// Design px -> viewport height units.
fn dp(px: f32) -> Val {
    Val::Vh(px / 10.8)
}

fn text(cmd: &mut ChildSpawnerCommands, font: &Handle<Font>, s: &str, size: f32, c: Color, extra: impl Bundle) {
    cmd.spawn((Text::new(s), TextFont { font: font.clone().into(), font_size: size.into(), ..default() }, TextColor(c), Fs(size), extra));
}

fn spawn(mut cmd: Commands, assets: Res<AssetServer>, mut mats: ResMut<Assets<VhsMat>>) {
    let ki: Handle<Font> = assets.load("fonts/Ki-Bold.otf"); // Ki_Font "Default" typeface
    // TS_Body24Grey / the tip rich text: the lighter Ki weight (the game's loading screen header is thinner than the headlines)
    let kim: Handle<Font> = assets.load("ui/Ki-Medium.otf");
    // Vhs_01.bk2, all 5711 frames as luma cells (vhs.wesl); data texture, linear
    let frames: Handle<Image> = assets.load_builder().with_settings(|s: &mut bevy::image::ImageLoaderSettings| s.is_srgb = false).load("textures/fx/vhs_01_atlas.png");
    let grey24 = Color::linear_rgb(0.701102, 0.701102, 0.701102); // TS_Body24Grey
    let dark32 = Color::linear_rgb(0.286458, 0.286458, 0.286458); // TS_Headline32DarkGrey
    let tips: Vec<String> = serde_json::from_str(include_str!("../../assets/ui_tips.json")).unwrap_or_default();
    let tip = tips.get(rand::random_range(0..tips.len().max(1))).cloned().unwrap_or_default();
    let full = Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() };
    cmd.spawn((full.clone(), GlobalZIndex(100), Pickable::IGNORE, Visibility::Hidden, Root)).with_children(|r| {
        r.spawn((full.clone(), MaterialNode(mats.add(VhsMat { p: Vec4::new(0.0, 1.0, 5.0, 0.0), frames: frames.clone() }))));
        // OverlaySlot HAlign/VAlign_Center, Ki 34, ToUpper
        let centre = Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() };
        // The panel's BackgroundBlur covers the text too (blurred "CAM DISCONNECTED" on the loading screen): offset copies
        // (BlurCopy) fade in with the blur strength. ponytail: 8 copies on a 5 px ring stand in for the real blur kernel.
        let blurred = |c: &mut ChildSpawnerCommands, s: &str, col: Color, marker: (Visibility,)| {
            c.spawn((Node::default(), marker.0)).with_children(|b| {
                for k in 0..8 {
                    let a = k as f32 * std::f32::consts::TAU / 8.0;
                    b.spawn(Node { position_type: PositionType::Absolute, left: dp(5.0 * a.cos()), top: dp(5.0 * a.sin()), ..default() })
                        .with_children(|o| text(o, &ki, s, CAM_PX, col.with_alpha(0.0), BlurCopy));
                }
                text(b, &ki, s, CAM_PX, col, BlurMain);
            })
            .id()
        };
        r.spawn(centre.clone()).with_children(|c| {
            let e = blurred(c, "CAM DISCONNECTED", Color::linear_rgb(1.0, 0.0, 0.0), (Visibility::Hidden,));
            c.commands().entity(e).insert(Disc);
        });
        r.spawn(centre).with_children(|c| {
            let e = blurred(c, "CAM CONNECTED", Color::linear_rgb(0.0, 1.0, 0.144128), (Visibility::Hidden,));
            c.commands().entity(e).insert(Conn);
        });
        // W_LoadingMenu VerticalBox_68: 68.79 px spacer, top row, fill, bottom row, 68.79 px spacer
        r.spawn((
            Node { flex_direction: FlexDirection::Column, padding: UiRect::vertical(dp(68.786511)), ..full.clone() },
            LoadingUi,
        ))
        .with_children(|v| {
            v.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, padding: UiRect::horizontal(dp(81.701225)), ..default() }).with_children(|h| {
                // W_BatteryIndicator draws M_UI_BatteryIndicator (graph compiled away; matched to the game's screen): an outlined
                // cell at BatteryCharge 0.8 with the terminal nub on the right
                h.spawn(Node { width: dp(44.0), height: dp(22.0), border: UiRect::all(dp(2.5)), padding: UiRect::all(dp(2.5)), ..default() })
                    .insert(BorderColor::all(Color::WHITE))
                    .with_children(|b| {
                        b.spawn((Node { width: percent(80), height: percent(100), ..default() }, BackgroundColor(Color::WHITE)));
                    });
                h.spawn((Node { width: dp(4.0), height: dp(10.0), ..default() }, BackgroundColor(Color::WHITE)));
                h.spawn(Node { flex_grow: 1.0, ..default() });
                text(h, &kim, "BODY EARLY ACCESS", 24.0, grey24, ());
                h.spawn(Node { width: dp(10.0), ..default() });
                text(h, &kim, &today(), 24.0, grey24, ());
                h.spawn(Node { width: dp(10.0), ..default() });
                // W_SystemInfoConfigEntry ProjectVersion / BuildVersion (the game's config values, as its loading screen shows them)
                text(h, &kim, "Project Version: V0.8.12.19346 Build Version: 80", 24.0, grey24, ());
                h.spawn(Node { flex_grow: 1.0, ..default() });
                text(h, &ki, "REC", 32.0, dark32, ());
                // Image_808: M_UI_RecIndicator 30x30, tint (0.526, 0, 0). ponytail: material compiled away; a dot that blinks at 1 Hz
                h.spawn((Node { width: dp(30.0), height: dp(30.0), margin: UiRect::left(dp(8.0)), border_radius: BorderRadius::MAX, ..default() }, BackgroundColor(Color::linear_rgb(0.526042, 0.0, 0.0)), RecDot));
            });
            v.spawn(Node { flex_grow: 1.0, ..default() });
            v.spawn(Node { flex_direction: FlexDirection::Row, align_items: AlignItems::End, padding: UiRect { left: dp(81.701225), right: dp(66.634717), ..default() }, ..default() }).with_children(|h| {
                // SizeBox 870, Tip rich text: DT_RichTextLoading Gray / White, Ki 18
                h.spawn(Node { width: dp(870.0), ..default() }).with_children(|s| {
                    s.spawn((Text::new("Tip: "), TextFont { font: kim.clone().into(), font_size: TIP_PX.into(), ..default() }, TextColor(Color::linear_rgb(0.286275, 0.286275, 0.286275)), Fs(TIP_PX)))
                        .with_child((TextSpan::new(tip), TextFont { font: kim.clone().into(), font_size: TIP_PX.into(), ..default() }, TextColor(Color::linear_rgb(0.701961, 0.701961, 0.701961)), Fs(TIP_PX)));
                });
                h.spawn(Node { flex_grow: 1.0, flex_direction: FlexDirection::Column, align_items: AlignItems::End, ..default() }).with_children(|c| {
                    text(c, &ki, "WAITING FOR CONNECTION...", 32.0, Color::WHITE, ()); // TS_Headline32
                    // SizeBox 540: M_UI_LoadingBar (material compiled away; matched to the game's screen): 28 grey segments that
                    // fill left to right with the level load (drive()).
                    c.spawn(Node { width: dp(540.0), height: dp(18.0), margin: UiRect::top(dp(14.0)), column_gap: dp(7.7), ..default() }).with_children(|bar| {
                        for i in 0..SEGMENTS {
                            bar.spawn((Node { width: dp(10.5), height: percent(100), ..default() }, BackgroundColor(Color::srgb(0.62, 0.62, 0.62)), Bar(i)));
                        }
                    });
                });
            });
        });
    });
}

/// "{D}/{M}/{Y}", two-digit day and month (W_LoadingMenu).
fn today() -> String {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() / 86_400) as i64;
    // civil-from-days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{d:02}/{m:02}/{y}")
}

/// (VHS + blur visible, VHS brightness, blur strength, "CAM DISCONNECTED", "CAM CONNECTED", loading UI) for a mode.
fn pose(mode: &Mode) -> (bool, f32, f32, bool, bool, bool) {
    match *mode {
        Mode::Loading(_) => (true, 0.09375, 5.0, true, false, true),
        Mode::Disconnected { t, instant } => {
            let on = instant || t >= 1.0;
            (on, 1.0, 5.0, on, false, false)
        }
        Mode::Connected(t) => {
            let blur = if t < 1.35 { 5.0 - 4.0 * t / 1.35 } else { (1.0 - (t - 1.35) / 0.15).max(0.0) };
            (t < 1.5, 1.0, blur, (0.001..0.75).contains(&t), (0.75..1.35).contains(&t), false)
        }
        Mode::Live => (false, 1.0, 0.0, false, false, false),
    }
}

#[allow(clippy::type_complexity)]
fn drive(
    time: Res<Time>,
    mut mode: ResMut<Mode>,
    mut ev: MessageReader<CamTransition>,
    mut reload: MessageReader<Reload>,
    map: Option<Res<crate::map::MapState>>,
    mut mats: ResMut<Assets<VhsMat>>,
    mut vis: ParamSet<(
        Single<&mut Visibility, With<Root>>,
        Single<&mut Visibility, With<Disc>>,
        Single<&mut Visibility, With<Conn>>,
        Single<&mut Visibility, With<LoadingUi>>,
    )>,
    mut dot: Single<&mut BackgroundColor, With<RecDot>>,
    mut bar: Query<(&Bar, &mut Visibility), (Without<Root>, Without<Disc>, Without<Conn>, Without<LoadingUi>)>,
    mut texts: Query<(&mut TextColor, Has<BlurCopy>), Or<(With<BlurCopy>, With<BlurMain>)>>,
) {
    let dt = time.delta_secs();
    let m = &mut *mode;
    if reload.read().count() > 0 {
        *m = Mode::Loading(0.0);
    }
    match m {
        // the game drops the loading screen once the level is up; + 0.8 s for auto exposure to settle (first frames blow out)
        Mode::Loading(t) => {
            if map.as_ref().is_none_or(|m| m.ready()) {
                *t += dt;
            }
            if *t > 0.8 {
                *m = Mode::Connected(0.0);
            }
        }
        Mode::Disconnected { t, .. } => *t += dt,
        Mode::Connected(t) => {
            // FastTransition plays at rate 2 (not sent here)
            *t += dt;
            if *t >= 1.5 {
                *m = Mode::Live;
            }
        }
        Mode::Live => {}
    }
    for e in ev.read() {
        *m = if e.connected { Mode::Connected(0.0) } else { Mode::Disconnected { t: 0.0, instant: e.instant } };
    }
    let (on, bright, blur, disc, conn, loading) = pose(m);
    let shown = |b: bool| if b { Visibility::Inherited } else { Visibility::Hidden };
    **vis.p0() = shown(on || disc || conn || loading);
    **vis.p1() = shown(disc);
    **vis.p2() = shown(conn);
    **vis.p3() = shown(loading);
    let t = time.elapsed_secs();
    for (_, mat) in mats.iter_mut() {
        mat.p = Vec4::new(t, if on { bright } else { 0.0 }, blur, 0.0);
    }
    dot.0 = Color::linear_rgba(0.526042, 0.0, 0.0, if t.fract() < 0.5 { 1.0 } else { 0.0 });
    // Load phases (Bevy has no byte progress for one glTF): file loading creeps toward 85 %, the instanced scene is 93 %,
    // the settled level fills the bar.
    let lit = match map.as_deref() {
        None => SEGMENTS,
        Some(m) if m.ready() => SEGMENTS,
        Some(m) if m.frames != i32::MAX => SEGMENTS * 93 / 100,
        Some(_) => 1 + ((1.0 - (-t / 6.0).exp()) * 0.85 * SEGMENTS as f32) as usize,
    };
    for (b, mut v) in &mut bar {
        *v = if b.0 < lit { Visibility::Inherited } else { Visibility::Hidden };
    }
    let k = (blur / 5.0).clamp(0.0, 1.0);
    for (mut c, copy) in &mut texts {
        c.0 = c.0.with_alpha(if copy { 0.16 * k } else { 1.0 - 0.6 * k });
    }
}

/// TextFont sizes are design px at 1080 lines: scale with the window height.
fn font_scale(win: Single<&Window, Changed<Window>>, mut q: Query<(&mut TextFont, &Fs)>) {
    let k = win.height() / 1080.0;
    for (mut f, s) in &mut q {
        f.font_size = (s.0 * k).into();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_menu_keys() {
        // DeactivatePanel: nothing until 1.0 s, then VHS + "CAM DISCONNECTED"
        assert!(!pose(&Mode::Disconnected { t: 0.9, instant: false }).0);
        assert!(pose(&Mode::Disconnected { t: 1.0, instant: false }).3);
        assert!(pose(&Mode::Disconnected { t: 0.0, instant: true }).0);
        // ActivatePanel: disconnected text until 0.75, connected 0.75..1.35, VHS off at 1.5
        let p = pose(&Mode::Connected(0.5));
        assert!(p.3 && !p.4 && p.0);
        let p = pose(&Mode::Connected(1.0));
        assert!(!p.3 && p.4);
        assert!(!pose(&Mode::Connected(1.5)).0);
        assert!((pose(&Mode::Connected(1.35)).2 - 1.0).abs() < 1e-4);
        assert_eq!(today().len(), 10);
    }
}
