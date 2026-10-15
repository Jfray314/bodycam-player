//! LAN / internet multiplayer (2-8 players): a small UDP protocol (bincode over std::net::UdpSocket), no extra crates beyond bincode.
//! Host-authoritative for damage / kills / respawn bookkeeping; every client moves its own player (client-side prediction = the normal
//! controller) and sends its state at 30 Hz; the host relays all states back as snapshots and clients interpolate remotes ~100 ms behind.
//! Events (shots, grenade throws, item sounds, damage, kills) ride a tiny reliable channel (resend until acked, in order) inside the same packets.
//! Start: env BODYCAM_HOST=port, BODYCAM_JOIN=ip[:port], or the tablet PLAY > HOST SERVERS screen (`Connect`).
// ponytail: no NAT traversal (forward the UDP port), no lag compensation beyond a fixed 100 ms rewind, host trusts client positions,
// grenade blast damage on clients is applied by the victim's own simulation, a hit is a capsule test + one world ray (no penetration).
use crate::items::{KNIFE_DAMAGE, KNIFE_RADIUS, KNIFE_RANGE};
use crate::audio::{PlayCue, RemoteSfx};
use crate::player::{IgnoreTraces, Player};
use crate::status::{PlayerDamage, SurvivorStatus};
use crate::weapon::{Fired, Health, Inventory, MontageStarted, RemoteShot, Stats};
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::net::{SocketAddr, UdpSocket};

const MAGIC: u32 = 0xB0D7_CA11;
pub const PORT: u16 = 7777;
const SEND_DT: f32 = 1.0 / 30.0;
const INTERP: f64 = 0.1;
const TIMEOUT: f32 = 6.0;
/// DA_GameModeDeathmatch: TeamConfig.MaxPlayers 7, ScoringConfig.ScoreLimit 40 (a kill = 1 point).
const MAX_PLAYERS: u8 = 7;
const SCORE_LIMIT: u32 = 40;

/// Deathmatch table for the tablet scoreboard: (player id, kills, deaths), best first; my id.
pub static SCORES: std::sync::Mutex<Vec<(u8, u32, u32)>> = std::sync::Mutex::new(Vec::new());
pub static MY_ID: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
const FIRE_RANGE: f32 = 5000.0;
const KILL_CREDIT_S: f32 = 10.0;

/// Open a session from the tablet (or env at startup).
#[derive(Message)]
pub struct Connect {
    pub host: bool,
    pub addr: String,
}

/// Everything a remote player needs to be drawn (the wire form of one player).
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct NetState {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub vel: [f32; 3],
    pub crouch: bool,
    pub lean: f32,
    pub aiming: bool,
    pub dead: bool,
    /// Equipped item row + mounted attachment ids.
    pub item: String,
    pub atts: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Ev {
    /// A shot: muzzle origin + one direction per pellet.
    Fire { row: String, supp: bool, origin: [f32; 3], dirs: Vec<[f32; 3]> },
    /// Reload / mag check / draw ... : the montage's sound notifies (seconds from now, cue).
    Sounds(Vec<(f32, String)>),
    Grenade { start: [f32; 3], vel: [f32; 3], flash: bool },
    /// Knife swing (host resolves the hit).
    Melee { origin: [f32; 3], dir: [f32; 3] },
    /// Drone crash blast.
    Blast { at: [f32; 3] },
    /// Host -> victim: apply damage (from = attacker).
    Damage(f32),
    Kill { killer: u8, victim: u8 },
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub enum Body {
    Join,
    Welcome { id: u8, map: String },
    State(NetState),
    Snap(Vec<(u8, NetState)>),
    Bye,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Pkt {
    pub magic: u32,
    pub from: u8,
    /// Highest in-order event id received from the peer.
    pub ack: u32,
    /// (event id, originating player, event) not yet acked.
    pub evs: Vec<(u32, u8, Ev)>,
    /// Lag compensation clock: sender time, echo of the peer's last `ts`, sender's measured round trip.
    pub ts: f32,
    pub echo: f32,
    pub rtt: f32,
    pub body: Body,
}

pub fn encode(p: &Pkt) -> Vec<u8> {
    bincode::serialize(p).unwrap_or_default()
}
pub fn decode(b: &[u8]) -> Option<Pkt> {
    bincode::deserialize::<Pkt>(b).ok().filter(|p| p.magic == MAGIC)
}

/// Reliable in-order event channel to one peer.
#[derive(Default)]
struct Link {
    out: Vec<(u32, u8, Ev)>,
    next: u32,
    got: u32,
}
impl Link {
    fn push(&mut self, from: u8, ev: Ev) {
        self.next += 1;
        self.out.push((self.next, from, ev));
        if self.out.len() > 256 {
            self.out.remove(0);
        }
    }
    fn pkt(&self, from: u8, body: Body) -> Pkt {
        Pkt { magic: MAGIC, from, ack: self.got, evs: self.out.clone(), ts: 0.0, echo: 0.0, rtt: 0.0, body }
    }
    /// Drops acked events; returns the newly received in-order ones.
    fn recv(&mut self, p: &Pkt) -> Vec<(u8, Ev)> {
        self.out.retain(|e| e.0 > p.ack);
        let mut v = vec![];
        for (id, from, ev) in &p.evs {
            if *id == self.got + 1 {
                self.got = *id;
                v.push((*from, ev.clone()));
            }
        }
        v
    }
}

/// One other player. Host: a connected client (addr + link). Client: anyone from the snapshots.
struct Other {
    addr: Option<SocketAddr>,
    link: Link,
    seen: f32,
    buf: VecDeque<(f64, NetState)>,
    hp: f32,
    last_by: Option<(u8, f32)>,
    dead: bool,
    ent: Option<Entity>,
    /// Host: peer's last `ts` (echoed back) and its reported round trip.
    last_ts: f32,
    rtt: f32,
}
impl Other {
    fn new(addr: Option<SocketAddr>, now: f32) -> Self {
        Other { addr, link: Link::default(), seen: now, buf: VecDeque::new(), hp: 100.0, last_by: None, dead: false, ent: None, last_ts: 0.0, rtt: 0.0 }
    }
}

#[derive(Resource)]
pub struct Net {
    sock: UdpSocket,
    host: bool,
    id: u8,
    map: String,
    /// Client: the host.
    up: Option<(SocketAddr, Link)>,
    joined: bool,
    join_t: f32,
    others: HashMap<u8, Other>,
    since_send: f32,
    gone: Vec<Entity>,
    stats: HashMap<String, Stats>,
    snd: Vec<(f32, String, u8)>,
    host_dead: bool,
    host_last_by: Option<(u8, f32)>,
    /// Client: measured round trip. Host: own recent positions (time, feet, crouch) for lag compensation.
    rtt: f32,
    hist: VecDeque<(f64, Vec3, bool)>,
}

/// A packet's events, delivered to `apply` (also the host's own events, from = 0).
#[derive(Message)]
struct NetEv {
    from: u8,
    ev: Ev,
}

/// Interpolated state of a remote player: what a third-person character needs.
#[derive(Component, Default, Clone)]
pub struct RemoteState {
    pub id: u8,
    pub pos: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub vel: Vec3,
    pub crouch: bool,
    pub lean: f32,
    pub aiming: bool,
    pub dead: bool,
    pub item: String,
    pub atts: Vec<String>,
}

/// Damageable collision shapes of a remote player (excluded from the host's world occlusion ray).
#[derive(Component)]
struct RemoteBody;

pub struct NetPlugin;
impl Plugin for NetPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<Connect>().add_message::<NetEv>().add_message::<RemoteSfx>().add_systems(Startup, env_start).add_systems(
            Update,
            (connect, (recv, local_events, apply, blast_damage, tick, sync_remotes, drive_remotes).chain().run_if(resource_exists::<Net>)).chain(),
        );
    }
}

fn map_name() -> &'static str {
    if crate::testmap::wanted() {
        "test"
    } else if crate::grid::wanted() {
        "grid"
    } else {
        ""
    }
}

fn env_start(mut out: MessageWriter<Connect>) {
    if let Ok(p) = std::env::var("BODYCAM_HOST") {
        out.write(Connect { host: true, addr: p });
    } else if let Ok(a) = std::env::var("BODYCAM_JOIN") {
        out.write(Connect { host: false, addr: a });
    }
}

fn connect(mut cmd: Commands, mut ev: MessageReader<Connect>, net: Option<Res<Net>>) {
    let Some(c) = ev.read().last() else { return };
    if net.is_some() {
        return; // one session per run
    }
    let made = if c.host {
        let port = c.addr.trim().parse::<u16>().unwrap_or(PORT);
        UdpSocket::bind(("0.0.0.0", port)).map(|s| (s, None)).map_err(|e| format!("bind {port}: {e}"))
    } else {
        let a = c.addr.trim();
        let a = if a.contains(':') { a.to_string() } else { format!("{a}:{PORT}") };
        match a.parse::<SocketAddr>() {
            Ok(addr) => UdpSocket::bind("0.0.0.0:0").map(|s| (s, Some(addr))).map_err(|e| e.to_string()),
            Err(e) => Err(format!("bad address {a}: {e}")),
        }
    };
    match made {
        Ok((sock, up)) => {
            let _ = sock.set_nonblocking(true);
            info!("net: {} {}", if c.host { "hosting on" } else { "joining" }, sock.local_addr().map(|a| a.to_string()).unwrap_or_default());
            cmd.insert_resource(Net {
                sock,
                host: c.host,
                id: 0,
                map: map_name().into(),
                up: up.map(|a| (a, Link::default())),
                joined: c.host,
                join_t: 0.0,
                others: HashMap::new(),
                since_send: 0.0,
                gone: vec![],
                stats: crate::weapon::load_stats(),
                snd: vec![],
                host_dead: false,
                host_last_by: None,
                rtt: 0.0,
                hist: VecDeque::new(),
            });
        }
        Err(e) => warn!("net: {e}"),
    }
}

fn relaunch(map: &str, join: &str) -> ! {
    let mut c = std::process::Command::new(std::env::current_exe().unwrap_or_default());
    if map.is_empty() { c.env_remove("BODYCAM_MAP"); } else { c.env("BODYCAM_MAP", map); }
    c.env("BODYCAM_JOIN", join).env("BODYCAM_RELAUNCHED", "1").env_remove("BODYCAM_HOST").env_remove("BODYCAM_AUTO");
    let _ = c.spawn();
    std::process::exit(0)
}

fn send(net: &Net, to: SocketAddr, p: &Pkt) {
    let _ = net.sock.send_to(&encode(p), to);
}

fn recv(mut net: ResMut<Net>, time: Res<Time>, mut out: MessageWriter<NetEv>) {
    let now = time.elapsed_secs();
    let tnow = time.elapsed_secs_f64();
    let mut buf = [0u8; 2048];
    let net = &mut *net;
    loop {
        let (n, addr) = match net.sock.recv_from(&mut buf) {
            Ok(v) => v,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(_) => continue, // Windows reports ICMP port-unreachable as a recv error
        };
        let Some(p) = decode(&buf[..n]) else { continue };
        if net.host {
            let id = match &p.body {
                Body::Join => {
                    let known = net.others.iter().find(|(_, o)| o.addr == Some(addr)).map(|(i, _)| *i);
                    let id = known.or_else(|| (1..MAX_PLAYERS).find(|i| !net.others.contains_key(i)));
                    let Some(id) = id else {
                        send(net, addr, &Pkt { magic: MAGIC, from: 0, ack: 0, evs: vec![], ts: 0.0, echo: 0.0, rtt: 0.0, body: Body::Bye });
                        continue;
                    };
                    if known.is_none() {
                        info!("net: player {id} joined from {addr}");
                        net.others.insert(id, Other::new(Some(addr), now));
                    }
                    send(net, addr, &Pkt { magic: MAGIC, from: 0, ack: 0, evs: vec![], ts: 0.0, echo: 0.0, rtt: 0.0, body: Body::Welcome { id, map: net.map.clone() } });
                    continue;
                }
                _ => p.from,
            };
            let Some(o) = net.others.get_mut(&id).filter(|o| o.addr == Some(addr)) else { continue };
            o.seen = now;
            (o.last_ts, o.rtt) = (p.ts, p.rtt.clamp(0.0, 1.0));
            let evs = o.link.recv(&p);
            match p.body {
                Body::State(s) => {
                    if s.dead && !o.dead {
                        o.hp = 0.0;
                    } else if !s.dead && o.dead {
                        o.hp = 100.0;
                        o.last_by = None;
                    }
                    o.dead = s.dead;
                    o.buf.push_back((tnow, s));
                    while o.buf.len() > 40 {
                        o.buf.pop_front();
                    }
                }
                Body::Bye => {
                    info!("net: player {id} left");
                    if let Some(o) = net.others.remove(&id) {
                        net.gone.extend(o.ent);
                    }
                    continue;
                }
                _ => {}
            }
            for (from, ev) in evs {
                if matches!(ev, Ev::Damage(_) | Ev::Kill { .. }) {
                    continue; // only the host decides those
                }
                for (i, o) in net.others.iter_mut() {
                    if *i != id {
                        o.link.push(from, ev.clone());
                    }
                }
                out.write(NetEv { from: id, ev });
            }
        } else {
            if net.up.as_ref().is_none_or(|(a, _)| *a != addr) {
                continue;
            }
            match &p.body {
                Body::Welcome { id, map } => {
                    if !net.joined {
                        net.joined = true;
                        net.id = *id;
                        MY_ID.store(*id, std::sync::atomic::Ordering::Relaxed);
                        net.map = map.clone();
                        info!("net: joined as player {id} (map '{map}')");
                        if map != map_name() && std::env::var("BODYCAM_RELAUNCHED").is_err() {
                            relaunch(map, &addr.to_string());
                        }
                    }
                }
                Body::Bye => {
                    warn!("net: host closed / full");
                    net.joined = false;
                    net.join_t = -1.0;
                }
                Body::Snap(players) => {
                    for (id, s) in players {
                        let o = net.others.entry(*id).or_insert_with(|| Other::new(None, now));
                        o.seen = now;
                        o.buf.push_back((tnow, s.clone()));
                        while o.buf.len() > 40 {
                            o.buf.pop_front();
                        }
                    }
                    let alive: Vec<u8> = players.iter().map(|p| p.0).collect();
                    let gone: Vec<u8> = net.others.keys().filter(|k| !alive.contains(k)).copied().collect();
                    for g in gone {
                        if let Some(o) = net.others.remove(&g) {
                            net.gone.extend(o.ent);
                        }
                    }
                }
                _ => {}
            }
            if p.echo > 0.0 {
                net.rtt = (now - p.echo).clamp(0.0, 1.0);
            }
            let evs = net.up.as_mut().map(|(_, l)| l.recv(&p)).unwrap_or_default();
            for (from, ev) in evs {
                out.write(NetEv { from, ev });
            }
        }
    }
}

/// Local gameplay events -> the wire (and, on the host, into `apply` for hit resolution).
fn local_events(
    mut net: ResMut<Net>,
    mut fired: MessageReader<Fired>,
    mut mont: MessageReader<MontageStarted>,
    mut thrown: MessageReader<crate::items::Thrown>,
    mut melee: MessageReader<crate::items::Melee>,
    mut crashed: MessageReader<crate::items::DroneCrashed>,
    mut out: MessageWriter<NetEv>,
) {
    let net = &mut *net;
    let id = net.id;
    let mut evs = vec![];
    for f in fired.read() {
        evs.push(Ev::Fire { row: f.row.clone(), supp: f.suppressed, origin: f.origin.to_array(), dirs: f.dirs.iter().map(|d| d.to_array()).collect() });
    }
    for m in mont.read() {
        use crate::weapon::ActionKind::*;
        if matches!(m.kind, CameraAdjust | FireOut) {
            continue;
        }
        let s: Vec<(f32, String)> = m.montage.notifies.iter().filter(|n| n.1 == "PlayWepSounds_C").filter_map(|n| Some((n.0, n.2["Sound"].as_str()?.split('.').next()?.to_string()))).collect();
        if !s.is_empty() {
            evs.push(Ev::Sounds(s));
        }
    }
    for t in thrown.read() {
        evs.push(Ev::Grenade { start: t.start.to_array(), vel: t.vel.to_array(), flash: t.flash });
    }
    for m in melee.read() {
        evs.push(Ev::Melee { origin: m.origin.to_array(), dir: m.dir.to_array() });
    }
    for c in crashed.read() {
        evs.push(Ev::Blast { at: c.at.to_array() });
    }
    for ev in evs {
        if net.host {
            for o in net.others.values_mut() {
                o.link.push(id, ev.clone());
            }
            if matches!(ev, Ev::Fire { .. } | Ev::Melee { .. }) {
                out.write(NetEv { from: id, ev });
            }
        } else if let Some((_, l)) = net.up.as_mut() {
            l.push(id, ev);
        }
    }
}

/// Capsule (feet, height h, radius r) vs ray: distance along the ray and the hit height above the feet.
fn ray_capsule(o: Vec3, d: Vec3, feet: Vec3, h: f32, r: f32) -> Option<(f32, f32)> {
    let (a, b) = (feet + Vec3::Y * r, feet + Vec3::Y * (h - r));
    let (u, w) = (b - a, o - a);
    let (uu, ud, wu, wd) = (u.dot(u), u.dot(d), w.dot(u), w.dot(d));
    let den = uu - ud * ud;
    let mut s = if den > 1e-6 { ((wu - wd * ud) / den).clamp(0.0, 1.0) } else { 0.0 };
    let t = (s * ud - wd).max(0.0);
    s = ((w + d * t).dot(u) / uu.max(1e-6)).clamp(0.0, 1.0);
    let dist = (o + d * t - (a + u * s)).length();
    (dist <= r).then(|| {
        let th = (t - (r * r - dist * dist).sqrt()).max(0.0);
        (th, (o + d * th).y - feet.y)
    })
}

/// Sample a state buffer at time `t` (interpolated, short extrapolation with the velocity past the newest sample).
fn sample(buf: &VecDeque<(f64, NetState)>, t: f64) -> Option<NetState> {
    let (t0, first) = buf.front()?;
    if t <= *t0 {
        return Some(first.clone());
    }
    for w in buf.iter().collect::<Vec<_>>().windows(2) {
        let ((ta, a), (tb, b)) = (w[0], w[1]);
        if t >= *ta && t <= *tb {
            let k = ((t - ta) / (tb - ta).max(1e-6)) as f32;
            let l = |x: [f32; 3], y: [f32; 3]| Vec3::from(x).lerp(Vec3::from(y), k).to_array();
            let dy = (b.yaw - a.yaw + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
            return Some(NetState { pos: l(a.pos, b.pos), vel: l(a.vel, b.vel), yaw: a.yaw + dy * k, pitch: a.pitch + (b.pitch - a.pitch) * k, lean: a.lean + (b.lean - a.lean) * k, ..b.clone() });
        }
    }
    let (tl, last) = buf.back()?;
    let ex = ((t - tl) as f32).min(0.15);
    Some(NetState { pos: (Vec3::from(last.pos) + Vec3::from(last.vel) * ex).to_array(), ..last.clone() })
}

#[allow(clippy::too_many_arguments)]
fn apply(
    mut cmd: Commands,
    mut net: ResMut<Net>,
    time: Res<Time>,
    mut ev: MessageReader<NetEv>,
    assets: Res<AssetServer>,
    mut shots: MessageWriter<RemoteShot>,
    mut sfx: MessageWriter<RemoteSfx>,
    mut dmg: MessageWriter<PlayerDamage>,
    mut blast: MessageWriter<crate::items::DetonateFor>,
    mut st: ResMut<SurvivorStatus>,
    me: Single<(&Transform, &Player)>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    body: Query<(), With<RemoteBody>>,
    mut chars: Query<&mut crate::character::CharacterState>,
) {
    let now = time.elapsed_secs();
    let tnow = time.elapsed_secs_f64();
    let net = &mut *net;
    let my_pos = me.0.translation;
    let my_id = net.id;
    for NetEv { from, ev } in ev.read() {
        let from_pos = |net: &Net| if *from == my_id { my_pos } else { net.others.get(from).and_then(|o| o.buf.back()).map_or(Vec3::ZERO, |s| Vec3::from(s.1.pos)) };
        match ev {
            Ev::Fire { row, supp, origin, dirs } => {
                let origin = Vec3::from(*origin);
                let dirs: Vec<Vec3> = dirs.iter().map(|d| Vec3::from(*d)).collect();
                if let Some(mut c) = net.others.get(from).and_then(|o| o.ent).and_then(|e| chars.get_mut(e).ok()) {
                    c.fired += 1; // 3P muzzle flash + kick
                }
                if *from != my_id {
                    let d_cm = (my_pos + Vec3::Y * 1.6).distance(origin) * 100.0;
                    sfx.write(RemoteSfx { row: row.clone(), kind: if *supp { "FireSuppressed" } else { "Fire" }, dist_cm: d_cm });
                    shots.write(RemoteShot { origin, dirs: dirs.clone(), suppressed: *supp });
                }
                if !net.host || (*from != my_id && origin.distance(from_pos(net) + Vec3::Y * 1.6) > 4.0) {
                    continue;
                }
                let dmg_base = net.stats.get(row).and_then(|s| s.damage).unwrap_or(30.0);
                let targets = targets_at(net, *from, tnow, my_pos, me.1.crouched, st.health > 0.0);
                for dir in &dirs {
                    let Ok(d3) = Dir3::new(*dir) else { continue };
                    let filter = |e: Entity| !ignore.contains(e) && !body.contains(e);
                    let wall = ray.cast_ray(Ray3d::new(origin, d3), &MeshRayCastSettings::default().with_filter(&filter)).first().map_or(FIRE_RANGE, |(_, h)| h.distance);
                    let best = targets
                        .iter()
                        .filter_map(|(i, feet, crouch)| {
                            let h = if *crouch { 1.15 } else { 1.8 };
                            ray_capsule(origin, *dir, *feet, h, 0.35).filter(|(t, _)| *t < wall && *t <= FIRE_RANGE).map(|(t, y)| (*i, t, y / h))
                        })
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    let Some((victim, _, frac)) = best else { continue };
                    let bone = if frac < 0.5 { "thigh" } else if frac < 0.72 { "spine_01" } else if frac < 0.85 { "spine_02" } else { "head" };
                    let amount = dmg_base * crate::weapon::bone_multiplier(bone);
                    deal(net, &mut dmg, *from, victim, amount, now, bone);
                }
            }
            Ev::Melee { origin, dir } => {
                if net.host {
                    let (o, d) = (Vec3::from(*origin), Vec3::from(*dir));
                    let targets = targets_at(net, *from, tnow, my_pos, me.1.crouched, st.health > 0.0);
                    let best = targets
                        .iter()
                        .filter_map(|(i, feet, crouch)| {
                            let h = if *crouch { 1.15 } else { 1.8 };
                            let mid = *feet + Vec3::Y * h * 0.6;
                            let v = mid - o;
                            let along = v.dot(d);
                            let perp = (v - d * along.clamp(0.0, KNIFE_RANGE)).length();
                            (along > -KNIFE_RADIUS && perp <= KNIFE_RADIUS + 0.3).then_some((*i, along))
                        })
                        .min_by(|a, b| a.1.total_cmp(&b.1));
                    let wall = Dir3::new(d).ok().and_then(|dir| ray.cast_ray(Ray3d::new(o, dir), &MeshRayCastSettings::default().with_filter(&|e: Entity| !ignore.contains(e) && !body.contains(e))).first().map(|(_, h)| h.distance));
                    if let Some((victim, along)) = best && wall.is_none_or(|w| w > along.max(0.0) - 0.05) {
                        deal(net, &mut dmg, *from, victim, KNIFE_DAMAGE, now, "melee");
                    }
                }
            }
            Ev::Blast { at } => {
                info!("net: player {from} drone blast");
                if *from != my_id {
                    blast.write(crate::items::DetonateFor { at: Vec3::from(*at), spec: crate::items::DRONE_BLAST, owner: *from });
                }
            }
            Ev::Sounds(list) => {
                for (t, cue) in list {
                    net.snd.push((*t, cue.clone(), *from));
                }
            }
            Ev::Grenade { start, vel, flash } => {
                info!("net: player {from} threw a grenade");
                if *from != my_id {
                    crate::items::spawn_grenade(&mut cmd, &assets, Vec3::from(*start), Vec3::from(*vel), *flash, *from);
                }
            }
            Ev::Damage(a) => {
                dmg.write(PlayerDamage(*a));
            }
            Ev::Kill { killer, victim } => {
                info!("kill feed: player {killer} killed player {victim}");
                if let Ok(mut t) = SCORES.lock() {
                    let mut bump = |id: u8, k: u32, d: u32| match t.iter_mut().find(|r| r.0 == id) {
                        Some(r) => (r.1, r.2) = (r.1 + k, r.2 + d),
                        None => t.push((id, k, d)),
                    };
                    if *killer != 255 && killer != victim {
                        bump(*killer, 1, 0);
                    }
                    bump(*victim, 0, 1);
                    t.sort_by_key(|r| std::cmp::Reverse(r.1));
                    if t.first().is_some_and(|r| r.1 >= SCORE_LIMIT) {
                        info!("match over: player {} reached the score limit {SCORE_LIMIT}", t[0].0);
                        t.clear(); // next match
                    }
                }
                if *victim == my_id {
                    st.health = st.health.min(0.0);
                }
            }
        }
    }
}

/// Host: radial blast damage for every player (host included) with line of sight, credited to the blast's owner.
fn blast_damage(
    mut net: ResMut<Net>,
    time: Res<Time>,
    mut ev: MessageReader<crate::items::BlastDamage>,
    mut dmg: MessageWriter<PlayerDamage>,
    st: Res<SurvivorStatus>,
    me: Single<&Transform, With<Player>>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    body: Query<(), With<RemoteBody>>,
) {
    let net = &mut *net;
    let now = time.elapsed_secs();
    for b in ev.read() {
        if !net.host {
            continue;
        }
        let crate::items::BlastSpec { base, inner, outer, exp, .. } = b.spec;
        let mut victims: Vec<(u8, Vec3)> = net.others.iter().filter(|(_, o)| !o.dead).filter_map(|(i, o)| Some((*i, Vec3::from(o.buf.back()?.1.pos)))).collect();
        if st.health > 0.0 {
            victims.push((0, me.translation));
        }
        for (v, feet) in victims {
            let head = feet + Vec3::Y * 1.6;
            let d = head - b.origin;
            let f = crate::items::falloff(d.length() * 100.0, inner, outer, exp);
            let filter = |e: Entity| !ignore.contains(e) && !body.contains(e);
            let blocked = Dir3::new(d).ok().is_some_and(|dir| ray.cast_ray(Ray3d::new(b.origin, dir), &MeshRayCastSettings::default().with_filter(&filter)).first().is_some_and(|(_, h)| h.distance < d.length() - 0.15));
            if f > 0.0 && !blocked {
                deal(net, &mut dmg, b.owner, v, base * f, now, "blast");
            }
        }
    }
}

/// Everyone `from` can hit, positioned where `from` saw them: lag compensation = interpolation delay + its round trip, read from
/// the per-peer state buffers (clients) and the host's own position history.
fn targets_at(net: &Net, from: u8, tnow: f64, my_pos: Vec3, my_crouch: bool, host_alive: bool) -> Vec<(u8, Vec3, bool)> {
    let back = INTERP + if from == net.id { 0.0 } else { net.others.get(&from).map_or(0.0, |o| o.rtt) as f64 };
    let tt = tnow - back;
    let mut t: Vec<_> = net.others.iter().filter(|(i, o)| **i != from && !o.dead).filter_map(|(i, o)| Some((*i, Vec3::from(sample(&o.buf, tt)?.pos), o.buf.back()?.1.crouch))).collect();
    if from != 0 && host_alive {
        let h = net.hist.iter().rev().find(|h| h.0 <= tt).or(net.hist.front()).map_or((my_pos, my_crouch), |h| (h.1, h.2));
        t.push((0, h.0, h.1));
    }
    t
}

/// Host: apply `amount` to `victim` (the host itself or a client) and remember who hit last (kill credit).
fn deal(net: &mut Net, dmg: &mut MessageWriter<PlayerDamage>, from: u8, victim: u8, amount: f32, now: f32, what: &str) {
    info!("net: player {from} hit player {victim} ({what}) for {amount:.0}");
    if victim == 0 {
        dmg.write(PlayerDamage(amount));
        net.host_last_by = Some((from, now));
    } else if let Some(o) = net.others.get_mut(&victim) {
        o.hp -= amount;
        o.last_by = Some((from, now));
        o.link.push(from, Ev::Damage(amount));
    }
}

fn local_state(p: &Player, tf: &Transform, inv: Option<&Inventory>, cfg: Option<&crate::ui::settings::Settings>) -> NetState {
    let item = inv.map(|i| i.slots[i.current].name.clone()).unwrap_or_default();
    let atts = cfg.and_then(|c| c.attachments.get(&item).cloned()).unwrap_or_default();
    NetState { pos: tf.translation.to_array(), yaw: p.yaw, pitch: p.pitch, vel: p.vel.to_array(), crouch: p.crouched, lean: p.lean, aiming: p.aiming, dead: p.dead, item, atts }
}

#[allow(clippy::too_many_arguments)]
fn tick(
    mut net: ResMut<Net>,
    time: Res<Time>,
    me: Single<(&Player, &Transform, Option<&Inventory>)>,
    cfg: Option<Res<crate::ui::settings::Settings>>,
    st: Res<SurvivorStatus>,
    mut cues: MessageWriter<PlayCue>,
    cam: Single<&GlobalTransform, With<crate::player::BodycamCamera>>,
    mut out: MessageWriter<NetEv>,
) {
    let dt = time.delta_secs();
    let now = time.elapsed_secs();
    let net = &mut *net;
    let mine = local_state(me.0, me.1, me.2, cfg.as_deref());
    if net.host {
        let t = time.elapsed_secs_f64();
        net.hist.push_back((t, me.1.translation, me.0.crouched));
        while net.hist.front().is_some_and(|h| t - h.0 > 1.5) {
            net.hist.pop_front();
        }
    }
    // Pending remote item sounds, attenuated by distance to where that player stands.
    let snd = std::mem::take(&mut net.snd);
    for (t, cue, from) in snd {
        if t > dt {
            net.snd.push((t - dt, cue, from));
        } else {
            let at = net.others.get(&from).and_then(|o| o.buf.back()).map_or(cam.translation(), |s| Vec3::from(s.1.pos) + Vec3::Y);
            cues.write(PlayCue { cue, dist_cm: cam.translation().distance(at) * 100.0 });
        }
    }
    if net.host {
        // Time out silent clients, regen their tracked health, announce kills.
        let silent: Vec<u8> = net.others.iter().filter(|(_, o)| now - o.seen > TIMEOUT).map(|(i, _)| *i).collect();
        for i in silent {
            info!("net: player {i} timed out");
            if let Some(o) = net.others.remove(&i) {
                net.gone.extend(o.ent);
            }
        }
        let mut kills = vec![];
        for (i, o) in net.others.iter_mut() {
            if !o.dead && o.hp < 100.0 && o.hp > 0.0 {
                o.hp = (o.hp + 2.0 * dt).min(100.0);
            }
            if o.hp <= 0.0 && !o.dead {
                o.dead = true;
                kills.push((o.last_by.filter(|b| now - b.1 < KILL_CREDIT_S).map_or(255, |b| b.0), *i));
            }
        }
        if st.health <= 0.0 && !net.host_dead {
            net.host_dead = true;
            kills.push((net.host_last_by.filter(|b| now - b.1 < KILL_CREDIT_S).map_or(255, |b| b.0), 0));
        } else if st.health > 0.0 {
            net.host_dead = false;
        }
        for (killer, victim) in kills {
            for o in net.others.values_mut() {
                o.link.push(0, Ev::Kill { killer, victim });
            }
            out.write(NetEv { from: 0, ev: Ev::Kill { killer, victim } });
        }
    } else if !net.joined {
        net.join_t -= dt;
        if net.join_t <= 0.0 {
            net.join_t = 0.5;
            if let Some((a, _)) = net.up {
                send(net, a, &Pkt { magic: MAGIC, from: 0, ack: 0, evs: vec![], ts: 0.0, echo: 0.0, rtt: 0.0, body: Body::Join });
            }
        }
        return;
    } else if now - net.others.get(&0).map_or(now, |o| o.seen) > TIMEOUT {
        warn!("net: host timed out");
        net.joined = false;
        net.join_t = 0.0;
    }
    net.since_send += dt;
    if net.since_send < SEND_DT {
        return;
    }
    net.since_send = 0.0;
    if net.host {
        let mut all: Vec<(u8, NetState)> = vec![(0, mine)];
        all.extend(net.others.iter().filter_map(|(i, o)| Some((*i, o.buf.back()?.1.clone()))));
        for (i, o) in &net.others {
            if let Some(a) = o.addr {
                let snap = all.iter().filter(|(j, _)| j != i).cloned().collect();
                let mut p = o.link.pkt(0, Body::Snap(snap));
                (p.ts, p.echo) = (now, o.last_ts);
                send(net, a, &p);
            }
        }
    } else if let Some((a, l)) = &net.up {
        let mut p = l.pkt(net.id, Body::State(mine));
        (p.ts, p.rtt) = (now, net.rtt);
        send(net, *a, &p);
    }
}

/// THE place where wire state becomes a visual: spawn a third-person character (character.rs) + an invisible hit capsule.
fn spawn_visual(cmd: &mut Commands, meshes: &mut Assets<Mesh>, mats: &mut Assets<StandardMaterial>, id: u8) -> Entity {
    let clear = mats.add(StandardMaterial { base_color: Color::srgba(0.0, 0.0, 0.0, 0.0), alpha_mode: AlphaMode::Blend, ..default() });
    let root = crate::character::spawn_character(cmd, Transform::default(), crate::character::CharacterState::default());
    let capsule = cmd.spawn((Mesh3d(meshes.add(Capsule3d::new(0.3, 1.2))), MeshMaterial3d(clear), Transform::from_xyz(0.0, 0.9, 0.0), RemoteBody, Health(1.0e9))).id();
    cmd.entity(root).insert(RemoteState { id, ..default() }).add_child(capsule);
    root
}

fn sync_remotes(mut cmd: Commands, mut net: ResMut<Net>, mut meshes: ResMut<Assets<Mesh>>, mut mats: ResMut<Assets<StandardMaterial>>) {
    for e in net.gone.drain(..) {
        cmd.entity(e).despawn();
    }
    for (id, o) in net.others.iter_mut() {
        if o.ent.is_none() && !o.buf.is_empty() {
            o.ent = Some(spawn_visual(&mut cmd, &mut meshes, &mut mats, *id));
            info!("net: spawned remote player {id}");
        }
    }
}

/// Interpolates every remote ~100 ms behind and maps `RemoteState` onto the character (second half of the one mapping spot).
fn drive_remotes(net: Res<Net>, time: Res<Time>, mut q: Query<(&mut Transform, &mut RemoteState, &mut crate::character::CharacterState)>) {
    let t = time.elapsed_secs_f64() - INTERP;
    for o in net.others.values() {
        let Some(ent) = o.ent else { continue };
        let Some(s) = sample(&o.buf, t) else { continue };
        let Ok((mut tf, mut rs, mut cs)) = q.get_mut(ent) else { continue };
        *rs = RemoteState { id: rs.id, pos: Vec3::from(s.pos), yaw: s.yaw, pitch: s.pitch, vel: Vec3::from(s.vel), crouch: s.crouch, lean: s.lean, aiming: s.aiming, dead: s.dead, item: s.item, atts: s.atts };
        tf.translation = rs.pos;
        tf.rotation = Quat::from_rotation_y(rs.yaw);
        (cs.pitch, cs.vel, cs.crouched, cs.lean, cs.aiming, cs.dead) = (rs.pitch, rs.vel, rs.crouch, rs.lean, rs.aiming, rs.dead);
        cs.sprinting = rs.vel.xz().length() > 4.0;
        if cs.item != rs.item {
            cs.item = rs.item.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packet_roundtrip() {
        let s = NetState { pos: [1.0, 2.0, 3.0], yaw: 0.5, item: "M4A1".into(), atts: vec!["suppressor".into()], aiming: true, ..default() };
        let mut l = Link::default();
        l.push(2, Ev::Fire { row: "M4A1".into(), supp: true, origin: [0.0, 1.6, 0.0], dirs: vec![[0.0, 0.0, -1.0]] });
        l.push(2, Ev::Damage(12.5));
        let p = l.pkt(2, Body::State(s));
        assert_eq!(decode(&encode(&p)), Some(p.clone()));
        assert!(decode(&[1, 2, 3]).is_none());
        // reliable channel: in-order delivery, duplicates ignored, acks drop events
        let mut rx = Link::default();
        assert_eq!(rx.recv(&p).len(), 2);
        assert_eq!(rx.recv(&p).len(), 0);
        let ack = rx.pkt(0, Body::Bye);
        l.recv(&ack);
        assert!(l.out.is_empty());
    }

    #[test]
    fn interpolation_and_capsule() {
        let mk = |x: f32| NetState { pos: [x, 0.0, 0.0], ..default() };
        let buf: VecDeque<_> = [(1.0, mk(0.0)), (2.0, mk(10.0))].into();
        assert!((sample(&buf, 1.5).unwrap().pos[0] - 5.0).abs() < 1e-4);
        assert_eq!(sample(&buf, 0.0).unwrap().pos[0], 0.0);
        let hit = ray_capsule(Vec3::new(0.0, 1.0, 5.0), Vec3::NEG_Z, Vec3::ZERO, 1.8, 0.35).unwrap();
        assert!((hit.0 - 4.65).abs() < 0.05 && (hit.1 - 1.0).abs() < 1e-3);
        assert!(ray_capsule(Vec3::new(1.0, 1.0, 5.0), Vec3::NEG_Z, Vec3::ZERO, 1.8, 0.35).is_none());
    }
}
