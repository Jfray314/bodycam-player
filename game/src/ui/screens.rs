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

fn item_card(cmd: &mut Commands, p: Entity, cx: &BuildCx, it: &Item, equipped: bool) {
    let c = button(cmd, p, Node { width: px(300), flex_shrink: 0.0, padding: UiRect::all(px(8)), margin: UiRect::bottom(px(14)), ..col() }, Act::Weapon(it.row.clone()), None, PANEL, PANEL_HOVER, Some(if equipped { GREEN } else { Color::srgb(0.2, 0.2, 0.22) }));
    img(cmd, c, cx, &icon_of(&it.row), 280.0, 140.0, Color::WHITE);
    let r = add(cmd, c, Node { justify_content: JustifyContent::SpaceBetween, width: percent(100), ..row() });
    txt(cmd, r, cx, &it.display.to_uppercase(), 26.0, Color::WHITE, Wt::Bold);
    if equipped {
        let g = add(cmd, r, Node { column_gap: px(5), ..row() });
        img(cmd, g, cx, "T_UI_Checkmark", 18.0, 18.0, GREEN);
    }
}

fn loadout(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = main_tabs(cx, false, cmd, root, Some(("[SPACE] PLAY >", Act::Push(Screen::Play))));
    cmd.entity(body).insert(BackgroundColor(DARK));
    header(cmd, body, cx, "LOADOUT 4", "Manage, swap, customize, and upgrade your equipment.");
    let cols = add(cmd, body, Node { column_gap: px(26), flex_grow: 1.0, min_height: px(0), ..row() });
    cmd.entity(cols).insert(Node { column_gap: px(26), flex_grow: 1.0, min_height: px(0), align_items: AlignItems::FlexStart, ..row() });
    let static_col = |cmd: &mut Commands, name: &str, item: &str, msg: &str| {
        let c = add(cmd, cols, Node { width: px(300), ..col() });
        txt(cmd, c, cx, name, 22.0, GREY, Wt::Bold);
        let b = button(cmd, c, Node { width: px(300), height: px(190), margin: UiRect::top(px(8)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, ..default() }, Act::Toast(msg.into()), None, PANEL, PANEL_HOVER, Some(Color::srgb(0.2, 0.2, 0.22)));
        txt(cmd, b, cx, item, 28.0, Color::WHITE, Wt::Bold);
    };
    static_col(cmd, "OPERATOR", "BODYCAM", "Operator skins are not part of this build.");
    for (name, slot) in [("PRIMARY", Slot::Primary), ("SECONDARY", Slot::Secondary)] {
        let c = add(cmd, cols, Node { width: px(320), height: percent(100), ..col() });
        txt(cmd, c, cx, name, 22.0, GREY, Wt::Bold);
        let list = add(cmd, c, (Node { margin: UiRect::top(px(8)), flex_grow: 1.0, min_height: px(0), overflow: Overflow::scroll_y(), ..col() }, ScrollPosition(Vec2::new(0.0, cx.ui.scroll[slot as usize])), Scroller(slot as usize)));
        let eq = if slot == Slot::Primary { &cx.s.primary } else { &cx.s.secondary };
        for it in cx.roster.0.iter().filter(|i| i.slot == slot) {
            item_card(cmd, list, cx, it, &it.row == eq);
        }
    }
    // EXPLOSIVE / MELEE / GADGET: the real items (items.rs / perks.rs); the card with the check is the equipped one.
    for (i, (name, rows, cur)) in [("EXPLOSIVE", &["Grenade G67", "Grenade Flash"][..], &cx.s.explosive), ("MELEE", &["Knife"][..], &cx.s.melee), ("GADGET", &["Kamikaze"][..], &cx.s.gadget)].into_iter().enumerate() {
        let c = add(cmd, cols, Node { width: px(250), ..col() });
        txt(cmd, c, cx, name, 22.0, GREY, Wt::Bold);
        for r in rows {
            let on = *r == cur.as_str();
            let b = button(cmd, c, Node { width: px(250), height: px(120), margin: UiRect::top(px(8)), justify_content: JustifyContent::Center, align_items: AlignItems::Center, column_gap: px(8), ..row() }, Act::Gear(i, r.to_string()), None, PANEL, PANEL_HOVER, Some(if on { GREEN } else { Color::srgb(0.2, 0.2, 0.22) }));
            txt(cmd, b, cx, &crate::items::display_name(r).to_uppercase(), 22.0, Color::WHITE, Wt::Bold);
            if on {
                img(cmd, b, cx, "T_UI_Checkmark", 18.0, 18.0, GREEN);
            }
        }
    }
    let foot = add(cmd, body, Node { column_gap: px(20), margin: UiRect::top(px(10)), ..row() });
    let e = add(cmd, foot, (Node { padding: UiRect::axes(px(22), px(10)), column_gap: px(8), ..row() }, BackgroundColor(Color::srgb(0.1, 0.25, 0.12))));
    img(cmd, e, cx, "T_UI_Checkmark", 22.0, 22.0, GREEN);
    txt(cmd, e, cx, "EQUIPPED", 24.0, GREEN, Wt::Bold);
    let d = button(cmd, foot, Node { padding: UiRect::axes(px(22), px(10)), ..row() }, Act::Toast("Only one loadout is available offline.".into()), Some(KeyCode::KeyD), PANEL, PANEL_HOVER, None);
    txt(cmd, d, cx, "[D] DUPLICATE", 24.0, Color::WHITE, Wt::Bold);
    let n = button(cmd, foot, Node { padding: UiRect::axes(px(22), px(10)), ..row() }, Act::Toast("Only one loadout is available offline.".into()), Some(KeyCode::KeyC), PANEL, PANEL_HOVER, None);
    txt(cmd, n, cx, "[C] NEW LOADOUT", 24.0, Color::WHITE, Wt::Bold);
}

fn weapon(cmd: &mut Commands, root: Entity, cx: &BuildCx) {
    let body = frame(cmd, root, cx, "< LOADOUTS", None, &[], 0, &|_| Act::Noop);
    cmd.entity(body).insert(BackgroundColor(DARK));
    let Some(it) = cx.roster.0.iter().find(|i| i.row == cx.ui.weapon) else { return };
    let slot_name = if it.slot == Slot::Primary { "PRIMARY" } else { "SECONDARY" };
    let eq = if it.slot == Slot::Primary { &cx.s.primary } else { &cx.s.secondary } == &it.row;
    let top = add(cmd, body, Node { justify_content: JustifyContent::SpaceBetween, width: percent(100), ..row() });
    let l = add(cmd, top, col());
    txt(cmd, l, cx, &it.display.to_uppercase(), 92.0, Color::WHITE, Wt::Extra);
    let rr = add(cmd, l, Node { column_gap: px(12), ..row() });
    img(cmd, rr, cx, "T_UI_Rarity0", 24.0, 24.0, Color::WHITE);
    txt(cmd, rr, cx, "Common", 28.0, GREY, Wt::Bold);
    if eq {
        img(cmd, rr, cx, "T_UI_Checkmark", 22.0, 22.0, GREEN);
        txt(cmd, rr, cx, "EQUIPPED", 24.0, GREEN, Wt::Bold);
    }
    let st = add(cmd, top, Node { padding: UiRect::all(px(16)), ..col() });
    cmd.entity(st).insert(BackgroundColor(PANEL));
    for (k, v) in [("DAMAGE", format!("{:.0}", it.damage)), ("MAGAZINE", format!("{:.0}", it.cap)), ("SLOT", slot_name.into())] {
        let r = add(cmd, st, Node { width: px(320), justify_content: JustifyContent::SpaceBetween, ..row() });
        txt(cmd, r, cx, k, 24.0, GREY, Wt::Medium);
        txt(cmd, r, cx, &v, 24.0, Color::WHITE, Wt::Bold);
    }
    let mid = add(cmd, body, Node { flex_grow: 1.0, width: percent(100), justify_content: JustifyContent::SpaceBetween, align_items: AlignItems::Center, ..row() });
    let a = button(cmd, mid, Node { padding: UiRect::axes(px(26), px(60)), ..row() }, Act::Cycle(-1), Some(KeyCode::ArrowLeft), CLEAR, PANEL_HOVER, None);
    txt(cmd, a, cx, "<", 60.0, Color::WHITE, Wt::Extra);
    img(cmd, mid, cx, &icon_of(&it.row), 640.0, 320.0, Color::WHITE); // smaller than the art's 1000x500 so the MODIFY list fits
    let b = button(cmd, mid, Node { padding: UiRect::axes(px(26), px(60)), ..row() }, Act::Cycle(1), Some(KeyCode::ArrowRight), CLEAR, PANEL_HOVER, None);
    txt(cmd, b, cx, ">", 60.0, Color::WHITE, Wt::Extra);
    let strip = add(cmd, body, Node { column_gap: px(10), ..row() });
    for o in cx.roster.0.iter().filter(|o| o.slot == it.slot) {
        let c = button(cmd, strip, Node { padding: UiRect::all(px(4)), ..default() }, Act::Weapon(o.row.clone()), None, PANEL, PANEL_HOVER, Some(if o.row == it.row { RED } else { Color::srgb(0.2, 0.2, 0.22) }));
        img(cmd, c, cx, &icon_of(&o.row), 96.0, 48.0, Color::WHITE);
    }
    // MODIFY: every attachment that fits (attach::available), by slot; a pick replaces the gun's default part of that slot.
    let slots = crate::attach::slots(cx.s, &it.row);
    let sel = cx.ui.att_slot.min(slots.len().saturating_sub(1));
    let tabs = add(cmd, body, Node { column_gap: px(8), row_gap: px(8), margin: UiRect::top(px(10)), flex_wrap: FlexWrap::Wrap, ..row() });
    for (n, s) in slots.iter().enumerate() {
        let b = button(cmd, tabs, Node { padding: UiRect::axes(px(12), px(6)), ..row() }, Act::AttSlot(n), None, if n == sel { RED } else { PANEL }, PANEL_HOVER, None);
        txt(cmd, b, cx, &s.replace('_', " ").to_uppercase(), 20.0, Color::WHITE, Wt::Bold);
    }
    let mounted = crate::attach::loadout(cx.s, &it.row);
    let list = add(cmd, body, (Node { height: px(330), width: percent(100), margin: UiRect::top(px(8)), column_gap: px(8), row_gap: px(8), flex_wrap: FlexWrap::Wrap, align_content: AlignContent::FlexStart, overflow: Overflow::scroll_y(), ..row() }, ScrollPosition(Vec2::new(0.0, cx.ui.scroll[3])), Scroller(3)));
    if let Some(slot) = slots.get(sel) {
        for id in crate::attach::available(&it.row).iter().filter(|id| crate::attach::data().items[id.as_str()].slot == *slot) {
            let on = mounted.contains(id);
            let tag = if crate::attach::is_default(&it.row, id) { " (DEFAULT)" } else { "" };
            let b = button(cmd, list, Node { padding: UiRect::axes(px(12), px(6)), ..row() }, Act::Att(id.as_str()), None, if on { GREEN } else { PANEL }, PANEL_HOVER, None);
            txt(cmd, b, cx, &format!("{}{tag}", crate::attach::label(id).to_uppercase()), 24.0, Color::WHITE, Wt::Bold);
        }
    }
    let foot = add(cmd, body, Node { column_gap: px(36), margin: UiRect::top(px(14)), ..row() });
    for (label, act, key) in [
        ("[ESC] BACK".to_string(), Act::Back, KeyCode::Escape),
        (format!("[F] SWAP {slot_name}"), Act::Equip, KeyCode::KeyF),
    ] {
        let b = button(cmd, foot, Node { padding: UiRect::axes(px(8), px(6)), ..row() }, act, Some(key), CLEAR, PANEL_HOVER, None);
        txt(cmd, b, cx, &label, 26.0, Color::WHITE, Wt::Bold);
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
