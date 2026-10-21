//! Tablet screens (HOME, PLAY, LOADOUT, weapon detail, MENU, SETTINGS, SOCIAL, CREDITS) as bevy_ui trees at the original
//! 1920x1080 UMG design size. Layout follows the real game's screens; sizes use the TS_* text-style numbers (Headline38 = 38 px).
use super::settings::{self, Kind, Set, ACTIONS};
use super::*;

pub const RED: Color = Color::linear_rgb(0.7835, 0.0513, 0.0513);
pub const GREEN: Color = Color::linear_rgb(0.286, 0.894, 0.337);
pub const GREY: Color = Color::linear_rgb(0.701, 0.701, 0.701);
pub const DARK: Color = Color::srgb(0.035, 0.035, 0.04);
pub const PANEL: Color = Color::srgb(0.085, 0.085, 0.095);
pub const PANEL_HOVER: Color = Color::srgb(0.16, 0.16, 0.18);
const CLEAR: Color = Color::NONE;

#[derive(Clone, Copy, PartialEq)]
pub enum Wt {
    Medium,
    Bold,
    Extra,
}

pub struct BuildCx<'a> {
    pub f: &'a Fonts,
    pub a: &'a AssetServer,
    pub ui: &'a Ui,
    pub s: &'a Settings,
    pub roster: &'a Roster,
    /// Weapon preview render (showcase.rs).
    pub show: Handle<Image>,
}

fn add(cmd: &mut Commands, parent: Entity, b: impl Bundle) -> Entity {
    let e = cmd.spawn(b).id();
    cmd.entity(parent).add_child(e);
    e
}

/// The Ki font has no "<" / ">" glyphs worth drawing: the game uses T_UI_ArrowLeft / T_UI_ArrowRight icons instead.
fn arrow_parts(s: &str) -> Option<Vec<&str>> {
    if !(s == "<" || s == ">" || s.starts_with("< ") || s.ends_with(" >") || s.contains(" > ")) {
        return None;
    }
    let mut out = vec![];
    let mut cur = 0;
    for (i, c) in s.char_indices() {
        if c == '<' || c == '>' {
            if i > cur {
                out.push(s[cur..i].trim());
            }
            out.push(&s[i..i + 1]);
            cur = i + 1;
        }
    }
    if cur < s.len() {
        out.push(s[cur..].trim());
    }
    Some(out.into_iter().filter(|x| !x.is_empty()).collect())
}

fn txt(cmd: &mut Commands, p: Entity, cx: &BuildCx, s: &str, size: f32, color: Color, w: Wt) -> Entity {
    if let Some(parts) = arrow_parts(s) {
        let r = add(cmd, p, Node { column_gap: px(size * 0.3), ..row() });
        for part in parts {
            match part {
                "<" | ">" => {
                    let a = size.max(36.0) * 0.8;
                    img(cmd, r, cx, if part == "<" { "T_UI_ArrowLeft" } else { "T_UI_ArrowRight" }, a, a, color);
                }
                t => {
                    txt(cmd, r, cx, t, size, color, w);
                }
            }
        }
        return r;
    }
    let font = match w {
        Wt::Medium => &cx.f.medium,
        Wt::Bold => &cx.f.bold,
        Wt::Extra => &cx.f.extra,
    };
    add(cmd, p, (Text::new(s), TextFont { font: font.clone().into(), font_size: size.into(), ..default() }, TextColor(color)))
}

fn row() -> Node {
    Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, ..default() }
}

fn col() -> Node {
    Node { flex_direction: FlexDirection::Column, ..default() }
}

fn img(cmd: &mut Commands, p: Entity, cx: &BuildCx, path: &str, w: f32, h: f32, tint: Color) -> Entity {
    add(cmd, p, (ImageNode { color: tint, ..ImageNode::new(cx.a.load(format!("ui/tex/{path}.png"))) }, Node { width: px(w), height: px(h), ..default() }))
}

/// A clickable box. `key` is its hotkey (drawn by the caller as "[K]").
fn button(cmd: &mut Commands, p: Entity, node: Node, act: Act, key: Option<KeyCode>, bg: Color, hover: Color, border: Option<Color>) -> Entity {
    let node = Node { border: if border.is_some() { UiRect::all(px(3)) } else { node.border }, ..node };
    let e = add(cmd, p, (node, BackgroundColor(bg), Btn { act, key, bg, hover, set: None }));
    if let Some(b) = border {
        cmd.entity(e).insert(BorderColor::all(b));
    }
    e
}

fn key_hint(k: KeyCode) -> String {
    format!("[{}]", match k {
        KeyCode::Space => "SPACE".into(),
        KeyCode::Escape => "ESC".into(),
        k => settings::key_name(k).to_uppercase(),
    })
}

/// Common chrome: top bar (back, primary button, tabs, currencies) and the body container below it.
fn frame(cmd: &mut Commands, root: Entity, cx: &BuildCx, back: &str, primary: Option<(&str, Act)>, tabs: &[&str], active: usize, tab_act: &dyn Fn(usize) -> Act) -> Entity {
    let page = add(cmd, root, Node { width: percent(100), height: percent(100), flex_direction: FlexDirection::Column, ..default() });
    let bar = add(cmd, page, Node { width: percent(100), height: px(96), padding: UiRect::axes(px(40), px(0)), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, ..default() });
    cmd.entity(bar).insert(BackgroundColor(Color::srgb(0.02, 0.02, 0.025)));
    let wide = tabs.len() > 3;
    let side = if wide { 330.0 } else { 520.0 };
    let left = add(cmd, bar, Node { width: px(side), ..row() });
    let b = button(cmd, left, Node { padding: UiRect::axes(px(18), px(10)), ..row() }, Act::Back, Some(KeyCode::Escape), CLEAR, PANEL_HOVER, None);
    txt(cmd, b, cx, back, 28.0, Color::WHITE, Wt::Bold);
    if let Some((label, act)) = primary {
        let pb = button(cmd, left, Node { margin: UiRect::left(px(24)), padding: UiRect::axes(px(26), px(10)), ..row() }, act, Some(KeyCode::Space), RED, Color::srgb(1.0, 0.3, 0.3), None);
        txt(cmd, pb, cx, label, 28.0, Color::WHITE, Wt::Bold);
    }
    let mid = add(cmd, bar, Node { column_gap: px(36), ..row() });
    if !tabs.is_empty() {
        let q = txt(cmd, mid, cx, "[Q]", 24.0, GREY, Wt::Medium);
        cmd.entity(q).insert(Btn { act: tab_act(usize::MAX), key: Some(KeyCode::KeyQ), bg: CLEAR, hover: CLEAR, set: None });
        for (i, t) in tabs.iter().enumerate() {
            let tb = button(cmd, mid, Node { padding: UiRect::axes(px(10), px(8)), border: UiRect::bottom(px(4)), ..row() }, tab_act(i), None, CLEAR, PANEL_HOVER, None);
            cmd.entity(tb).insert(BorderColor::all(if i == active { RED } else { CLEAR }));
            txt(cmd, tb, cx, t, 30.0, if i == active { Color::WHITE } else { GREY }, Wt::Bold);
        }
        let e = txt(cmd, mid, cx, "[E]", 24.0, GREY, Wt::Medium);
        cmd.entity(e).insert(Btn { act: tab_act(usize::MAX - 1), key: Some(KeyCode::KeyE), bg: CLEAR, hover: CLEAR, set: None });
    }
    let right = add(cmd, bar, Node { width: px(side), justify_content: JustifyContent::FlexEnd, column_gap: px(14), ..row() });
    for (amount, key, icon) in [("1000", "[E]", "T_UI_ReissadCurrency"), ("893", "[R]", "T_UI_ELOCurrency")].into_iter().filter(|_| !wide) {
        let c = add(cmd, right, (Node { padding: UiRect::axes(px(14), px(8)), column_gap: px(10), ..row() }, BackgroundColor(PANEL)));
        img(cmd, c, cx, icon, 26.0, 26.0, Color::WHITE);
        txt(cmd, c, cx, amount, 26.0, Color::WHITE, Wt::Bold);
        txt(cmd, c, cx, key, 18.0, GREY, Wt::Medium);
    }
    let party = add(cmd, right, (Node { padding: UiRect::axes(px(14), px(8)), column_gap: px(10), ..row() }, BackgroundColor(PANEL)));
    img(cmd, party, cx, "T_UI_SocialIcon", 26.0, 26.0, Color::WHITE);
    txt(cmd, party, cx, "1/1", 26.0, Color::WHITE, Wt::Bold);
    let gear = button(cmd, right, Node { padding: UiRect::all(px(8)), ..row() }, Act::Push(Screen::Menu), None, PANEL, PANEL_HOVER, None);
    img(cmd, gear, cx, "T_UI_SettingsIcon", 34.0, 34.0, Color::WHITE);
    add(cmd, page, Node { width: percent(100), flex_grow: 1.0, min_height: px(0), padding: UiRect::axes(px(70), px(36)), flex_direction: FlexDirection::Column, ..default() })
}

fn header(cmd: &mut Commands, p: Entity, cx: &BuildCx, title: &str, sub: &str) {
    let h = add(cmd, p, Node { margin: UiRect::bottom(px(24)), column_gap: px(18), ..row() });
    let r = add(cmd, h, (Node { width: px(46), height: px(46), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, BackgroundColor(RED)));
    txt(cmd, r, cx, "R", 30.0, Color::WHITE, Wt::Extra);
    let c = add(cmd, h, col());
    txt(cmd, c, cx, title, 46.0, Color::WHITE, Wt::Extra);
    txt(cmd, c, cx, sub, 24.0, GREY, Wt::Medium);
}

fn main_tabs(cx: &BuildCx, home: bool, cmd: &mut Commands, root: Entity, primary: Option<(&str, Act)>) -> Entity {
    frame(cmd, root, cx, "< BACK", primary, &["HOME", "LOADOUT"], if home { 0 } else { 1 }, &|i| match i {
        0 => Act::Go(Screen::Home),
        1 => Act::Go(Screen::Loadout),
        _ => Act::Tab(if i == usize::MAX { -1 } else { 1 }),
    })
}

pub fn build(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    match cx.ui.screen {
        Screen::Home => home(cmd, root, cx),
        Screen::Play => play(cmd, root, cx),
        Screen::Loadout => loadout(cmd, root, cx),
        Screen::Weapon => weapon(cmd, root, cx),
        Screen::Modify => modify(cmd, root, cx),
        Screen::Select => select(cmd, root, cx),
        Screen::Swap => swap(cmd, root, cx),
        Screen::Menu => menu(cmd, root, cx),
        Screen::Settings => settings_screen(cmd, root, cx),
        Screen::Social => social(cmd, root, cx),
        Screen::Credits => credits(cmd, root, cx),
        Screen::Scoreboard => scoreboard(cmd, root, cx),
    }
    // Toast line at the bottom.
    if !cx.ui.toast.0.is_empty() {
        let t = add(cmd, root, (Node { position_type: PositionType::Absolute, bottom: px(40), left: px(0), width: percent(100), justify_content: JustifyContent::Center, ..default() },));
        let b = add(cmd, t, (Node { padding: UiRect::axes(px(30), px(12)), ..default() }, BackgroundColor(Color::srgba(0.0, 0.0, 0.0, 0.85))));
        txt(cmd, b, cx, &cx.ui.toast.0, 26.0, Color::WHITE, Wt::Bold);
    }
}

fn home(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = main_tabs(cx, true, cmd, root, Some(("[SPACE] PLAY >", Act::Push(Screen::Play))));
    cmd.entity(body).insert(BackgroundColor(DARK));
    let art = add(cmd, body, (ImageNode { color: Color::srgba(1.0, 1.0, 1.0, 0.35), ..ImageNode::new(cx.a.load("ui/tex/BannerBodycam.png")) }, Node { position_type: PositionType::Absolute, right: px(0), top: px(0), width: px(1100), height: px(780), ..default() }));
    let _ = art;
    let tag = add(cmd, body, (Node { padding: UiRect::axes(px(16), px(6)), margin: UiRect::top(px(60)), align_self: AlignSelf::FlexStart, ..default() }, BackgroundColor(RED)));
    txt(cmd, tag, cx, "MAJOR UPDATE", 24.0, Color::WHITE, Wt::Bold);
    let t = txt(cmd, body, cx, "WELCOME TO BODYCAM\nWELCOME TO THE MAJOR UPDATE !", 72.0, Color::WHITE, Wt::Extra);
    cmd.entity(t).insert(Node { margin: UiRect::vertical(px(22)), max_width: px(1300), ..default() });
    let p = txt(cmd, body, cx, "We've almost rebuilt the game from the ground up.\n\nAll animations have been completely redone and massively expanded. A brand new loadout system has been introduced, featuring hundreds of attachments per weapon along with many new weapons. We've also added new gameplay elements like a drone and an RC car, fully redesigned the sound design and the UI, reworked the lobby map, and introduced a brand new map: Trenches.\n\nThis update marks the beginning of a new phase for Bodycam. Our goal is to make the game even more realistic, more immersive, and to push the boundaries of what a realistic FPS can be.", 22.0, GREY, Wt::Medium);
    cmd.entity(p).insert(Node { max_width: px(1000), ..default() });
    if cx.ui.news {
        let n = add(cmd, body, (Node { margin: UiRect::top(px(24)), padding: UiRect::all(px(20)), max_width: px(900), ..col() }, BackgroundColor(PANEL)));
        txt(cmd, n, cx, "NEWS", 30.0, RED, Wt::Bold);
        txt(cmd, n, cx, "Online services, matchmaking and the shop are offline in this build. Settings and loadout are saved on this PC.", 24.0, Color::WHITE, Wt::Medium);
    }
    let b = button(cmd, body, Node { margin: UiRect::top(px(36)), padding: UiRect::axes(px(26), px(12)), align_self: AlignSelf::FlexStart, ..row() }, Act::News, Some(KeyCode::KeyF), PANEL, PANEL_HOVER, Some(GREY));
    txt(cmd, b, cx, "[F] > OPEN NEWS", 28.0, Color::WHITE, Wt::Bold);
}

fn play(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let searching = cx.ui.search_t.is_some();
    let label = if searching { "SEARCHING..." } else { "[SPACE] SEARCH >" };
    let body = frame(cmd, root, cx, "< MENU", Some((label, Act::Search)), &["CASUAL", "COMPETITIVE", "HOST SERVERS"], cx.ui.play_tab, &|i| match i {
        usize::MAX => Act::Tab(-1),
        i if i == usize::MAX - 1 => Act::Tab(1),
        i => Act::SetTab(i),
    });
    cmd.entity(body).insert(BackgroundColor(DARK));
    let (title, sub) = match cx.ui.play_tab {
        0 => ("CASUAL MATCHMAKING", "Jump quickly into fast-paced game modes in PvP or PvPvE."),
        1 => ("COMPETITIVE", "Play tactical, competitive matches where teamwork decide every round. Climb the ranks with a fully visible Elo system."),
        _ => ("HOST SERVERS", "Create private matches where you can challenge your friends and define your own rules."),
    };
    header(cmd, body, cx, title, sub);
    if cx.ui.play_tab == 2 {
        // LAN / internet play (net.rs): HOST opens a match on port 7777; JOIN connects to the typed address.
        let r = add(cmd, body, Node { margin: UiRect::top(px(24)), column_gap: px(24), align_items: AlignItems::Center, ..row() });
        let b = button(cmd, r, Node { width: px(300), height: px(90), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, Act::Host, None, PANEL, PANEL_HOVER, Some(GREEN));
        txt(cmd, b, cx, "HOST MATCH (7777)", 30.0, Color::WHITE, Wt::Bold);
        let f = button(cmd, r, Node { width: px(420), height: px(90), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, Act::IpField, None, PANEL, PANEL_HOVER, Some(if cx.ui.ip_focus { GREEN } else { RED }));
        let shown = format!("{}{}", cx.s.join_ip, if cx.ui.ip_focus { "_" } else { "" });
        txt(cmd, f, cx, &shown, 30.0, Color::WHITE, Wt::Medium);
        let j = button(cmd, r, Node { width: px(300), height: px(90), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, Act::Join, None, PANEL, PANEL_HOVER, Some(GREEN));
        txt(cmd, j, cx, "JOIN", 30.0, Color::WHITE, Wt::Bold);
        txt(cmd, body, cx, "Click the address to edit it (digits, '.' and ';' for ':'). Both players need the same map.", 20.0, Color::WHITE, Wt::Medium);
        return;
    }
    if cx.ui.play_tab != 0 {
        txt(cmd, body, cx, "Maintenance in progress - Coming back soon", 34.0, RED, Wt::Bold);
        return;
    }
    // Online matchmaking is offline in this build: CASUAL lists the local worlds; clicking one loads it (ui::launch).
    txt(cmd, body, cx, "Pick a world to load into.", 22.0, GREY, Wt::Medium);
    let grid = add(cmd, body, Node { margin: UiRect::top(px(14)), flex_wrap: FlexWrap::Wrap, column_gap: px(24), row_gap: px(24), ..row() });
    let current = if crate::testmap::wanted() { "test" } else if crate::grid::wanted() { "grid" } else { "" };
    for (i, (name, map, zone, mode, desc)) in OFFLINE.iter().enumerate() {
        let here = *map == current && (zone.is_empty() || *map == "grid") && *mode == std::env::var("BODYCAM_MODE").unwrap_or_default();
        let card = button(cmd, grid, Node { width: px(390), height: px(200), padding: UiRect::all(px(12)), row_gap: px(10), ..col() }, Act::Offline(i), None, PANEL, PANEL_HOVER, Some(if here { GREEN } else { Color::srgb(0.2, 0.2, 0.22) }));
        txt(cmd, card, cx, name, 30.0, Color::WHITE, Wt::Extra);
        txt(cmd, card, cx, desc, 18.0, GREY, Wt::Medium);
        if here && zone.is_empty() {
            let st = add(cmd, card, Node { column_gap: px(6), margin: UiRect::top(Val::Auto), ..row() });
            txt(cmd, st, cx, "CURRENT", 20.0, GREEN, Wt::Bold);
            img(cmd, st, cx, "T_UI_Checkmark", 20.0, 20.0, GREEN);
        }
    }
}

fn icon_of(row: &str) -> String {
    match row {
        "DRAGUNOV" => "Weapons_Default_Dragunov".into(),
        "kriss vector" => "Weapons_Default_Kriss_Vector".into(),
        "SCAR" => "Weapons_Default_Scar".into(),
        "BC5" => "Weapons_Default_BK-102S".into(),
        r => format!("Weapons_Default_{r}"),
    }
}

/// DT_PerkItems rows of each gear slot as the game lists them (SWAP EXPLOSIVE / MELEE / GADGET): row, DT_PerkSkins ItemName,
/// ItemIcons (assets-local/ui/tex/icons), whether this build plays it (items.rs / perks.rs). The rest show locked, as on an
/// account that has not unlocked them.
const GEAR: [&[(&str, &str, &str, bool)]; 3] = [
    &[
        ("Grenade G67", "Frag Grenade", "icons/GrenadeG67", true),
        ("Grenade Flash 9-Bangers", "Flash 9-Bangers", "icons/GrenadeFlash9-Bangers", true),
        ("Grenade Flash", "Flash", "icons/GrenadeFlash", true),
        ("Grenade N67", "N67 Sticky", "icons/GrenadeN67", true),
        ("Grenade RGN", "RGN Impact", "icons/GrenadeRGN", true),
        ("Grenade G68", "MB78", "icons/GrenadeG68", true),
    ],
    &[
        ("Knife", "Knife", "icons/KnifeBaseReceiver", true),
        ("Karambit", "Karambit", "icons/KarambitBaseArcticEchoez", true),
        ("Folding", "Folding", "icons/FoldingBaseReceiver", true),
        ("Blade", "Blade", "icons/BladeBaseReceiver", true),
    ],
    &[
        ("Kamikaze", "Kamikaze", "icons/Kamikaze", true),
        ("Sentinel", "Sentinel", "icons/Sentinel", false),
        ("Trataxes", "Trataxes", "icons/Icon_Trataxes", false),
        ("Kyoshin", "Kyoshin", "icons/Icon_Kyoshin", false),
    ],
];

fn gear_icon(row: &str) -> &'static str {
    GEAR.iter().flat_map(|g| g.iter()).find(|g| g.0 == row).map_or("icons/DefaultOperator_02", |g| g.2)
}

/// Red padlock (the game's locked-item badge) drawn with nodes.
fn padlock(cmd: &mut Commands, p: Entity) {
    let m = add(cmd, p, Node { position_type: PositionType::Absolute, right: px(10), top: px(8), width: px(22), height: px(28), flex_direction: FlexDirection::Column, align_items: AlignItems::Center, ..default() });
    add(cmd, m, (Node { width: px(14), height: px(12), border: UiRect { left: px(3), right: px(3), top: px(3), bottom: px(0) }, border_radius: BorderRadius::top(px(7)), ..default() }, BorderColor::all(RED)));
    add(cmd, m, (Node { width: px(22), height: px(16), ..default() }, BackgroundColor(RED)));
}

const CARD_EDGE: Color = Color::srgb(0.2, 0.2, 0.22);

/// One loadout card (the real LOADOUT / weapon screen strip): slot name top-left, item image, item name.
fn card(cmd: &mut Commands, p: Entity, cx: &BuildCx, w: f32, h: f32, slot: &str, icon: &str, name: &str, act: Act, edge: Color) -> Entity {
    let c = button(cmd, p, Node { width: px(w), height: px(h), padding: UiRect::all(px(10)), flex_shrink: 0.0, ..col() }, act, None, Color::srgb(0.05, 0.05, 0.06), PANEL_HOVER, Some(edge));
    if !slot.is_empty() {
        txt(cmd, c, cx, slot, 22.0, Color::WHITE, Wt::Bold);
    }
    let m = add(cmd, c, Node { flex_grow: 1.0, width: percent(100), min_height: px(0), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() });
    if !icon.is_empty() {
        img(cmd, m, cx, icon, w - 30.0, (h - 90.0).max(30.0), Color::WHITE);
    }
    txt(cmd, c, cx, name, 26.0, Color::WHITE, Wt::Bold);
    c
}

/// OPERATOR PRIMARY SECONDARY EXPLOSIVE MELEE GADGET cards; `active` = the highlighted one (0 = operator .. 5 = gadget).
fn loadout_strip(cmd: &mut Commands, p: Entity, cx: &BuildCx, w: f32, h: f32, active: Option<usize>) {
    let row = add(cmd, p, Node { column_gap: px(10), ..row() });
    let gun = |r: &str| cx.roster.0.iter().find(|i| i.row == r).map_or(r.to_string(), |i| i.display.clone());
    let cards: [(&str, String, String, Act); 6] = [
        ("OPERATOR", "icons/DefaultOperator_02".into(), "Bodycam".into(), Act::Toast("Operator skins are not part of this build.".into())),
        ("PRIMARY", icon_of(&cx.s.primary), gun(&cx.s.primary), Act::Weapon(cx.s.primary.clone())),
        ("SECONDARY", icon_of(&cx.s.secondary), gun(&cx.s.secondary), Act::Weapon(cx.s.secondary.clone())),
        ("EXPLOSIVE", gear_icon(&cx.s.explosive).into(), crate::items::display_name(&cx.s.explosive).into(), Act::SwapKind(2)),
        ("MELEE", gear_icon(&cx.s.melee).into(), crate::items::display_name(&cx.s.melee).into(), Act::SwapKind(3)),
        ("GADGET", gear_icon(&cx.s.gadget).into(), crate::items::display_name(&cx.s.gadget).into(), Act::SwapKind(4)),
    ];
    for (i, (slot, icon, name, act)) in cards.into_iter().enumerate() {
        let ww = if i == 1 || i == 2 { w * 1.6 } else { w };
        card(cmd, row, cx, ww, h, slot, &icon, &name, act, if Some(i) == active { Color::WHITE } else { CARD_EDGE });
    }
}

fn foot_buttons(cmd: &mut Commands, p: Entity, cx: &BuildCx, items: &[(&str, Act, Option<KeyCode>, bool)]) -> Entity {
    let foot = add(cmd, p, Node { column_gap: px(16), margin: UiRect::top(px(14)), align_items: AlignItems::Center, ..row() });
    for (label, act, key, red) in items {
        let b = button(cmd, foot, Node { padding: UiRect::axes(px(16), px(8)), ..row() }, act.clone(), *key, if *red { RED } else { CLEAR }, PANEL_HOVER, if *red { None } else { Some(CARD_EDGE) });
        txt(cmd, b, cx, label, 26.0, Color::WHITE, Wt::Bold);
    }
    foot
}

fn loadout(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = main_tabs(cx, false, cmd, root, Some(("[SPACE] PLAY >", Act::Push(Screen::Play))));
    cmd.entity(body).insert(BackgroundColor(DARK));
    header(cmd, body, cx, "LOADOUT 1", "Manage, swap, customize, and upgrade your equipment.");
    // the equipped loadout row (red frame = selected); offline there is one loadout
    let r = add(cmd, body, (Node { padding: UiRect::all(px(10)), border: UiRect::all(px(3)), ..row() }, BorderColor::all(RED), BackgroundColor(Color::srgb(0.08, 0.03, 0.03))));
    loadout_strip(cmd, r, cx, 230.0, 300.0, None);
    let foot = add(cmd, body, Node { column_gap: px(20), margin: UiRect::top(px(20)), ..row() });
    let e = add(cmd, foot, (Node { padding: UiRect::axes(px(22), px(10)), column_gap: px(8), ..row() }, BackgroundColor(Color::srgb(0.1, 0.25, 0.12))));
    img(cmd, e, cx, "T_UI_Checkmark", 22.0, 22.0, GREEN);
    txt(cmd, e, cx, "EQUIPPED", 24.0, GREEN, Wt::Bold);
    let d = button(cmd, foot, Node { padding: UiRect::axes(px(22), px(10)), ..row() }, Act::Toast("Only one loadout is available offline.".into()), Some(KeyCode::KeyD), PANEL, PANEL_HOVER, None);
    txt(cmd, d, cx, "[D] DUPLICATE", 24.0, Color::WHITE, Wt::Bold);
    let n = button(cmd, foot, Node { padding: UiRect::axes(px(22), px(10)), ..row() }, Act::Toast("Only one loadout is available offline.".into()), Some(KeyCode::KeyC), PANEL, PANEL_HOVER, None);
    txt(cmd, n, cx, "[C] NEW LOADOUT", 24.0, Color::WHITE, Wt::Bold);
}

/// Title + rarity (top left of the weapon screens).
fn title(cmd: &mut Commands, p: Entity, cx: &BuildCx, name: &str) {
    let l = add(cmd, p, col());
    txt(cmd, l, cx, &name.to_uppercase(), 80.0, Color::WHITE, Wt::Extra);
    let rr = add(cmd, l, Node { column_gap: px(12), ..row() });
    img(cmd, rr, cx, "T_UI_Rarity0", 24.0, 24.0, Color::WHITE);
    txt(cmd, rr, cx, "Common", 26.0, GREY, Wt::Bold);
}

fn weapon(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = frame(cmd, root, cx, "< LOADOUTS", None, &[], 0, &|_| Act::Noop);
    cmd.entity(body).insert(BackgroundColor(DARK));
    let Some(it) = cx.roster.0.iter().find(|i| i.row == cx.ui.weapon) else { return };
    title(cmd, body, cx, &it.display);
    let mid = add(cmd, body, Node { flex_grow: 1.0, min_height: px(0), width: percent(100), justify_content: JustifyContent::Center, ..row() });
    show(cmd, mid, cx, 520.0);
    loadout_strip(cmd, body, cx, 190.0, 200.0, Some(if it.slot == Slot::Primary { 1 } else { 2 }));
    let kind = if it.slot == Slot::Primary { 0 } else { 1 };
    let foot = foot_buttons(cmd, body, cx, &[
        ("[ESC] BACK", Act::Back, Some(KeyCode::Escape), false),
        ("[SPACE] MODIFY", Act::Push(Screen::Modify), Some(KeyCode::Space), false),
        (if kind == 0 { "[F] SWAP PRIMARY" } else { "[F] SWAP SECONDARY" }, Act::SwapKind(kind), Some(KeyCode::KeyF), true),
    ]);
    for (s, d) in [("<", -1), (">", 1)] {
        let b = button(cmd, foot, Node { padding: UiRect::axes(px(18), px(4)), margin: UiRect::left(px(if d < 0 { 200.0 } else { 0.0 })), ..row() }, Act::Cycle(d), Some(if d < 0 { KeyCode::ArrowLeft } else { KeyCode::ArrowRight }), CLEAR, PANEL_HOVER, None);
        txt(cmd, b, cx, s, 40.0, Color::WHITE, Wt::Extra);
    }
}

fn show(cmd: &mut Commands, p: Entity, cx: &BuildCx, h: f32) {
    let w = h * super::showcase::W as f32 / super::showcase::H as f32;
    add(cmd, p, (ImageNode::new(cx.show.clone()), Node { width: px(w), height: px(h), ..default() }));
}

/// Category of an attachment inside its slot (ItemClassID segment after the slot: "Close Range", "Suppressor"...).
pub fn cat_of(id: &str) -> String {
    crate::attach::data().items.get(id).and_then(|i| i.cls.split_once('.')).map_or(String::new(), |(_, c)| c.split('.').next().unwrap_or("").to_string())
}

/// SELECT category tabs of slot `slot` (index into attach::slots) on `row`, in the game's order where it has one.
pub fn select_cats(s: &crate::ui::settings::Settings, row: &str, slot: usize) -> Vec<String> {
    const ORDER: [&str; 4] = ["Iron Sights", "Close Range", "Medium Range", "Long Range"];
    let Some(sl) = crate::attach::slots(s, row).get(slot).copied() else { return vec![] };
    let mut v: Vec<String> = crate::attach::available(row).iter().filter(|i| crate::attach::data().items[i.as_str()].slot == sl).map(|i| cat_of(i)).collect();
    v.sort_by_key(|c| (ORDER.iter().position(|o| o == c).unwrap_or(ORDER.len()), c.clone()));
    v.dedup();
    v
}

/// The mounted part in slot `slot` (default or pick).
pub fn slot_part(s: &crate::ui::settings::Settings, row: &str, slot: usize) -> Option<String> {
    let sl = crate::attach::slots(s, row).get(slot).copied()?;
    crate::attach::loadout(s, row).into_iter().find(|id| crate::attach::data().items.get(id.as_str()).is_some_and(|i| i.slot == sl))
}

fn att_icon(id: &str) -> String {
    crate::attach::data().items.get(id).and_then(|i| i.icon.clone()).map_or(String::new(), |i| format!("icons/{i}"))
}

fn modify(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let Some(it) = cx.roster.0.iter().find(|i| i.row == cx.ui.weapon) else { return };
    let body = frame(cmd, root, cx, &format!("< MODIFY {}", it.display.to_uppercase()), None, &[], 0, &|_| Act::Noop);
    cmd.entity(body).insert(BackgroundColor(DARK));
    title(cmd, body, cx, &it.display);
    let mid = add(cmd, body, Node { flex_grow: 1.0, min_height: px(0), width: percent(100), align_items: AlignItems::Center, ..row() });
    // WeaponAttributeSet values of this build (GE_WeaponStat + the parts' GE_AttachmentStat)
    let stats = add(cmd, mid, Node { width: px(260), row_gap: px(8), ..col() });
    let st = |a: &str, b: f32| crate::attach::stat(cx.s, &it.row, a, b);
    for (k, v) in [("Fire Rate", format!("{:.2}", st("FireRate", it.fire_rate))), ("Damage", format!("{:.0}", st("Damage", it.damage))), ("Mag Size", format!("{:.0}", st("AmmoCapacity", it.cap).round()))] {
        let c = add(cmd, stats, (Node { padding: UiRect::axes(px(12), px(6)), column_gap: px(14), border: UiRect::all(px(2)), ..row() }, BorderColor::all(CARD_EDGE)));
        txt(cmd, c, cx, &format!("> {k}"), 22.0, Color::WHITE, Wt::Bold);
        txt(cmd, c, cx, &v, 22.0, Color::WHITE, Wt::Bold);
    }
    show(cmd, mid, cx, 420.0);
    let slots = crate::attach::slots(cx.s, &it.row);
    let strip = add(cmd, body, Node { width: percent(100), column_gap: px(8), row_gap: px(8), flex_wrap: FlexWrap::Wrap, ..row() });
    for (n, sl) in slots.iter().enumerate() {
        let part = slot_part(cx.s, &it.row, n);
        let (icon, name) = part.as_deref().map_or((String::new(), "+".to_string()), |id| (att_icon(id), crate::attach::label(id).to_string()));
        card(cmd, strip, cx, 170.0, 165.0, &sl.replace('_', " ").to_uppercase(), &icon, &name, Act::AttSlot(n), CARD_EDGE);
    }
    let foot = foot_buttons(cmd, body, cx, &[("[ESC] BACK", Act::Back, Some(KeyCode::Escape), false)]);
    txt(cmd, foot, cx, &format!("[{} Attachments]", crate::attach::available(&it.row).len()), 24.0, GREY, Wt::Bold);
}

fn select(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let wrow = cx.ui.weapon.clone();
    let slots = crate::attach::slots(cx.s, &wrow);
    let Some(sl) = slots.get(cx.ui.att_slot).copied() else { return };
    let cats = select_cats(cx.s, &wrow, cx.ui.att_slot);
    let cat = cats.get(cx.ui.att_cat).cloned().unwrap_or_default();
    let tabs: Vec<String> = cats.iter().map(|c| if c.is_empty() { "STANDARD".into() } else { c.to_uppercase() }).collect();
    let tabs: Vec<&str> = if tabs.len() > 1 { tabs.iter().map(String::as_str).collect() } else { vec![] };
    let body = frame(cmd, root, cx, &format!("< SELECT {}", sl.replace('_', " ").to_uppercase()), None, &tabs, cx.ui.att_cat, &|i| match i {
        usize::MAX => Act::Tab(-1),
        i if i == usize::MAX - 1 => Act::Tab(1),
        i => Act::SetTab(i),
    });
    cmd.entity(body).insert(BackgroundColor(DARK));
    let cur = slot_part(cx.s, &wrow, cx.ui.att_slot);
    title(cmd, body, cx, cur.as_deref().map_or("NONE", crate::attach::label));
    let mid = add(cmd, body, Node { flex_grow: 1.0, min_height: px(0), width: percent(100), justify_content: JustifyContent::Center, ..row() });
    show(cmd, mid, cx, 360.0);
    let ids: Vec<&String> = crate::attach::available(&wrow).iter().filter(|i| crate::attach::data().items[i.as_str()].slot == sl && cat_of(i) == cat).collect();
    let list = add(cmd, body, (Node { width: percent(100), height: px(380), column_gap: px(10), row_gap: px(10), flex_wrap: FlexWrap::Wrap, align_content: AlignContent::FlexStart, overflow: Overflow::scroll_y(), ..row() }, ScrollPosition(Vec2::new(0.0, cx.ui.scroll[3])), Scroller(3)));
    for id in &ids {
        let on = cur.as_deref() == Some(id.as_str());
        let name = format!("{}{}", crate::attach::label(id), if crate::attach::is_default(&wrow, id) { " *" } else { "" });
        let c = card(cmd, list, cx, 200.0, 180.0, "", &att_icon(id), &name, Act::Att(id.as_str()), if on { Color::WHITE } else { CARD_EDGE });
        if on {
            let m = add(cmd, c, Node { position_type: PositionType::Absolute, right: px(8), top: px(8), ..default() });
            img(cmd, m, cx, "T_UI_Checkmark", 28.0, 28.0, Color::WHITE);
        }
    }
    let foot = foot_buttons(cmd, body, cx, &[("[ESC] BACK", Act::Back, Some(KeyCode::Escape), false), ("[SPACE] UNEQUIP", Act::Unequip, Some(KeyCode::Space), false)]);
    let k = ids.iter().position(|i| cur.as_deref() == Some(i.as_str())).map_or(0, |p| p + 1);
    txt(cmd, foot, cx, &format!("[{k}/{}]   * = default part", ids.len()), 24.0, GREY, Wt::Bold);
}

/// SWAP tabs: gun classes of the slot ("Assault Rifles"...), or one tab for the gear slots.
pub fn swap_cats(roster: &Roster, kind: usize) -> Vec<String> {
    if kind >= 2 {
        return vec![];
    }
    let slot = if kind == 0 { Slot::Primary } else { Slot::Secondary };
    let mut v: Vec<String> = roster.0.iter().filter(|i| i.slot == slot).filter_map(|i| crate::attach::data().classes.get(&i.row).cloned()).collect();
    v.sort();
    v.dedup();
    v
}

fn swap(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let kind = cx.ui.swap;
    let names = ["PRIMARY", "SECONDARY", "EXPLOSIVE", "MELEE", "GADGET"];
    let cats = swap_cats(cx.roster, kind);
    let tabs: Vec<String> = cats.iter().map(|c| c.to_uppercase()).collect();
    let tabs: Vec<&str> = tabs.iter().map(String::as_str).collect();
    let body = frame(cmd, root, cx, &format!("< SWAP {}", names[kind]), None, &tabs, cx.ui.swap_cat, &|i| match i {
        usize::MAX => Act::Tab(-1),
        i if i == usize::MAX - 1 => Act::Tab(1),
        i => Act::SetTab(i),
    });
    cmd.entity(body).insert(BackgroundColor(DARK));
    // (row, display, icon, equipped, action)
    let opts: Vec<(String, String, String, bool, Act)> = if kind < 2 {
        let slot = if kind == 0 { Slot::Primary } else { Slot::Secondary };
        let eq = if kind == 0 { &cx.s.primary } else { &cx.s.secondary };
        let cat = cats.get(cx.ui.swap_cat);
        cx.roster.0.iter().filter(|i| i.slot == slot && crate::attach::data().classes.get(&i.row) == cat).map(|i| (i.row.clone(), i.display.clone(), icon_of(&i.row), &i.row == eq, Act::SwapGun(i.row.clone()))).collect()
    } else {
        let cur = [&cx.s.explosive, &cx.s.melee, &cx.s.gadget][kind - 2];
        GEAR[kind - 2]
            .iter()
            .map(|(r, name, icon, ok)| {
                let act = if *ok { Act::Gear(kind - 2, r.to_string()) } else { Act::Toast(format!("{name} is locked.")) };
                (r.to_string(), name.to_string(), icon.to_string(), *r == cur.as_str(), act)
            })
            .collect()
    };
    let cur = opts.iter().find(|o| o.3);
    title(cmd, body, cx, cur.map_or("", |o| o.1.as_str()));
    let mid = add(cmd, body, Node { flex_grow: 1.0, min_height: px(0), width: percent(100), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..row() });
    if let Some(o) = cur {
        img(cmd, mid, cx, &o.2, if kind < 2 { 900.0 } else { 420.0 }, if kind < 2 { 450.0 } else { 420.0 }, Color::WHITE);
    }
    let list = add(cmd, body, Node { width: percent(100), column_gap: px(10), row_gap: px(10), flex_wrap: FlexWrap::Wrap, ..row() });
    for (_, name, icon, on, act) in &opts {
        let c = card(cmd, list, cx, if kind < 2 { 300.0 } else { 200.0 }, 190.0, "", icon, name, act.clone(), if *on { Color::WHITE } else { CARD_EDGE });
        if matches!(act, Act::Toast(_)) {
            padlock(cmd, c);
        }
        if *on {
            let m = add(cmd, c, Node { position_type: PositionType::Absolute, right: px(8), top: px(8), ..default() });
            img(cmd, m, cx, "T_UI_Checkmark", 28.0, 28.0, Color::WHITE);
        }
    }
    let foot = foot_buttons(cmd, body, cx, &[("[ESC] BACK", Act::Back, Some(KeyCode::Escape), false)]);
    let open = opts.iter().filter(|o| !matches!(o.4, Act::Toast(_))).count();
    txt(cmd, foot, cx, &format!("[{open}/{}]", opts.len()), 24.0, GREY, Wt::Bold);
    if cur.is_some() {
        let e = add(cmd, foot, (Node { padding: UiRect::axes(px(16), px(8)), column_gap: px(8), ..row() }, BackgroundColor(Color::srgb(0.1, 0.25, 0.12))));
        img(cmd, e, cx, "T_UI_Checkmark", 22.0, 22.0, GREEN);
        txt(cmd, e, cx, "EQUIPPED", 24.0, GREEN, Wt::Bold);
    }
}

/// W_ScoreboardSubmenu stand-in: deathmatch table (kills / deaths), first to 40 wins (DA_GameModeDeathmatch).
fn scoreboard(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = frame(cmd, root, cx, "[ESC] BACK", None, &[], 0, &|_| Act::Noop);
    cmd.entity(body).insert(BackgroundColor(DARK));
    header(cmd, body, cx, "DEATHMATCH", "First to 40 kills wins.");
    let rows = crate::net::SCORES.lock().map(|t| t.clone()).unwrap_or_default();
    let me = crate::net::MY_ID.load(std::sync::atomic::Ordering::Relaxed);
    if rows.is_empty() {
        txt(cmd, body, cx, "No kills yet.", 28.0, Color::WHITE, Wt::Medium);
    }
    for (id, k, d) in rows {
        let who = if id == me { "YOU".to_string() } else { format!("PLAYER {id}") };
        txt(cmd, body, cx, &format!("{who}    {k} kills    {d} deaths"), 30.0, if id == me { GREEN } else { Color::WHITE }, Wt::Bold);
    }
}

fn menu(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = frame(cmd, root, cx, "[ESC] BACK", None, &[], 0, &|_| Act::Noop);
    cmd.entity(body).insert(BackgroundColor(DARK));
    let grid = add(cmd, body, Node { margin: UiRect::top(px(60)), flex_wrap: FlexWrap::Wrap, column_gap: px(30), row_gap: px(30), justify_content: JustifyContent::Center, ..row() });
    for (name, icon, act) in [
        ("SETTINGS", "T_UI_SettingsIcon", Act::Push(Screen::Settings)),
        ("PARTY", "T_UI_SocialIcon", Act::Push(Screen::Social)),
        ("COMMUNITY", "T_UI_Discord", Act::Toast("Community links open in a browser: offline.".into())),
        ("EXIT GAME", "T_UI_ExitIcon", Act::Exit),
        ("REPORT BUG", "T_UI_Bug", Act::Toast("Bug reports are sent online: offline.".into())),
        ("FEATURES UPVOTE", "T_UI_Vote", Act::Toast("Feature voting is online only.".into())),
        ("SCOREBOARD", "T_UI_Leader", Act::Push(Screen::Scoreboard)),
        ("CREDITS", "T_UI_Leader", Act::Push(Screen::Credits)),
    ] {
        let t = button(cmd, grid, Node { width: px(400), height: px(260), flex_direction: FlexDirection::Column, justify_content: JustifyContent::Center, align_items: AlignItems::Center, row_gap: px(18), ..default() }, act, None, PANEL, PANEL_HOVER, Some(Color::srgb(0.2, 0.2, 0.22)));
        img(cmd, t, cx, icon, 110.0, 110.0, Color::WHITE);
        txt(cmd, t, cx, name, 34.0, Color::WHITE, Wt::Bold);
    }
}

pub const TABS: [&str; 7] = ["VIDEO", "GRAPHICS", "AUDIO", "GAME", "KEYBOARD", "GAMEPAD", "DRONE"];

fn sections(tab: usize) -> Vec<(&'static str, Vec<Set>)> {
    match tab {
        0 => vec![("DISPLAY", vec![Set::WindowMode, Set::Resolution, Set::ResScale]), ("FRAMERATE", vec![Set::VSync, Set::FpsLimit]), ("IMAGE", vec![Set::Gamma])],
        1 => vec![("CAMERA", vec![Set::LensFx])],
        2 => vec![("VOLUME", vec![Set::Master, Set::Sfx])],
        3 => vec![("CONTROLS", vec![Set::Sensitivity, Set::Fov])],
        _ => vec![],
    }
}

fn settings_screen(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = frame(cmd, root, cx, "< SETTINGS", None, &TABS, cx.ui.tab, &|i| match i {
        usize::MAX => Act::Tab(-1),
        i if i == usize::MAX - 1 => Act::Tab(1),
        i => Act::SetTab(i),
    });
    cmd.entity(body).insert(BackgroundColor(DARK));
    let cols = add(cmd, body, Node { flex_grow: 1.0, min_height: px(0), column_gap: px(40), ..row() });
    cmd.entity(cols).insert(Node { flex_grow: 1.0, min_height: px(0), column_gap: px(40), align_items: AlignItems::Stretch, ..row() });
    let list = add(cmd, cols, (Node { width: px(1100), overflow: Overflow::scroll_y(), padding: UiRect::right(px(20)), ..col() }, ScrollPosition(Vec2::new(0.0, cx.ui.scroll[2])), Scroller(2)));
    if cx.ui.tab == 4 {
        for (i, (_, label, _)) in ACTIONS.iter().enumerate() {
            let r = add(cmd, list, (Node { width: percent(100), height: px(64), flex_shrink: 0.0, padding: UiRect::axes(px(20), px(0)), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, margin: UiRect::bottom(px(6)), ..default() }, BackgroundColor(PANEL)));
            txt(cmd, r, cx, label, 28.0, Color::WHITE, Wt::Medium);
            let b = button(cmd, r, Node { width: px(260), height: px(46), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, Act::Rebind(i), None, Color::srgb(0.16, 0.16, 0.18), PANEL_HOVER, Some(if cx.ui.rebinding == Some(i) { RED } else { Color::srgb(0.3, 0.3, 0.32) }));
            let t = txt(cmd, b, cx, &if cx.ui.rebinding == Some(i) { "PRESS A KEY".into() } else { settings::key_name(settings::bound(cx.s, ACTIONS[i].0)).to_uppercase() }, 26.0, Color::WHITE, Wt::Bold);
            cmd.entity(t).insert(KeyLabel(i));
        }
    } else if sections(cx.ui.tab).is_empty() {
        txt(cmd, list, cx, "No settings available in this build.", 30.0, GREY, Wt::Medium);
    }
    for (title, sets) in sections(cx.ui.tab) {
        let h = txt(cmd, list, cx, title, 30.0, RED, Wt::Extra);
        cmd.entity(h).insert(Node { margin: UiRect::new(px(0), px(0), px(14), px(10)), ..default() });
        for s in sets {
            let sp = settings::spec(s);
            let r = add(cmd, list, (Node { width: percent(100), height: px(76), flex_shrink: 0.0, padding: UiRect::axes(px(20), px(0)), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, margin: UiRect::bottom(px(6)), ..default() }, BackgroundColor(PANEL), Btn { act: Act::Noop, key: None, bg: PANEL, hover: PANEL_HOVER, set: Some(s) }));
            txt(cmd, r, cx, sp.label, 28.0, Color::WHITE, Wt::Medium);
            let c = add(cmd, r, Node { column_gap: px(14), ..row() });
            match sp.kind {
                Kind::Slider { .. } => {
                    let track = button(cmd, c, Node { width: px(420), height: px(30), align_items: AlignItems::Center, ..default() }, Act::Slide(s), None, Color::srgb(0.2, 0.2, 0.22), Color::srgb(0.26, 0.26, 0.28), None);
                    let fill = add(cmd, track, (Node { height: percent(100), width: percent(0), ..default() }, BackgroundColor(RED), SliderFill(s)));
                    let _ = fill;
                    let v = txt(cmd, c, cx, "", 26.0, Color::WHITE, Wt::Bold);
                    cmd.entity(v).insert((ValueText(s), Node { width: px(110), ..default() }));
                }
                Kind::Select => {
                    let l = button(cmd, c, Node { padding: UiRect::axes(px(16), px(6)), ..row() }, Act::Step(s, -1), None, CLEAR, PANEL_HOVER, None);
                    txt(cmd, l, cx, "<", 30.0, Color::WHITE, Wt::Extra);
                    let v = txt(cmd, c, cx, "", 28.0, Color::WHITE, Wt::Bold);
                    cmd.entity(v).insert((ValueText(s), Node { width: px(210), justify_content: JustifyContent::Center, ..default() }, TextLayout { justify: Justify::Center, ..default() }));
                    let r = button(cmd, c, Node { padding: UiRect::axes(px(16), px(6)), ..row() }, Act::Step(s, 1), None, CLEAR, PANEL_HOVER, None);
                    txt(cmd, r, cx, ">", 30.0, Color::WHITE, Wt::Extra);
                }
                Kind::Toggle => {
                    let b = button(cmd, c, Node { width: px(140), height: px(46), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, Act::Toggle(s), None, Color::srgb(0.16, 0.16, 0.18), PANEL_HOVER, Some(Color::srgb(0.3, 0.3, 0.32)));
                    let v = txt(cmd, b, cx, "", 26.0, Color::WHITE, Wt::Bold);
                    cmd.entity(v).insert(ValueText(s));
                }
            }
        }
    }
    let side = add(cmd, cols, (Node { width: px(560), padding: UiRect::all(px(28)), row_gap: px(14), ..col() }, BackgroundColor(PANEL)));
    let t = txt(cmd, side, cx, "", 38.0, Color::WHITE, Wt::Extra);
    cmd.entity(t).insert(HelpText(true));
    let d = txt(cmd, side, cx, "", 26.0, GREY, Wt::Medium);
    cmd.entity(d).insert(HelpText(false));
    let foot = add(cmd, body, Node { column_gap: px(36), margin: UiRect::top(px(10)), ..row() });
    for (label, act, key) in [("[ESC] BACK", Act::Back, KeyCode::Escape), ("[T] RESET SELECTED", Act::ResetSel, KeyCode::KeyT), ("[R] RESET ALL", Act::ResetAll, KeyCode::KeyR)] {
        let b = button(cmd, foot, Node { padding: UiRect::axes(px(8), px(6)), ..row() }, act, Some(key), CLEAR, PANEL_HOVER, None);
        txt(cmd, b, cx, label, 26.0, Color::WHITE, Wt::Bold);
    }
}

fn social(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = frame(cmd, root, cx, "< SOCIAL", None, &[], 0, &|_| Act::Noop);
    cmd.entity(body).insert(BackgroundColor(DARK));
    txt(cmd, body, cx, "PARTY [1]", 40.0, Color::WHITE, Wt::Extra);
    let card = add(cmd, body, (Node { margin: UiRect::vertical(px(20)), padding: UiRect::all(px(16)), column_gap: px(20), width: px(560), ..row() }, BackgroundColor(PANEL)));
    img(cmd, card, cx, "T_UI_ProfilePicture", 96.0, 96.0, Color::WHITE);
    let c = add(cmd, card, col());
    txt(cmd, c, cx, &std::env::var("USERNAME").unwrap_or_else(|_| "PLAYER".into()).to_uppercase(), 34.0, Color::WHITE, Wt::Bold);
    let l = add(cmd, c, Node { column_gap: px(8), ..row() });
    img(cmd, l, cx, "T_UI_Leader", 22.0, 22.0, Color::WHITE);
    txt(cmd, l, cx, "Party leader", 22.0, GREY, Wt::Medium);
    txt(cmd, body, cx, "FRIENDS", 40.0, Color::WHITE, Wt::Extra);
    txt(cmd, body, cx, "The Party System is in maintenance. To play with a friend, invite them to your Shooting Range, then HOST a match.", 26.0, GREY, Wt::Medium);
}

fn credits(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = frame(cmd, root, cx, "< CREDITS", None, &[], 0, &|_| Act::Noop);
    cmd.entity(body).insert(BackgroundColor(DARK));
    txt(cmd, body, cx, "CREDITS", 52.0, Color::WHITE, Wt::Extra);
    let t = txt(cmd, body, cx, "Bodycam is made by its original developers. This is an unofficial offline Rust/Bevy recreation of the first-person player for study. See the original game's credits for the full list.", 28.0, GREY, Wt::Medium);
    cmd.entity(t).insert(Node { max_width: px(1000), margin: UiRect::top(px(20)), ..default() });
}

pub fn default_help(tab: usize) -> Option<Set> {
    sections(tab).first().and_then(|s| s.1.first().copied())
}
