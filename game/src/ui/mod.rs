//! The Bodycam menu: a physical tablet the player raises in first person (BP_Tablet, SKM_Tablet, A_FP_Tablet_* clips).
//! The UMG screens are rebuilt with bevy_ui at the original 1920x1080 design size and rendered to a texture that is put on the
//! tablet's screen material (M_Tablet_03). Escape / Tab (IA_Score) raise and lower it. While it is up, gameplay input is eaten
//! (`gate`) and the mouse moves the white dot cursor (IA_TabletCursorMouse); the bracket hotkeys of each screen work too.
//! Settings (settings.rs) are applied live and saved to a JSON file; LOADOUT picks the guns that are actually equipped.
pub mod settings;
mod screens;
pub mod showcase;

use crate::weapon::{Inventory, MontageStarted};
use bevy::camera::visibility::RenderLayers;
use bevy::camera::{ClearColorConfig, ImageRenderTarget, RenderTarget};
use bevy::gltf::GltfMaterialName;
use bevy::input::keyboard::KeyboardInput;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseWheel};
use bevy::prelude::*;
use bevy::render::render_resource::TextureFormat;
use bevy::ui::{CalculatedClip, UiGlobalTransform, UiTargetCamera};
use bevy::window::{CursorGrabMode, CursorOptions};
use screens::BuildCx;
use settings::{Kind, Set, Settings, ACTIONS};
use std::collections::HashSet;

pub const W: f32 = 1920.0;
pub const H: f32 = 1080.0;
/// Seconds after the raise starts until the screen takes input / until the lowered tablet is gone (clip lengths 1.13 / 0.9).
const READY_AT: f32 = 0.9;
const CLOSE_AT: f32 = 0.85;
const CURSOR_SPEED: f32 = 1.4;

/// Fake cursor motion for BODYCAM_AUTO (CurR/CurL/CurU/CurD).
#[derive(Resource, Default)]
pub struct FakeCursor(pub Vec2);

/// Tablet state read by the viewmodel: `shown` = model attached, `cmd` = play raise (true) / put-away (false) once.
#[derive(Resource, Default)]
pub struct Tablet {
    pub shown: bool,
    pub open: bool,
    pub ready: bool,
    pub t: f32,
    pub cmd: Option<bool>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Screen {
    Home,
    Play,
    Loadout,
    Weapon,
    /// MODIFY <gun>: slot cards of the current build.
    Modify,
    /// SELECT <slot>: the options of `Ui::att_slot`, by category.
    Select,
    /// SWAP <slot>: `Ui::swap` = 0 primary, 1 secondary, 2 explosive, 3 melee, 4 gadget.
    Swap,
    Menu,
    Settings,
    Social,
    Credits,
    Scoreboard,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Push(Screen),
    Go(Screen),
    Back,
    Close,
    Tab(i32),
    SetTab(usize),
    Step(Set, i32),
    Slide(Set),
    Toggle(Set),
    ResetSel,
    ResetAll,
    Rebind(usize),
    Weapon(String),
    Cycle(i32),
    Equip,
    /// Toggle an attachment id on the shown weapon (MODIFY).
    Att(&'static str),
    /// MODIFY slot card: open SELECT for that slot.
    AttSlot(usize),
    /// SELECT: remove the pick of the slot (back to the gun's default part).
    Unequip,
    /// Loadout card PRIMARY..GADGET -> its SWAP screen.
    SwapKind(usize),
    /// SWAP PRIMARY / SECONDARY: equip this gun.
    SwapGun(String),
    /// LOADOUT EXPLOSIVE / MELEE / GADGET column (0/1/2): equip that DT_PerkItems row.
    Gear(usize, String),
    Search,
    /// PLAY > HOST SERVERS: host a match / join the typed address / focus the address field.
    Host,
    Join,
    IpField,
    Offline(usize),
    News,
    Toast(String),
    Exit,
    Noop,
}

#[derive(Component)]
pub struct Btn {
    pub act: Act,
    pub key: Option<KeyCode>,
    pub bg: Color,
    pub hover: Color,
    pub set: Option<Set>,
}
#[derive(Component)]
pub struct ValueText(pub Set);
#[derive(Component)]
pub struct SliderFill(pub Set);
#[derive(Component)]
pub struct KeyLabel(pub usize);
/// true = title, false = description of the hovered setting.
#[derive(Component)]
pub struct HelpText(pub bool);
#[derive(Component)]
pub struct Scroller(pub usize);
#[derive(Component)]
struct CursorDot;
#[derive(Component)]
struct UiRoot;
#[derive(Component)]
struct Content;

pub struct Fonts {
    pub bold: Handle<Font>,
    pub medium: Handle<Font>,
    pub extra: Handle<Font>,
}

#[derive(Resource)]
struct UiAssets {
    fonts: Fonts,
    image: Handle<Image>,
    screen_mat: Handle<StandardMaterial>,
    body_mat: Handle<StandardMaterial>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Primary = 0,
    Secondary = 1,
}

pub struct Item {
    pub row: String,
    pub display: String,
    pub slot: Slot,
    pub damage: f32,
    pub fire_rate: f32,
    pub cap: f32,
}

/// Loadout items = every DT_WeaponItems row we have an animation set for.
#[derive(Resource, Default)]
pub struct Roster(pub Vec<Item>);

fn display_name(row: &str) -> String {
    match row {
        "M4A1" => "M4",
        "AK47" => "AK-47",
        "DRAGUNOV" => "Dragunov",
        "Glock17" => "Glock 17",
        "Mossberg" => "Mossberg 590",
        "Remington700" => "Remington 700",
        "SCAR" => "SCAR-H",
        "kriss vector" => "Kriss Vector",
        r => r,
    }
    .into()
}

const SECONDARY: [&str; 6] = ["CZ-P-09", "Deagle", "Glock17", "Kobra", "M1911", "Revolver"];
/// (label, BODYCAM_MAP, BODYCAM_START zone)
/// PLAY > CASUAL worlds: (name, BODYCAM_MAP, BODYCAM_START zone, BODYCAM_MODE, description). Picking one relaunches into it.
pub const OFFLINE: [(&str, &str, &str, &str, &str); 7] = [
    ("LOBBY", "", "", "", "The real lobby map: shooting range, workbench and shoot house."),
    ("TEST GRID", "grid", "", "", "Big test world with lobby, range, obstacle course and CQB. F1 / F3 / F4 / F5 teleport, F7 bots hostile, F10 respawn bots."),
    ("RANGE", "grid", "range", "", "Test grid, starting on the shooting range: targets at set distances."),
    ("COURSE", "grid", "course", "", "Test grid, starting on the obstacle course: stairs, ramps, jumps and crouch gaps."),
    ("CQB", "grid", "cqb", "", "Test grid, starting in the CQB block: rooms, doors and bots."),
    ("TEST MAP", "test", "", "", "Tiny flat-lit map with distance stripes and targets for quick checks."),
    ("BODYBOMB", "test", "", "bomb", "Bomb mode on the test map: plant and defuse rounds."),
];

#[derive(Resource)]
pub struct Ui {
    pub screen: Screen,
    stack: Vec<Screen>,
    pub tab: usize,
    pub play_tab: usize,
    pub weapon: String,
    pub offline: usize,
    pub search_t: Option<f32>,
    pub ip_focus: bool,
    /// Some(true) = host, Some(false) = join: sent as net::Connect after the frame's input.
    net_req: Option<bool>,
    /// PLAY picked a world: level.rs loads it.
    level_req: Option<[String; 3]>,
    pub news: bool,
    pub help: Option<Set>,
    pub rebinding: Option<usize>,
    cursor: Vec2,
    drag: Option<(Set, f32, f32)>,
    pub scroll: [f32; 4],
    /// MODIFY / SELECT: selected attachment slot (index into attach::slots of the gun) and category tab in it.
    pub att_slot: usize,
    pub att_cat: usize,
    /// SWAP screen kind (0 primary .. 4 gadget) and its class tab.
    pub swap: usize,
    pub swap_cat: usize,
    dirty: bool,
    pub toast: (String, f32),
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            screen: Screen::Home,
            stack: vec![],
            tab: 0,
            play_tab: 0,
            weapon: String::new(),
            offline: 0,
            search_t: None,
            ip_focus: false,
            net_req: None,
            level_req: None,
            news: false,
            help: None,
            rebinding: None,
            // BODYCAM_CURSOR=x,y: start position of the tablet cursor (scripted tests)
            cursor: std::env::var("BODYCAM_CURSOR").ok().and_then(|v| v.split_once(',').map(|(x, y)| Vec2::new(x.parse().unwrap_or(0.0), y.parse().unwrap_or(0.0)))).unwrap_or(Vec2::new(W / 2.0, H / 2.0)),
            drag: None,
            scroll: [0.0; 4],
            att_slot: 0,
            att_cat: 0,
            swap: 0,
            swap_cat: 0,
            dirty: true,
            toast: (String::new(), 0.0),
        }
    }
}

/// Raw input of this frame, captured before the gameplay input is cleared.
#[derive(Resource, Default)]
struct MenuInput {
    pressed: Vec<KeyCode>,
    click: bool,
    held: bool,
    motion: Vec2,
    scroll: f32,
}

pub struct UiPlugin;
impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        let s = settings::load();
        app.insert_resource(s)
            .add_plugins(showcase::ShowcasePlugin)
            .init_resource::<Tablet>()
            .init_resource::<Ui>()
            .init_resource::<MenuInput>()
            .init_resource::<FakeCursor>()
            .init_resource::<Roster>()
            .add_systems(Startup, setup)
            .add_systems(PreUpdate, gate.after(bevy::input::InputSystems).after(crate::dev::autotest).after(crate::transition::lock_input))
            .add_systems(
                Update,
                (settings::apply, settings::res_scale, roster, tablet_state, ui_input, rebuild, refresh, screen_material, screen_gain).chain(),
            )
            .add_systems(Last, settings::limit_fps);
    }
}

fn setup(mut cmd: Commands, assets: Res<AssetServer>, mut images: ResMut<Assets<Image>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    let image = images.add(Image::new_target_texture(W as u32, H as u32, TextureFormat::Rgba8UnormSrgb, None));
    let cam = cmd
        .spawn((
            Camera2d,
            Camera { order: -10, is_active: false, clear_color: ClearColorConfig::Custom(Color::BLACK), ..default() },
            RenderTarget::Image(ImageRenderTarget { handle: image.clone(), scale_factor: 1.0 }),
            RenderLayers::layer(7),
            UiCamera,
        ))
        .id();
    cmd.spawn((Node { width: px(W), height: px(H), ..default() }, UiTargetCamera(cam), UiRoot));
    // A very faint LCD look on the screen: every third pixel row a touch darker (scanline tile, nearest-sampled, on top of the menu).
    // ponytail: not from the game's M_Tablet_03 (its graph is compiled away); per user request.
    let mut rows = Image::new(
        bevy::render::render_resource::Extent3d { width: 1, height: 3, depth_or_array_layers: 1 },
        bevy::render::render_resource::TextureDimension::D2,
        vec![0, 0, 0, 34, 0, 0, 0, 0, 0, 0, 0, 0],
        TextureFormat::Rgba8UnormSrgb,
        bevy::asset::RenderAssetUsages::RENDER_WORLD,
    );
    rows.sampler = bevy::image::ImageSampler::nearest();
    cmd.spawn((
        Node { position_type: PositionType::Absolute, width: px(W), height: px(H), ..default() },
        ImageNode { image: images.add(rows), image_mode: NodeImageMode::Tiled { tile_x: true, tile_y: true, stretch_value: 1.0 }, ..default() },
        GlobalZIndex(1000),
        Pickable::IGNORE,
        UiTargetCamera(cam),
    ));
    let screen_mat = mats.add(StandardMaterial { base_color: Color::BLACK, emissive: LinearRgba::gray(0.3), emissive_texture: Some(image.clone()), perceptual_roughness: 0.6, reflectance: 0.0,
        // The screen quad only spans u 0.004..0.656, v 0.043..0.392 of the UE composite render target: stretch that rect over our whole UI texture.
        uv_transform: bevy::math::Affine2::from_scale_angle_translation(Vec2::new(1.0 / 0.652, 1.0 / 0.349), 0.0, Vec2::new(-0.0039 / 0.652, -0.043 / 0.349)),
        ..default() });
    let body_mat = mats.add(StandardMaterial { base_color: Color::srgb(0.045, 0.045, 0.05), metallic: 0.0, perceptual_roughness: 0.85, ..default() });
    cmd.insert_resource(UiAssets {
        fonts: Fonts { bold: assets.load("fonts/Ki-Bold.otf"), medium: assets.load("ui/Ki-Medium.otf"), extra: assets.load("ui/Ki-ExtraBold.otf") },
        image,
        screen_mat,
        body_mat,
    });
}

#[derive(Component)]
struct UiCamera;

/// Eats gameplay input while the tablet is up and applies keybinds otherwise.
fn gate(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut mouse: ResMut<ButtonInput<MouseButton>>,
    mut motion: ResMut<AccumulatedMouseMotion>,
    mut scroll: ResMut<AccumulatedMouseScroll>,
    mut wheel: ResMut<Messages<MouseWheel>>,
    mut kb: MessageReader<KeyboardInput>,
    tablet: Res<Tablet>,
    settings: Res<Settings>,
    fake: Res<FakeCursor>,
    mut mi: ResMut<MenuInput>,
    mut prev: Local<HashSet<KeyCode>>,
) {
    let repeats: HashSet<KeyCode> = kb.read().filter(|e| e.repeat).map(|e| e.key_code).collect();
    // A key counts once per physical press: clearing the input below makes held keys look freshly pressed every frame.
    mi.pressed = keys.get_just_pressed().filter(|k| !repeats.contains(k) && !prev.contains(k)).copied().collect();
    *prev = keys.get_pressed().copied().collect();
    mi.click = mouse.just_pressed(MouseButton::Left);
    mi.held = mouse.pressed(MouseButton::Left);
    mi.motion = motion.delta + fake.0;
    mi.scroll = scroll.delta.y;
    keys.reset(KeyCode::Escape); // owned by the menu (IA_Score)
    keys.reset(KeyCode::Tab);
    if tablet.shown {
        keys.reset_all();
        mouse.reset_all();
        motion.delta = Vec2::ZERO;
        scroll.delta = Vec2::ZERO;
        wheel.clear();
    } else {
        settings::remap(&settings, &mut keys);
    }
}

fn roster(inv: Option<Single<&Inventory>>, mut r: ResMut<Roster>) {
    let Some(inv) = inv else { return };
    if !r.0.is_empty() {
        return;
    }
    for w in inv.slots.iter().chain(&inv.pool).filter(|w| w.kind == crate::weapon::ItemKind::Gun) {
        r.0.push(Item {
            row: w.name.clone(),
            display: display_name(&w.name),
            slot: if SECONDARY.contains(&w.name.as_str()) { Slot::Secondary } else { Slot::Primary },
            damage: w.stats.damage.unwrap_or(0.0),
            fire_rate: w.stats.fire_rate.unwrap_or(0.0),
            cap: w.stats.ammo_capacity.unwrap_or(0.0),
        });
    }
    r.0.sort_by_key(|i| (i.slot as u8, i.display.clone()));
}

fn tablet_state(
    time: Res<Time>,
    mi: Res<MenuInput>,
    mut tablet: ResMut<Tablet>,
    mut ui: ResMut<Ui>,
    inv: Option<Single<&mut Inventory>>,
    mut out: MessageWriter<MontageStarted>,
    cursor: Option<Single<&mut CursorOptions>>,
    cam: Option<Single<&mut Camera, With<UiCamera>>>,
    ready: Res<crate::dev::Ready>,
) {
    if ready.0.is_none() {
        return;
    }
    tablet.t += time.delta_secs();
    let toggle = mi.pressed.contains(&KeyCode::Tab) || (mi.pressed.contains(&KeyCode::Escape) && !tablet.shown);
    if toggle && !tablet.shown {
        info!("tablet: raise");
        *tablet = Tablet { shown: true, open: true, ready: false, t: 0.0, cmd: Some(true) };
        *ui = Ui { cursor: ui.cursor, scroll: ui.scroll, ..default() };
        if let Some(mut c) = cursor {
            c.grab_mode = CursorGrabMode::Locked;
            c.visible = false;
        }
    } else if toggle && tablet.ready && tablet.open {
        close(&mut tablet);
    }
    if tablet.open && !tablet.ready && tablet.t >= READY_AT {
        tablet.ready = true;
    }
    if tablet.shown && !tablet.open && tablet.t >= CLOSE_AT {
        tablet.shown = false;
        if let Some(mut inv) = inv {
            inv.redraw(&mut out);
        }
    }
    if let Some(mut c) = cam {
        c.is_active = tablet.shown;
    }
}

fn close(t: &mut Tablet) {
    (t.open, t.ready, t.t, t.cmd) = (false, false, 0.0, Some(false));
}

/// PLAY > CASUAL: load the picked world in place (level.rs) behind the loading screen; the current one just closes the tablet.
fn launch(ui: &mut Ui, tablet: &mut Tablet) {
    let (_, map, zone, mode, _) = OFFLINE[ui.offline];
    let cur = crate::level::level();
    ui.search_t = None;
    ui.dirty = true;
    close(tablet);
    if !(map == cur[0] && zone.is_empty() && map != "grid" && mode == cur[2]) {
        ui.level_req = Some([map, zone, mode].map(String::from));
    }
}

struct Cx<'a> {
    ui: &'a mut Ui,
    s: &'a mut Settings,
    tablet: &'a mut Tablet,
    inv: Option<&'a mut Inventory>,
    roster: &'a Roster,
    quit: bool,
}

fn go(cx: &mut Cx, s: Screen, push: bool) {
    if push {
        cx.ui.stack.push(cx.ui.screen);
    } else {
        cx.ui.stack.clear();
    }
    cx.ui.screen = s;
    cx.ui.help = None;
    cx.ui.rebinding = None;
    cx.ui.dirty = true;
}

fn handle(act: Act, cx: &mut Cx, rect: Option<(f32, f32)>) {
    cx.ui.dirty = true;
    match act {
        Act::Push(s) => go(cx, s, true),
        Act::Go(s) => go(cx, s, false),
        Act::Back => {
            if let Some(s) = cx.ui.stack.pop() {
                cx.ui.screen = s;
                cx.ui.rebinding = None;
            } else if cx.ui.screen == Screen::Weapon {
                go(cx, Screen::Loadout, false);
            } else {
                close(cx.tablet);
            }
        }
        Act::Close => close(cx.tablet),
        Act::Tab(d) => {
            let (cur, n) = match cx.ui.screen {
                Screen::Settings => (&mut cx.ui.tab, settings_tabs()),
                Screen::Play => (&mut cx.ui.play_tab, 3),
                Screen::Select => (&mut cx.ui.att_cat, screens::select_cats(cx.s, &cx.ui.weapon, cx.ui.att_slot).len().max(1)),
                Screen::Swap => (&mut cx.ui.swap_cat, screens::swap_cats(cx.roster, cx.ui.swap).len().max(1)),
                Screen::Home => return go(cx, Screen::Loadout, false),
                Screen::Loadout => return go(cx, Screen::Home, false),
                _ => return,
            };
            *cur = (*cur as i32 + d).rem_euclid(n as i32) as usize;
            cx.ui.help = None;
            cx.ui.scroll[2] = 0.0;
        }
        Act::SetTab(i) => {
            if cx.ui.screen == Screen::Settings {
                cx.ui.tab = i;
                cx.ui.help = None;
                cx.ui.scroll[2] = 0.0;
            } else if cx.ui.screen == Screen::Select {
                cx.ui.att_cat = i;
                cx.ui.scroll[3] = 0.0;
            } else if cx.ui.screen == Screen::Swap {
                cx.ui.swap_cat = i;
            } else {
                cx.ui.play_tab = i;
            }
        }
        Act::Step(set, d) => {
            let sp = settings::spec(set);
            let v = cx.s.get(set);
            let n = Settings::options(set);
            cx.s.set(set, if let Kind::Select = sp.kind { (v as i32 + d).rem_euclid(n as i32) as f32 } else { v });
            cx.ui.dirty = false;
        }
        Act::Toggle(set) => {
            let v = cx.s.get(set);
            cx.s.set(set, 1.0 - v);
            cx.ui.dirty = false;
        }
        Act::Slide(set) => {
            if let (Some((left, width)), Kind::Slider { min, max, step }) = (rect, settings::spec(set).kind) {
                cx.ui.drag = Some((set, left, width));
                let f = ((cx.ui.cursor.x - left) / width).clamp(0.0, 1.0);
                cx.s.set(set, (((min + f * (max - min)) / step).round() * step).clamp(min, max));
            }
            cx.ui.dirty = false;
        }
        Act::ResetSel => {
            if let Some(s) = cx.ui.help {
                cx.s.reset(s);
            }
            cx.ui.dirty = false;
        }
        Act::ResetAll => {
            let keep = (cx.s.primary.clone(), cx.s.secondary.clone(), cx.s.explosive.clone(), cx.s.melee.clone(), cx.s.gadget.clone());
            *cx.s = Settings { primary: keep.0, secondary: keep.1, explosive: keep.2, melee: keep.3, gadget: keep.4, ..default() };
            cx.ui.dirty = false;
            cx.ui.toast = ("All settings reset.".into(), 2.5);
            cx.ui.dirty = true;
        }
        Act::Rebind(i) => cx.ui.rebinding = Some(i),
        Act::Weapon(row) => {
            if row != cx.ui.weapon {
                (cx.ui.att_slot, cx.ui.scroll[3]) = (0, 0.0);
            }
            cx.ui.weapon = row;
            if cx.ui.screen != Screen::Weapon {
                go(cx, Screen::Weapon, true);
            }
        }
        Act::Cycle(d) => {
            let slot = cx.roster.0.iter().find(|i| i.row == cx.ui.weapon).map(|i| i.slot);
            let list: Vec<&Item> = cx.roster.0.iter().filter(|i| Some(i.slot) == slot).collect();
            if let Some(p) = list.iter().position(|i| i.row == cx.ui.weapon) {
                cx.ui.weapon = list[(p as i32 + d).rem_euclid(list.len() as i32) as usize].row.clone();
            }
        }
        Act::Equip => {
            if let Some(it) = cx.roster.0.iter().find(|i| i.row == cx.ui.weapon) {
                if it.slot == Slot::Primary {
                    cx.s.primary = it.row.clone();
                } else {
                    cx.s.secondary = it.row.clone();
                }
                if let Some(inv) = cx.inv.as_deref_mut() {
                    inv.equip(&cx.s.primary, &cx.s.secondary);
                }
                cx.ui.toast = (format!("{} equipped.", it.display), 2.5);
            }
        }
        Act::Att(id) => {
            let row = cx.ui.weapon.clone();
            crate::attach::toggle(cx.s, &row, id);
            settings::save(cx.s);
            let on = crate::attach::loadout(cx.s, &row).iter().any(|x| x == id);
            cx.ui.toast = (format!("{} {}.", crate::attach::label(id), if on { "mounted" } else { "removed" }), 2.0);
        }
        Act::AttSlot(i) => {
            cx.ui.att_slot = i;
            cx.ui.scroll[3] = 0.0;
            // open on the category of the part in that slot
            let cats = screens::select_cats(cx.s, &cx.ui.weapon, i);
            let cur = screens::slot_part(cx.s, &cx.ui.weapon, i);
            cx.ui.att_cat = cur.and_then(|id| cats.iter().position(|c| *c == screens::cat_of(&id))).unwrap_or(0);
            go(cx, Screen::Select, true);
        }
        Act::Unequip => {
            let row = cx.ui.weapon.clone();
            if let Some(id) = screens::slot_part(cx.s, &row, cx.ui.att_slot).filter(|id| cx.s.has_att(&row, id)) {
                cx.s.toggle_att(&row, &id);
                settings::save(cx.s);
                cx.ui.toast = (format!("{} removed.", crate::attach::label(&id)), 2.0);
            }
        }
        Act::SwapKind(k) => {
            cx.ui.swap = k;
            cx.ui.swap_cat = 0;
            if k < 2 {
                let cur = if k == 0 { cx.s.primary.clone() } else { cx.s.secondary.clone() };
                let cats = screens::swap_cats(cx.roster, k);
                cx.ui.swap_cat = cats.iter().position(|c| crate::attach::data().classes.get(&cur) == Some(c)).unwrap_or(0);
            }
            go(cx, Screen::Swap, true);
        }
        Act::SwapGun(row) => {
            cx.ui.weapon = row;
            handle(Act::Equip, cx, rect);
            return;
        }
        Act::Gear(slot, row) => {
            match slot {
                0 => cx.s.explosive = row.clone(),
                1 => cx.s.melee = row.clone(),
                _ => cx.s.gadget = row.clone(),
            }
            if let Some(inv) = cx.inv.as_deref_mut() {
                inv.set_item(crate::items::SLOT_GRENADE + slot, crate::items::load_item(&row));
            }
            cx.ui.toast = (format!("{} equipped.", crate::items::display_name(&row)), 2.5);
        }
        Act::Offline(i) => {
            cx.ui.offline = i;
            settings::save(cx.s);
            launch(cx.ui, cx.tablet);
        }
        Act::Host => cx.ui.net_req = Some(true),
        Act::Join => cx.ui.net_req = Some(false),
        Act::IpField => cx.ui.ip_focus = !cx.ui.ip_focus,
        Act::Search => {
            cx.ui.search_t = Some(0.0);
            cx.ui.toast = ("Online matchmaking is offline: starting a local session.".into(), 3.0);
        }
        Act::News => cx.ui.news = !cx.ui.news,
        Act::Toast(m) => cx.ui.toast = (m, 3.0),
        Act::Exit => {
            settings::save(cx.s);
            cx.quit = true;
        }
        Act::Noop => cx.ui.dirty = false,
    }
}

fn settings_tabs() -> usize {
    screens::TABS.len()
}

fn ui_input(
    time: Res<Time>,
    mi: Res<MenuInput>,
    mut tablet: ResMut<Tablet>,
    mut ui: ResMut<Ui>,
    mut s: ResMut<Settings>,
    roster: Res<Roster>,
    inv: Option<Single<&mut Inventory>>,
    btns: Query<(Entity, &Btn, &ComputedNode, &UiGlobalTransform, Option<&CalculatedClip>)>,
    mut scrollers: Query<(&Scroller, &mut ScrollPosition, &ComputedNode, &UiGlobalTransform)>,
    mut exit: MessageWriter<AppExit>,
    mut connect: MessageWriter<crate::net::Connect>,
    ready: Res<crate::dev::Ready>,
    mut script: Local<Option<(Vec<(f32, Vec2)>, usize)>>,
    mut cmd: Commands,
) {
    let dt = time.delta_secs();
    // BODYCAM_CLICKS="t:x,y;t:x,y": warp the cursor and click at sim time t after "viewmodel ready" (scripted tests).
    let (plan, next) = script.get_or_insert_with(|| {
        let v = std::env::var("BODYCAM_CLICKS").unwrap_or_default();
        let p = v.split(';').filter_map(|e| {
            let (t, xy) = e.split_once(':')?;
            let (x, y) = xy.split_once(',')?;
            Some((t.parse().ok()?, Vec2::new(x.parse().ok()?, y.parse().ok()?)))
        });
        (p.collect(), 0)
    });
    let mut scripted = false;
    if let (Some(r), true) = (ready.0, tablet.ready) {
        if *next < plan.len() && time.elapsed_secs() - r >= plan[*next].0 {
            ui.cursor = plan[*next].1;
            *next += 1;
            scripted = true;
        }
    }
    if ui.toast.1 > 0.0 {
        ui.toast.1 -= dt;
        if ui.toast.1 <= 0.0 {
            ui.toast.0.clear();
            ui.dirty = true;
        }
    }
    if let Some(t) = &mut ui.search_t {
        *t += dt;
        if *t > 2.0 && tablet.open {
            launch(&mut ui, &mut tablet);
        }
    }
    if !tablet.shown || !tablet.ready {
        return;
    }
    if !scripted {
        ui.cursor = (ui.cursor + mi.motion * CURSOR_SPEED).clamp(Vec2::ZERO, Vec2::new(W - 1.0, H - 1.0));
    }
    let c = ui.cursor;
    // Smallest button under the cursor wins (nested buttons).
    let hit = btns
        .iter()
        .filter(|(_, _, n, g, clip)| n.contains_point(**g, c) && clip.is_none_or(|k| k.contains_point(c)))
        .min_by(|a, b| (a.2.size().x * a.2.size().y).total_cmp(&(b.2.size().x * b.2.size().y)));
    let rect_of = |n: &ComputedNode, g: &UiGlobalTransform| (g.translation.x - n.size().x / 2.0, n.size().x);
    let mut ps = (*s).clone();
    let mut inv = inv;
    let mut act: Option<(Act, Option<(f32, f32)>)> = None;
    if let Some((_, b, ..)) = hit {
        if let Some(set) = b.set {
            ui.help = Some(set);
        }
    }
    if ui.ip_focus {
        // JOIN address field: digits, '.', ':' (Semicolon), Backspace; Enter/Escape leave it, a click acts as usual.
        for k in &mi.pressed {
            let c = match k {
                KeyCode::Period | KeyCode::NumpadDecimal => Some('.'),
                KeyCode::Semicolon => Some(':'),
                KeyCode::Digit0 | KeyCode::Numpad0 => Some('0'),
                KeyCode::Digit1 | KeyCode::Numpad1 => Some('1'),
                KeyCode::Digit2 | KeyCode::Numpad2 => Some('2'),
                KeyCode::Digit3 | KeyCode::Numpad3 => Some('3'),
                KeyCode::Digit4 | KeyCode::Numpad4 => Some('4'),
                KeyCode::Digit5 | KeyCode::Numpad5 => Some('5'),
                KeyCode::Digit6 | KeyCode::Numpad6 => Some('6'),
                KeyCode::Digit7 | KeyCode::Numpad7 => Some('7'),
                KeyCode::Digit8 | KeyCode::Numpad8 => Some('8'),
                KeyCode::Digit9 | KeyCode::Numpad9 => Some('9'),
                _ => None,
            };
            if let Some(c) = c && ps.join_ip.len() < 40 {
                ps.join_ip.push(c);
            } else if *k == KeyCode::Backspace {
                ps.join_ip.pop();
            } else if matches!(k, KeyCode::Enter | KeyCode::Escape) {
                ui.ip_focus = false;
            }
            ui.dirty = true;
        }
        if mi.click && let Some((_, b, n, g, _)) = hit {
            act = Some((b.act.clone(), Some(rect_of(n, g))));
        }
    } else if let Some(i) = ui.rebinding {
        // Rebind prompt: the next key becomes the bind (Escape cancels).
        if let Some(k) = mi.pressed.first().copied() {
            ui.rebinding = None;
            ui.dirty = true;
            let table = settings::key_table();
            if k != KeyCode::Escape && let Some((_, name)) = table.iter().find(|(c, _)| *c == k) {
                if ACTIONS[i].2 == k {
                    ps.binds.remove(ACTIONS[i].0);
                } else {
                    ps.binds.insert(ACTIONS[i].0.into(), name.clone());
                }
            }
        }
    } else if mi.click || scripted {
        if let Some((_, b, n, g, _)) = hit {
            act = Some((b.act.clone(), Some(rect_of(n, g))));
        }
    } else if let Some(k) = mi.pressed.iter().find(|k| **k != KeyCode::Tab) {
        // Hotkeys: Escape is also "back".
        if let Some((_, b, ..)) = btns.iter().find(|(_, b, ..)| b.key == Some(*k)) {
            act = Some((b.act.clone(), None));
        } else if *k == KeyCode::Escape {
            act = Some((Act::Back, None));
        }
    }
    // Slider drag.
    if let Some((set, left, width)) = ui.drag {
        if mi.held {
            if let Kind::Slider { min, max, step } = settings::spec(set).kind {
                let f = ((c.x - left) / width).clamp(0.0, 1.0);
                ps.set(set, (((min + f * (max - min)) / step).round() * step).clamp(min, max));
            }
        } else {
            ui.drag = None;
        }
    }
    // Wheel scrolls the list under the cursor.
    if mi.scroll != 0.0 {
        for (sc, mut pos, n, g) in &mut scrollers {
            if n.contains_point(*g, c) {
                pos.0.y = (pos.0.y - mi.scroll * 70.0).max(0.0);
                ui.scroll[sc.0] = pos.0.y;
            }
        }
    }
    for (sc, pos, ..) in &scrollers {
        ui.scroll[sc.0] = pos.0.y;
    }
    let mut cx = Cx { ui: &mut ui, s: &mut ps, tablet: &mut tablet, inv: inv.as_deref_mut().map(|i| &mut **i), roster: &roster, quit: false };
    if let Some((a, r)) = act {
        handle(a, &mut cx, r);
    }
    let quit = cx.quit;
    if let Some(l) = ui.level_req.take() {
        cmd.insert_resource(crate::level::LoadLevel(Some(l)));
    }
    if let Some(host) = ui.net_req.take() {
        connect.write(crate::net::Connect { host, addr: ps.join_ip.clone() });
        ui.toast = (if host { "Hosting a match.".into() } else { format!("Joining {}...", ps.join_ip) }, 3.0);
        ui.dirty = true;
    }
    if quit {
        exit.write(AppExit::Success);
    }
    if ps != *s {
        *s = ps;
    }
}

fn rebuild(
    mut cmd: Commands,
    mut ui: ResMut<Ui>,
    s: Res<Settings>,
    roster: Res<Roster>,
    assets: Res<AssetServer>,
    a: Res<UiAssets>,
    tablet: Res<Tablet>,
    show: Res<showcase::Showcase>,
    root: Single<Entity, With<UiRoot>>,
    old: Query<Entity, With<Content>>,
    dot: Query<(), With<CursorDot>>,
) {
    if !tablet.shown || !ui.dirty {
        return;
    }
    ui.dirty = false;
    for e in &old {
        cmd.entity(e).despawn();
    }
    let page = cmd.spawn((Node { width: px(W), height: px(H), position_type: PositionType::Absolute, ..default() }, Content, BackgroundColor(screens::DARK))).id();
    cmd.entity(*root).add_child(page);
    let cx = BuildCx { f: &a.fonts, a: &assets, ui: &ui, s: &s, roster: &roster, show: show.image.clone() };
    screens::build(&mut cmd, page, &cx);
    if dot.is_empty() {
        let d = cmd.spawn((Node { position_type: PositionType::Absolute, width: px(16), height: px(16), border_radius: BorderRadius::all(px(8)), border: UiRect::all(px(2)), ..default() }, BackgroundColor(Color::WHITE), BorderColor::all(Color::BLACK), GlobalZIndex(100), CursorDot)).id();
        cmd.entity(*root).add_child(d);
    }
}

/// Per-frame updates that don't need a rebuild: cursor dot, hover colours, live values and the help panel.
fn refresh(
    ui: Res<Ui>,
    s: Res<Settings>,
    mut dot: Query<&mut Node, With<CursorDot>>,
    mut btns: Query<(&Btn, &ComputedNode, &UiGlobalTransform, Option<&CalculatedClip>, &mut BackgroundColor)>,
    mut values: Query<(&ValueText, &mut Text), Without<HelpText>>,
    mut fills: Query<(&SliderFill, &mut Node), Without<CursorDot>>,
    mut help: Query<(&HelpText, &mut Text), Without<ValueText>>,
    mut labels: Query<(&KeyLabel, &mut Text), (Without<ValueText>, Without<HelpText>)>,
) {
    for mut n in &mut dot {
        n.left = px(ui.cursor.x - 8.0);
        n.top = px(ui.cursor.y - 8.0);
    }
    let c = ui.cursor;
    for (b, n, g, clip, mut bg) in &mut btns {
        let over = n.contains_point(*g, c) && clip.is_none_or(|k| k.contains_point(c));
        bg.0 = if over && b.hover != b.bg { b.hover } else { b.bg };
    }
    for (v, mut t) in &mut values {
        let x = s.text(v.0);
        if t.0 != x {
            t.0 = x;
        }
    }
    for (f, mut n) in &mut fills {
        if let Kind::Slider { min, max, .. } = settings::spec(f.0).kind {
            n.width = percent(((s.get(f.0) - min) / (max - min)).clamp(0.0, 1.0) * 100.0);
        }
    }
    let shown = ui.help.or_else(|| screens::default_help(ui.tab));
    for (h, mut t) in &mut help {
        t.0 = match (shown, h.0) {
            (Some(set), true) => settings::spec(set).label.to_uppercase(),
            (Some(set), false) => settings::spec(set).help.into(),
            (None, true) => "KEYBINDINGS".into(),
            (None, false) => "Click a key, then press the new one. Escape cancels.".into(),
        };
    }
    for (k, mut t) in &mut labels {
        if ui.rebinding != Some(k.0) {
            t.0 = settings::key_name(settings::bound(&s, ACTIONS[k.0].0)).to_uppercase();
        }
    }
}

/// Puts the rendered UI on the tablet's screen material once the model has spawned.
/// postfx::tablet_plain: the screen glow follows the per-map tablet exposure so it reads the same in every room.
fn screen_gain(g: Res<crate::postfx::TabletScreenGain>, a: Res<UiAssets>, mut mats: ResMut<Assets<StandardMaterial>>) {
    if g.is_changed()
        && let Some(mut m) = mats.get_mut(&a.screen_mat)
    {
        m.emissive = LinearRgba::gray(0.3 * g.0);
        debug!("tablet screen gain {}", g.0);
    }
}

fn screen_material(mut cmd: Commands, a: Res<UiAssets>, new: Query<(Entity, &GltfMaterialName), Added<GltfMaterialName>>) {
    for (e, n) in &new {
        // The exported body textures carry a camo pattern map instead of the base colour: a plain rugged black body instead.
        match n.0.as_str() {
            "M_Tablet_03" => cmd.entity(e).insert(MeshMaterial3d(a.screen_mat.clone())),
            "MI_Tablet_01" | "MI_Tablet_02" => cmd.entity(e).insert(MeshMaterial3d(a.body_mat.clone())),
            _ => &mut cmd.entity(e),
        };
    }
    let _ = &a.image;
}
