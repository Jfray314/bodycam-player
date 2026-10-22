//! `BODYCAM_MAP=grid`: big test grid world with four zones (lobby, shooting range, obstacle course, CQB block) for trying
//! guns, stress, near-miss audio, damage and movement. Pure primitives, spawns instantly. Player starts in the lobby facing -Z.
//! Keys: F1 lobby, F3 range, F4 course, F5 CQB (teleport), F7 bots hostile/passive, F10 respawn bots. `BODYCAM_START=range|course|cqb`
//! teleports at launch. Collision: `Colliders` (AABB / wedge list) - the player (player.rs `movement`) and bots (bots.rs) use it.
// ponytail: collision is a flat list scanned per query (a few hundred blocks), circle-vs-rectangle push-out, no ceilings (head bump) and
// bullets ignore it (they hit meshes). Swap for a grid/BVH or a physics crate if the block count passes ~2000.
use crate::bots::{self, BotSpec};
use crate::player::Player;
use crate::weapon::Health;
use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

const STEP: f32 = 0.5; // max step-up height (m)
const RADIUS: f32 = 0.3; // player collision radius (m)
const T: f32 = 0.3; // wall thickness
const RESPAWN: f32 = 4.0; // s until a shot-down range target stands again
/// (name, spawn, yaw): F1 / F3 / F4 / F5.
const ZONES: [(&str, Vec3, f32); 4] = [("lobby", Vec3::new(0.0, 0.0, 3.0), 0.0), ("range", Vec3::new(-38.0, 0.0, -13.0), 0.0), ("course", Vec3::new(40.0, 0.0, -13.0), 0.0), ("cqb", Vec3::new(0.0, 0.0, -42.0), 0.0)];
const ZONE_KEYS: [KeyCode; 4] = [KeyCode::F1, KeyCode::F3, KeyCode::F4, KeyCode::F5];

pub fn wanted() -> bool {
    crate::level::level()[0] == "grid"
}

/// Solid footprint standing on the ground; the top height is h0 at the min edge and h1 at the max edge along x or z (equal = flat box).
pub struct Block {
    min: Vec2,
    max: Vec2,
    h0: f32,
    h1: f32,
    along_z: bool,
}

impl Block {
    fn top(&self, q: Vec2) -> f32 {
        if self.h0 == self.h1 {
            return self.h0;
        }
        let (a, lo, hi) = if self.along_z { (q.y, self.min.y, self.max.y) } else { (q.x, self.min.x, self.max.x) };
        self.h0 + (self.h1 - self.h0) * ((a - lo) / (hi - lo)).clamp(0.0, 1.0)
    }
}

#[derive(Resource, Default)]
pub struct Colliders(pub Vec<Block>);

impl Colliders {
    /// Horizontal push-out of a circle at `pos` from every block taller than `ref_y` + STEP; cancels `vel` into the wall.
    pub fn push_out(&self, pos: &mut Vec3, r: f32, ref_y: f32, mut vel: Option<&mut Vec3>) {
        for _ in 0..3 {
            for b in &self.0 {
                let p = Vec2::new(pos.x, pos.z);
                let q = p.clamp(b.min, b.max);
                if b.top(q) <= ref_y + STEP + 1e-3 {
                    continue;
                }
                let d = p - q;
                let l2 = d.length_squared();
                if l2 >= r * r {
                    continue;
                }
                let (n, push) = if l2 > 1e-8 {
                    let l = l2.sqrt();
                    (d / l, r - l)
                } else {
                    // Centre inside the block: leave through the nearest face.
                    let faces = [(p.x - b.min.x, Vec2::NEG_X), (b.max.x - p.x, Vec2::X), (p.y - b.min.y, Vec2::NEG_Y), (b.max.y - p.y, Vec2::Y)];
                    let (dist, n) = faces.into_iter().min_by(|a, b| a.0.total_cmp(&b.0)).unwrap();
                    (n, dist + r)
                };
                pos.x += n.x * push;
                pos.z += n.y * push;
                if let Some(v) = vel.as_deref_mut() {
                    let vn = v.x * n.x + v.z * n.y;
                    if vn < 0.0 {
                        v.x -= vn * n.x;
                        v.z -= vn * n.y;
                    }
                }
            }
        }
    }

    /// Highest walkable surface under the point (blocks no taller than `ref_y` + STEP), 0 = floor.
    pub fn ground(&self, p: Vec2, ref_y: f32) -> f32 {
        self.0.iter().filter(|b| p.cmpge(b.min).all() && p.cmple(b.max).all()).map(|b| b.top(p)).filter(|&t| t <= ref_y + STEP + 1e-3).fold(0.0, f32::max)
    }

    /// Would a circle of radius `r` at `p` overlap a block taller than `ref_y` + STEP (what `push_out` would push out of)?
    pub fn blocked(&self, p: Vec2, r: f32, ref_y: f32) -> bool {
        self.0.iter().any(|b| {
            let q = p.clamp(b.min, b.max);
            (p - q).length_squared() < r * r && b.top(q) > ref_y + STEP + 1e-3
        })
    }

    /// Grid A* (0.5 m cells, 8 neighbours, no corner cutting) from `from` (standing at `from_y`) to `to`, as waypoints excluding the start.
    /// Walks ramps/stairs: a step is free while the next cell's blocks are no taller than the current height + STEP, and the height follows `ground`.
    /// Empty = no route within the search budget; an unreachable goal gives the path to the closest cell reached.
    /// ponytail: one-shot grid over the block list (no cache), capped at 6000 expansions; big maps want a prebuilt navmesh.
    pub fn path(&self, from: Vec2, from_y: f32, to: Vec2, r: f32) -> Vec<Vec2> {
        use std::cmp::Reverse;
        use std::collections::{BinaryHeap, HashMap};
        const CELL: f32 = 0.5;
        let cell = |p: Vec2| ((p.x / CELL).round() as i32, (p.y / CELL).round() as i32);
        let centre = |c: (i32, i32)| Vec2::new(c.0 as f32 * CELL, c.1 as f32 * CELL);
        let (start, goal) = (cell(from), cell(to));
        let h = |c: (i32, i32)| ((c.0 - goal.0).abs().max((c.1 - goal.1).abs()) as f32 + 0.414 * (c.0 - goal.0).abs().min((c.1 - goal.1).abs()) as f32) * CELL;
        let key = |f: f32| (f * 1000.0) as u32;
        let mut open = BinaryHeap::new();
        let mut best: HashMap<(i32, i32), (f32, f32, (i32, i32))> = HashMap::new(); // cell -> (g, height, parent)
        best.insert(start, (0.0, from_y, start));
        open.push(Reverse((key(h(start)), start)));
        let (mut closest, mut closest_h) = (start, h(start));
        let mut expanded = 0;
        while let Some(Reverse((_, c))) = open.pop() {
            if c == goal {
                closest = c;
                break;
            }
            expanded += 1;
            if expanded > 6000 {
                break;
            }
            let (g, y, _) = best[&c];
            if h(c) < closest_h {
                (closest, closest_h) = (c, h(c));
            }
            for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1), (1, 1), (1, -1), (-1, 1), (-1, -1)] {
                let n = (c.0 + dx, c.1 + dz);
                let p = centre(n);
                if self.blocked(p, r, y) || (dx != 0 && dz != 0 && (self.blocked(centre((c.0 + dx, c.1)), r, y) || self.blocked(centre((c.0, c.1 + dz)), r, y))) {
                    continue;
                }
                let ng = g + if dx != 0 && dz != 0 { 1.414 * CELL } else { CELL };
                if best.get(&n).is_some_and(|b| b.0 <= ng) {
                    continue;
                }
                best.insert(n, (ng, self.ground(p, y), c));
                open.push(Reverse((key(ng + h(n)), n)));
            }
        }
        let mut out = vec![];
        let mut c = closest;
        while c != start {
            out.push(centre(c));
            c = best[&c].2;
        }
        out.reverse();
        // The exact goal replaces the last cell centre when that cell is the goal cell.
        if closest == goal && let Some(l) = out.last_mut() {
            *l = to;
        }
        out
    }

    /// Player step: walls push out, then snap to the ground (falling lands on it, a grounded player sticks down 0.25 m for stairs/ramps).
    pub fn collide(&self, old_y: f32, pos: &mut Vec3, vel: &mut Vec3, grounded: bool) -> bool {
        let ref_y = pos.y.max(old_y);
        self.push_out(pos, RADIUS, ref_y, Some(vel));
        let g = self.ground(Vec2::new(pos.x, pos.z), ref_y);
        let on = vel.y <= 0.0 && pos.y - g <= if grounded { 0.25 } else { 0.0 } + 1e-3;
        if on {
            pos.y = g;
        }
        on
    }
}

// ---- builder ----------------------------------------------------------------------------------------------------------------------

struct B<'a, 'w, 's> {
    cmd: &'a mut Commands<'w, 's>,
    meshes: &'a mut Assets<Mesh>,
    mats: &'a mut Assets<StandardMaterial>,
    images: &'a mut Assets<Image>,
    col: Vec<Block>,
}

type Mat = Handle<StandardMaterial>;
/// Wall opening: (lo, hi, sill, head) along the wall. Door = sill 0; window = sill 0.9; above `head` is a visual lintel.
type Opening = (f32, f32, f32, f32);
const DOOR: fn(f32, f32) -> Opening = |lo, hi| (lo, hi, 0.0, 2.2);
const WINDOW: fn(f32, f32) -> Opening = |lo, hi| (lo, hi, 0.9, 2.1);

impl B<'_, '_, '_> {
    fn mat(&mut self, r: f32, g: f32, b: f32) -> Mat {
        self.mats.add(StandardMaterial { base_color: Color::srgb(r, g, b), perceptual_roughness: 0.9, ..default() })
    }

    /// Box centred at `c`; `solid` adds a collider topped at its upper face (it must stand on the ground).
    fn bx(&mut self, c: Vec3, size: Vec3, m: &Mat, solid: bool) -> Entity {
        if solid {
            let (min, max) = (Vec2::new(c.x - size.x / 2.0, c.z - size.z / 2.0), Vec2::new(c.x + size.x / 2.0, c.z + size.z / 2.0));
            let h = c.y + size.y / 2.0;
            self.col.push(Block { min, max, h0: h, h1: h, along_z: false });
        }
        self.cmd.spawn((Mesh3d(self.meshes.add(Cuboid::from_size(size))), MeshMaterial3d(m.clone()), Transform::from_translation(c))).id()
    }

    /// Wall along x (`axis_x`, at z = `at`) or along z (at x = `at`) from `a` to `b`, `h` tall, with openings.
    fn wall(&mut self, axis_x: bool, at: f32, a: f32, b: f32, h: f32, ops: &[Opening], m: &Mat) {
        let seg = |s: &mut Self, lo: f32, hi: f32, y0: f32, y1: f32, solid: bool| {
            if hi - lo < 1e-3 || y1 - y0 < 1e-3 {
                return;
            }
            let (mid, len, y) = ((lo + hi) / 2.0, hi - lo, (y0 + y1) / 2.0);
            let (c, size) = if axis_x { (Vec3::new(mid, y, at), Vec3::new(len, y1 - y0, T)) } else { (Vec3::new(at, y, mid), Vec3::new(T, y1 - y0, len)) };
            s.bx(c, size, m, solid && y0 == 0.0);
        };
        let mut ops = ops.to_vec();
        ops.sort_by(|x, y| x.0.total_cmp(&y.0));
        let mut x = a;
        for (lo, hi, sill, head) in ops {
            seg(self, x, lo, 0.0, h, true);
            seg(self, lo, hi, 0.0, sill, true);
            seg(self, lo, hi, head, h, false);
            x = hi;
        }
        seg(self, x, b, 0.0, h, true);
    }

    /// Sloped solid: footprint min..max, top height h0 at the min edge and h1 at the max edge along z (`along_z`) or x.
    fn wedge(&mut self, min: Vec2, max: Vec2, h0: f32, h1: f32, along_z: bool, m: &Mat) {
        let h = |x: f32, z: f32| if along_z { if z == min.y { h0 } else { h1 } } else if x == min.x { h0 } else { h1 };
        let (a, b, c, d) = ((min.x, min.y), (max.x, min.y), (max.x, max.y), (min.x, max.y));
        let top = |p: (f32, f32)| Vec3::new(p.0, h(p.0, p.1), p.1);
        let bot = |p: (f32, f32)| Vec3::new(p.0, 0.0, p.1);
        let mid = Vec3::new((min.x + max.x) / 2.0, h0.max(h1) / 2.0, (min.y + max.y) / 2.0);
        let (mut pos, mut nor) = (vec![], vec![]);
        let mut quad = |q: [Vec3; 4]| {
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                let mut n = (t[1] - t[0]).cross(t[2] - t[0]);
                if n.length() < 1e-6 {
                    continue;
                }
                let flip = n.dot((t[0] + t[1] + t[2]) / 3.0 - mid) < 0.0;
                n = n.normalize() * if flip { -1.0 } else { 1.0 };
                let t = if flip { [t[0], t[2], t[1]] } else { t };
                for v in t {
                    pos.push(v.to_array());
                    nor.push(n.to_array());
                }
            }
        };
        quad([top(a), top(b), top(c), top(d)]);
        quad([bot(a), bot(b), bot(c), bot(d)]);
        for (p, q) in [(a, b), (b, c), (c, d), (d, a)] {
            quad([bot(p), bot(q), top(q), top(p)]);
        }
        let uv = vec![[0.0f32, 0.0]; pos.len()];
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, pos)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, nor)
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv);
        self.cmd.spawn((Mesh3d(self.meshes.add(mesh)), MeshMaterial3d(m.clone()), Transform::default()));
        self.col.push(Block { min, max, h0, h1, along_z });
    }

    /// Flat quad at height `y` whose UVs follow world x/z (5 m per texture tile) so the grid lines up across patches.
    fn tiled(&mut self, x0: f32, z0: f32, x1: f32, z1: f32, y: f32, m: &Mat) {
        let p = [[x0, y, z0], [x1, y, z0], [x1, y, z1], [x0, y, z1]];
        let mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, p.iter().map(|v| [v[0] / 5.0, v[2] / 5.0]).collect::<Vec<_>>())
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32, 1.0, 0.0]; 4])
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, p.to_vec())
            .with_inserted_indices(Indices::U32(vec![0, 3, 2, 0, 2, 1]));
        self.cmd.spawn((Mesh3d(self.meshes.add(mesh)), MeshMaterial3d(m.clone()), Transform::default()));
    }

    /// Text panel `width` m wide centred at `c`, rotated by `rot` (identity faces +Z, readable from +Z).
    fn panel(&mut self, c: Vec3, rot: Quat, width: f32, lines: &[&str], bg: [u8; 3], fg: [u8; 3]) {
        let img = sign_image(lines, bg, fg);
        let h = width * img.height() as f32 / img.width() as f32;
        let tex = self.images.add(img);
        let m = self.mats.add(StandardMaterial { base_color_texture: Some(tex), unlit: true, ..default() });
        self.cmd.spawn((Mesh3d(self.meshes.add(Rectangle::new(width, h))), MeshMaterial3d(m), Transform::from_translation(c + rot * Vec3::Z * 0.03).with_rotation(rot)));
        let back = self.mat(0.1, 0.1, 0.12);
        self.cmd.spawn((Mesh3d(self.meshes.add(Cuboid::new(width + 0.2, h + 0.2, 0.05))), MeshMaterial3d(back), Transform::from_translation(c).with_rotation(rot)));
    }

    /// Upright sign facing +Z (`yaw` 0) on two posts, bottom at `y0`.
    fn banner(&mut self, x: f32, z: f32, y0: f32, width: f32, lines: &[&str], bg: [u8; 3]) {
        let h = width * (lines.len() * 6 + 5) as f32 / (lines.iter().map(|l| l.len()).max().unwrap_or(1) * 4 + 5) as f32;
        let post = self.mat(0.2, 0.2, 0.22);
        for s in [-1.0, 1.0] {
            self.bx(Vec3::new(x + s * (width / 2.0 + 0.1), (y0 + h) / 2.0, z - 0.05), Vec3::new(0.12, y0 + h, 0.12), &post, true);
        }
        self.panel(Vec3::new(x, y0 + h / 2.0, z), Quat::IDENTITY, width, lines, bg, [255, 255, 255]);
    }

    /// Text lying on the floor, readable when looking toward -Z.
    fn floor_label(&mut self, x: f32, z: f32, width: f32, lines: &[&str]) {
        self.panel(Vec3::new(x, 0.03, z), Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2), width, lines, [40, 40, 44], [255, 230, 80]);
    }
}

// ---- procedural textures ---------------------------------------------------------------------------------------------------------

/// 3x5 pixel font, row-major bits.
fn glyph(c: char) -> &'static str {
    match c.to_ascii_uppercase() {
        'A' => "010101111101101", 'B' => "110101110101110", 'C' => "011100100100011", 'D' => "110101101101110", 'E' => "111100110100111",
        'F' => "111100110100100", 'G' => "011100101101011", 'H' => "101101111101101", 'I' => "111010010010111", 'J' => "001001001101010",
        'K' => "101101110101101", 'L' => "100100100100111", 'M' => "101111111101101", 'N' => "110101101101101", 'O' => "010101101101010",
        'P' => "110101110100100", 'Q' => "010101101111011", 'R' => "110101110101101", 'S' => "011100010001110", 'T' => "111010010010010",
        'U' => "101101101101111", 'V' => "101101101101010", 'W' => "101101111111101", 'X' => "101101010101101", 'Y' => "101101010010010",
        'Z' => "111001010100111", '0' => "111101101101111", '1' => "010110010010111", '2' => "110001010100111", '3' => "110001010001110",
        '4' => "101101111001001", '5' => "111100110001110", '6' => "011100111101111", '7' => "111001010010010", '8' => "111101111101111",
        '9' => "111101111001110", ':' => "000010000010000", '-' => "000000111000000", '.' => "000000000000010", '/' => "001001010100100",
        '+' => "000010111010000", '<' => "001010100010001", '>' => "100010001010100", '^' => "010101000000000", '=' => "000111000111000",
        '!' => "010010010000010", ',' => "000000000010100", '(' => "010100100100010", ')' => "010001001001010",
        _ => "000000000000000",
    }
}

fn sign_image(lines: &[&str], bg: [u8; 3], fg: [u8; 3]) -> Image {
    const PAD: usize = 3;
    let cols = lines.iter().map(|l| l.chars().count()).max().unwrap_or(1);
    let (w, h) = (cols * 4 - 1 + 2 * PAD, lines.len() * 6 - 1 + 2 * PAD);
    let mut px = vec![255u8; w * h * 4];
    let mut put = |x: usize, y: usize, c: [u8; 3]| px[(y * w + x) * 4..][..3].copy_from_slice(&c);
    for y in 0..h {
        for x in 0..w {
            put(x, y, if x == 0 || y == 0 || x == w - 1 || y == h - 1 { fg } else { bg });
        }
    }
    for (row, l) in lines.iter().enumerate() {
        for (col, c) in l.chars().enumerate() {
            for (i, b) in glyph(c).bytes().enumerate() {
                if b == b'1' {
                    put(PAD + col * 4 + i % 3, PAD + row * 6 + i / 3, fg);
                }
            }
        }
    }
    let mut im = Image::new(Extent3d { width: w as u32, height: h as u32, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    im.sampler = ImageSampler::nearest();
    im
}

/// 5 m tile (1 m lines, thick line on the tile edge) with a full mip chain so the lines hold up at 50 m.
fn grid_image() -> Image {
    const N: usize = 1024;
    let mut px = vec![255u8; N * N * 4];
    let line = |i: usize| -> u8 {
        let edge = i < 6 || i >= N - 6;
        let f = ((i as f32 + 0.5) / (N as f32 / 5.0)).fract();
        if edge { 2 } else if !(0.0075..=0.9925).contains(&f) { 1 } else { 0 }
    };
    for y in 0..N {
        for x in 0..N {
            let c = match line(x).max(line(y)) { 2 => 8, 1 => 34, _ => 84 };
            px[(y * N + x) * 4..][..3].copy_from_slice(&[c, c, c + 2]);
        }
    }
    let mut all = px.clone();
    let (mut prev, mut n) = (px, N);
    while n > 1 {
        let m = n / 2;
        let mut next = vec![255u8; m * m * 4];
        for y in 0..m {
            for x in 0..m {
                for ch in 0..3 {
                    let s: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| prev[((2 * y + dy) * n + 2 * x + dx) * 4 + ch] as u32).sum();
                    next[(y * m + x) * 4 + ch] = (s / 4) as u8;
                }
            }
        }
        all.extend_from_slice(&next);
        prev = next;
        n = m;
    }
    let mut im = Image::new(Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 }, TextureDimension::D2, vec![0; N * N * 4], TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default());
    im.texture_descriptor.mip_level_count = 11;
    im.data = Some(all);
    im.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::Repeat,
        address_mode_v: ImageAddressMode::Repeat,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        anisotropy_clamp: 16,
        ..default()
    });
    im
}

fn target_image() -> Image {
    const N: usize = 64;
    let mut px = vec![255u8; N * N * 4];
    for y in 0..N {
        for x in 0..N {
            let (dx, dy) = (x as f32 / (N - 1) as f32 - 0.5, y as f32 / (N - 1) as f32 - 0.5);
            let r = (dx * dx + dy * dy).sqrt() * 2.0;
            let c: [u8; 3] = if r > 1.0 { [60, 60, 62] } else if r < 0.12 { [230, 40, 30] } else if (r * 5.0) as i32 % 2 == 0 { [215, 190, 140] } else { [150, 40, 30] };
            px[(y * N + x) * 4..][..3].copy_from_slice(&c);
        }
    }
    Image::new(Extent3d { width: N as u32, height: N as u32, depth_or_array_layers: 1 }, TextureDimension::D2, px, TextureFormat::Rgba8UnormSrgb, RenderAssetUsages::default())
}

// ---- range targets ------------------------------------------------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum Kind {
    Static,
    /// Slides along x by +-amp m at speed rad/s.
    Slide(f32, f32),
    /// Up for 60% of `period` s, hidden below the floor otherwise; offset by phase s.
    Pop(f32, f32),
}

#[derive(Component)]
struct RangeTarget {
    origin: Vec3,
    kind: Kind,
}

struct Slot {
    e: Entity,
    pos: Vec3,
    kind: Kind,
    timer: f32,
}

#[derive(Resource)]
struct Targets {
    art: [(Handle<Mesh>, Handle<StandardMaterial>); 2], // board, pole
    slots: Vec<Slot>,
}

fn spawn_target(cmd: &mut Commands, art: &[(Handle<Mesh>, Handle<StandardMaterial>); 2], pos: Vec3, kind: Kind) -> Entity {
    cmd.spawn((
        crate::level::MapRoot,
        RangeTarget { origin: pos, kind },
        Health(100.0),
        Mesh3d(art[0].0.clone()),
        MeshMaterial3d(art[0].1.clone()),
        Transform::from_translation(pos),
    ))
    .with_children(|c| {
        c.spawn((Mesh3d(art[1].0.clone()), MeshMaterial3d(art[1].1.clone()), Transform::from_xyz(0.0, -0.45 - (pos.y - 0.45) / 2.0, -0.04).with_scale(Vec3::new(1.0, pos.y - 0.45, 1.0))));
    })
    .id()
}

fn range_targets(time: Res<Time>, mut t: ResMut<Targets>, mut cmd: Commands, alive: Query<(), With<RangeTarget>>) {
    let art = t.art.clone();
    for s in &mut t.slots {
        if alive.contains(s.e) {
            s.timer = RESPAWN;
        } else {
            s.timer -= time.delta_secs();
            if s.timer <= 0.0 {
                s.e = spawn_target(&mut cmd, &art, s.pos, s.kind);
                s.timer = RESPAWN;
            }
        }
    }
}

fn move_targets(time: Res<Time>, mut q: Query<(&mut Transform, &RangeTarget)>) {
    let t = time.elapsed_secs();
    for (mut tf, r) in &mut q {
        tf.translation = r.origin
            + match r.kind {
                Kind::Static => Vec3::ZERO,
                Kind::Slide(amp, speed) => Vec3::X * amp * (t * speed).sin(),
                Kind::Pop(period, phase) => Vec3::NEG_Y * if (t + phase) % period < period * 0.6 { 0.0 } else { 2.2 },
            };
    }
}

// ---- world ------------------------------------------------------------------------------------------------------------------------

pub struct GridPlugin;
impl Plugin for GridPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (teleport, range_targets, move_targets).run_if(resource_exists::<Targets>));
    }
}

/// level.rs switch: drop the grid's resources (its targets are MapRoot entities).
pub fn unload(world: &mut World) {
    world.remove_resource::<Targets>();
    world.remove_resource::<Colliders>();
}

fn teleport(keys: Res<ButtonInput<KeyCode>>, mut q: Single<(&mut Player, &mut Transform)>, targets: Res<Targets>) {
    // the level's start zone, once per load (Targets is inserted by each grid load)
    let start = targets.is_added().then(|| crate::level::level()[1].clone());
    let dest = start.and_then(|s| ZONES.iter().find(|z| z.0 == s)).or_else(|| ZONE_KEYS.iter().position(|&k| keys.just_pressed(k)).map(|i| &ZONES[i]));
    if let Some(&(name, pos, yaw)) = dest {
        let (p, tf) = &mut *q;
        (tf.translation, p.yaw, p.pitch, p.vel) = (pos, yaw, 0.0, Vec3::ZERO);
        info!("teleport {name}");
    }
}

pub fn spawn(cmd: &mut Commands, meshes: &mut Assets<Mesh>, mats: &mut Assets<StandardMaterial>, images: &mut Assets<Image>) {
    let grid = images.add(grid_image());
    let tint = |mats: &mut Assets<StandardMaterial>, c: Color| mats.add(StandardMaterial { base_color: c, base_color_texture: Some(grid.clone()), perceptual_roughness: 0.95, ..default() });
    let (floor, lobby_m, range_m, course_m, cqb_m) = (
        tint(mats, Color::WHITE),
        tint(mats, Color::srgb(0.75, 0.85, 1.2)),
        tint(mats, Color::srgb(1.0, 1.15, 0.85)),
        tint(mats, Color::srgb(1.25, 1.1, 0.8)),
        tint(mats, Color::srgb(0.8, 0.8, 0.85)),
    );
    let target_tex = images.add(target_image());
    let board_m = mats.add(StandardMaterial { base_color_texture: Some(target_tex), perceptual_roughness: 0.9, ..default() });
    let art = [(meshes.add(Cuboid::new(0.7, 0.9, 0.04)), board_m), (meshes.add(Cuboid::new(0.06, 1.0, 0.06)), mats.add(StandardMaterial { base_color: Color::srgb(0.85, 0.85, 0.85), ..default() }))];
    let mut b = B { cmd, meshes, mats, images, col: vec![] };
    let (wall, conc, crate_m, yellow, dark, red) = (b.mat(0.45, 0.45, 0.43), b.mat(0.3, 0.3, 0.32), b.mat(0.3, 0.36, 0.2), b.mat(0.9, 0.8, 0.1), b.mat(0.15, 0.15, 0.17), b.mat(0.8, 0.15, 0.1));

    // Floor, zone patches (same grid texture, tinted) and the perimeter.
    let (x0, x1, z0, z1) = (-80.0, 80.0, -110.0, 25.0);
    b.tiled(x0, z0, x1, z1, 0.0, &floor);
    for (m, a, c) in [(&lobby_m, (-8.0, -8.0), (8.0, 8.0)), (&range_m, (-56.0, -72.0), (-20.0, -12.0)), (&course_m, (22.0, -96.0), (58.0, -12.0)), (&cqb_m, (-14.0, -74.0), (14.0, -46.0))] {
        b.tiled(a.0, a.1, c.0, c.1, 0.015, m);
    }
    b.wall(true, z0, x0, x1, 4.0, &[], &wall);
    b.wall(true, z1, x0, x1, 4.0, &[], &wall);
    b.wall(false, x0, z0, z1, 4.0, &[], &wall);
    b.wall(false, x1, z0, z1, 4.0, &[], &wall);

    // Lobby: key board + zone board ahead of the spawn, concrete barriers further out.
    b.panel(
        Vec3::new(-4.0, 2.7, -7.0),
        Quat::IDENTITY,
        6.0,
        &["BODYCAM TEST GRID", "", "WASD MOVE  SHIFT SPRINT", "CTRL CROUCH  SPACE JUMP", "LMB FIRE  RMB AIM  R RELOAD", "Q E LEAN  1-5 WEAPONS", "", "F1 LOBBY    F3 RANGE", "F4 COURSE   F5 CQB", "F7 BOTS HOSTILE/PASSIVE", "F10 RESPAWN BOTS", "F2 POSTFX   F6 GUNFIGHT"],
        [20, 24, 40],
        [255, 255, 255],
    );
    b.panel(
        Vec3::new(4.5, 3.0, -7.0),
        Quat::IDENTITY,
        4.5,
        &["ZONES", "", "F3 RANGE", "<- WEST 38 M", "STATIC AND MOVING", "TARGETS 5-50 M", "", "F4 COURSE", "EAST 40 M ->", "WALLS RAMPS STAIRS", "", "F5 CQB", "AHEAD 45 M", "BOTS INSIDE"],
        [40, 24, 20],
        [255, 230, 80],
    );
    for (x, z) in [(-6.0, -30.0), (6.0, -26.0), (-10.0, -38.0), (12.0, -36.0)] {
        b.bx(Vec3::new(x, 0.5, z), Vec3::new(3.0, 1.0, 0.6), &conc, true);
    }

    // Shooting range: firing line z=-16, distance stripes + boards at 5/10/25/50 m, backstop at z=-72.
    let fl = -16.0;
    b.banner(-38.0, -10.0, 2.4, 7.0, &["SHOOTING RANGE", "STATIC LANES  MOVING  POP-UPS"], [30, 50, 30]);
    b.bx(Vec3::new(-38.0, 0.02, fl), Vec3::new(36.0, 0.02, 0.2), &yellow, false);
    b.wall(true, -72.0, -56.0, -20.0, 5.0, &[], &conc);
    b.wall(false, -56.0, -72.0, -12.0, 3.0, &[], &wall);
    b.wall(false, -20.0, -72.0, -12.0, 3.0, &[], &wall);
    let lanes = [-50.0, -42.0, -34.0, -26.0];
    for (i, &x) in lanes.iter().enumerate() {
        for d in [5.0, 10.0, 25.0, 50.0] {
            b.bx(Vec3::new(x, 0.02, fl - d), Vec3::new(7.0, 0.02, 0.12), &red, false);
            b.floor_label(x - 2.5, fl - d + 0.8, 1.2, &[&format!("{d:.0} M")]);
        }
        if i < 3 {
            b.bx(Vec3::new(x + 4.0, 0.6, fl - 3.0), Vec3::new(0.2, 1.2, 6.0), &wall, true); // booth divider
        }
        b.bx(Vec3::new(x, 0.5, fl + 0.4), Vec3::new(1.6, 1.0, 0.5), &crate_m, true); // bench
    }
    let mut slots = vec![];
    let mut tgt = |b: &mut B, x: f32, d: f32, kind: Kind| {
        let pos = Vec3::new(x, 1.4, fl - d);
        slots.push(Slot { e: spawn_target(b.cmd, &art, pos, kind), pos, kind, timer: RESPAWN });
    };
    for (i, d) in [5.0, 10.0, 25.0, 50.0].into_iter().enumerate() {
        tgt(&mut b, -50.0 + if i % 2 == 0 { -1.5 } else { 1.5 }, d, Kind::Static);
        tgt(&mut b, -42.0 + if i % 2 == 0 { 1.5 } else { -1.5 }, d, Kind::Static);
    }
    tgt(&mut b, -34.0, 15.0, Kind::Slide(2.5, 0.8));
    tgt(&mut b, -34.0, 30.0, Kind::Slide(2.5, 1.3));
    tgt(&mut b, -26.0, 10.0, Kind::Pop(5.0, 0.0));
    tgt(&mut b, -26.0, 20.0, Kind::Pop(5.0, 1.7));
    tgt(&mut b, -26.0, 35.0, Kind::Pop(6.0, 3.1));
    b.floor_label(-34.0, fl - 20.0, 3.0, &["MOVING"]);
    b.floor_label(-26.0, fl - 15.0, 3.0, &["POP-UPS"]);

    // Obstacle course (x 22..58, z -12..-96), runs toward -Z.
    b.banner(40.0, -10.0, 2.4, 7.0, &["OBSTACLE COURSE", "STEP  JUMP  CLIMB  CORRIDOR"], [60, 45, 20]);
    b.wall(false, 22.0, -96.0, -12.0, 3.0, &[], &wall);
    b.wall(false, 58.0, -96.0, -12.0, 3.0, &[], &wall);
    b.wall(true, -96.0, 22.0, 58.0, 3.0, &[], &wall);
    for (z, h, label) in [(-20.0, 0.4, "STEP 0.4 M"), (-26.0, 0.8, "JUMP 0.8 M")] {
        b.bx(Vec3::new(40.0, h / 2.0, z), Vec3::new(36.0, h, 0.4), &conc, true);
        b.floor_label(40.0, z + 1.2, 3.0, &[label]);
    }
    b.wall(true, -32.0, 22.0, 58.0, 1.2, &[(38.0, 42.0, 0.0, 9.0)], &conc);
    b.floor_label(40.0, -30.8, 3.0, &["1.2 M - GAP"]);
    for (i, h) in [0.45, 0.9, 1.35].into_iter().enumerate() {
        b.bx(Vec3::new(28.0 + i as f32, h / 2.0, -44.0), Vec3::new(1.0, h, 1.0), &crate_m, true);
    }
    b.floor_label(29.0, -42.3, 3.0, &["CRATE STAIRS"]);
    for (x, z) in [(34.0, -42.0), (46.0, -46.0), (38.0, -50.0), (50.0, -52.0), (42.0, -40.0)] {
        b.bx(Vec3::new(x, 0.6, z), Vec3::new(1.5, 1.2, 1.5), &crate_m, true);
    }
    // Ramp up to a 1.4 m platform, stairs down; stairs up to a 1.6 m platform, ramp down.
    b.wedge(Vec2::new(27.0, -70.0), Vec2::new(31.0, -58.0), 1.4, 0.0, true, &conc);
    b.bx(Vec3::new(31.0, 0.7, -74.0), Vec3::new(10.0, 1.4, 8.0), &wall, true);
    for i in 0..6 {
        let h = 1.2 - 0.2 * i as f32;
        b.bx(Vec3::new(36.3 + 0.6 * i as f32, h / 2.0, -75.0), Vec3::new(0.6, h, 4.0), &wall, true);
    }
    for i in 0..8 {
        let h = 0.2 * (i + 1) as f32;
        b.bx(Vec3::new(47.0, h / 2.0, -58.25 - 0.5 * i as f32), Vec3::new(4.0, h, 0.5), &wall, true);
    }
    b.bx(Vec3::new(49.0, 0.8, -66.0), Vec3::new(10.0, 1.6, 8.0), &wall, true);
    b.wedge(Vec2::new(50.0, -80.0), Vec2::new(54.0, -70.0), 0.0, 1.6, true, &conc);
    // Narrow corridor (1 m wide, open top) into a doorway.
    b.wall(false, 39.35, -90.0, -82.0, 2.5, &[], &dark);
    b.wall(false, 40.65, -90.0, -82.0, 2.5, &[], &dark);
    b.wall(true, -92.0, 22.0, 58.0, 3.0, &[DOOR(39.0, 41.0)], &wall);
    b.floor_label(40.0, -80.5, 3.0, &["NARROW 1 M"]);

    // CQB block (centre x 0, z -60): rooms, doorways, windows, cover. Local coordinates, +z towards the lobby.
    let cz = -60.0;
    b.banner(0.0, -42.0, 2.8, 6.0, &["CQB BLOCK", "BOTS INSIDE  F7 TOGGLE"], [60, 20, 20]);
    let walls: [(bool, f32, f32, f32, Vec<Opening>); 10] = [
        (true, -14.0, -14.0, 14.0, vec![]),
        (true, 14.0, -14.0, 14.0, vec![DOOR(-1.0, 1.0)]),
        (false, -14.0, -14.0, 14.0, vec![WINDOW(-8.0, -6.5), WINDOW(2.0, 3.5)]),
        (false, 14.0, -14.0, 14.0, vec![WINDOW(-3.0, -1.5), WINDOW(6.0, 7.5)]),
        (true, 4.0, -14.0, 14.0, vec![DOOR(-9.0, -7.0), WINDOW(-1.0, 0.5), DOOR(5.0, 7.0)]),
        (true, -6.0, -14.0, 14.0, vec![WINDOW(-8.0, -6.5), DOOR(-1.0, 1.0), DOOR(9.0, 11.0)]),
        (false, -5.0, -14.0, -6.0, vec![DOOR(-11.0, -9.0)]),
        (false, 5.0, -14.0, -6.0, vec![WINDOW(-13.0, -11.5), DOOR(-9.0, -7.0)]),
        (false, -4.0, -6.0, 4.0, vec![DOOR(-1.0, 1.0)]),
        (false, 4.0, -6.0, 4.0, vec![DOOR(-1.0, 1.0)]),
    ];
    for (axis_x, at, a, c, ops) in &walls {
        // axis_x walls vary x (at = local z); the others vary z (at = local x).
        if *axis_x {
            b.wall(true, cz + at, *a, *c, 3.0, ops, &wall);
        } else {
            b.wall(false, *at, cz + a, cz + c, 3.0, &ops.iter().map(|o| (cz + o.0, cz + o.1, o.2, o.3)).collect::<Vec<_>>(), &wall);
        }
    }
    for (x, z, w, h, d) in [(-8.0, 10.0, 1.4, 1.2, 1.4), (7.0, 11.0, 1.4, 1.2, 1.4), (0.0, 9.0, 2.0, 0.8, 0.8), (-9.0, -1.0, 1.6, 0.8, 0.8), (9.0, 1.0, 1.2, 1.2, 1.2), (0.0, -2.0, 1.0, 1.0, 1.0), (-10.0, -10.0, 1.2, 1.2, 1.2), (10.0, -10.0, 2.0, 0.8, 0.8), (0.0, -10.0, 1.4, 1.2, 1.4)] {
        b.bx(Vec3::new(x, h / 2.0, cz + z), Vec3::new(w, h, d), &crate_m, true);
    }

    // Lighting.
    let col = Colliders(std::mem::take(&mut b.col));
    b.cmd.spawn((DirectionalLight { illuminance: 8_000.0, shadow_maps_enabled: true, ..default() }, Transform::from_xyz(30.0, 40.0, 20.0).looking_at(Vec3::new(0.0, 0.0, -40.0), Vec3::Y)));
    b.cmd.insert_resource(GlobalAmbientLight { brightness: 700.0, ..default() });
    b.cmd.insert_resource(col);
    b.cmd.insert_resource(Targets { art, slots });

    // Bots: CQB (6) + two course guards, routes stay inside their room (no path finding).
    let l = |x: f32, z: f32| Vec3::new(x, 0.0, cz + z);
    let route = |pts: &[(f32, f32)]| pts.iter().map(|&(x, z)| l(x, z)).collect::<Vec<_>>();
    let mut specs = vec![];
    for pts in [&[(-9.0, 8.0), (-4.0, 12.0), (-10.0, 12.0)][..], &[(9.0, 8.0), (4.0, 12.0), (10.0, 6.0)], &[(0.0, 0.0), (2.5, -3.0), (-2.5, -3.0)], &[(-10.0, -2.0), (-7.0, 1.5), (-12.0, 1.0)], &[(10.0, -3.0), (7.0, 2.0)], &[(0.0, -9.0), (-2.0, -12.0), (2.0, -12.0)]] {
        specs.push(BotSpec { route: route(pts) });
    }
    for pts in [[(30.0, -87.0), (34.0, -87.0)], [(46.0, -87.0), (50.0, -87.0)]] {
        specs.push(BotSpec { route: pts.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect() });
    }
    bots::setup(b.cmd, b.meshes, b.mats, specs);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blk(min: (f32, f32), max: (f32, f32), h0: f32, h1: f32) -> Block {
        Block { min: min.into(), max: max.into(), h0, h1, along_z: true }
    }

    /// A tall wall pushes the player out, a 0.4 m step lifts them, a ramp follows its slope, and a fall lands on a box.
    #[test]
    fn walls_steps_ramps() {
        let c = Colliders(vec![blk((-5.0, 0.0), (5.0, 1.0), 2.0, 2.0), blk((-5.0, 3.0), (5.0, 4.0), 0.4, 0.4), blk((-5.0, 6.0), (5.0, 10.0), 0.0, 2.0)]);
        let (mut pos, mut vel) = (Vec3::new(0.0, 0.0, -0.1), Vec3::new(0.0, 0.0, 1.0));
        c.collide(0.0, &mut pos, &mut vel, true);
        assert!(pos.z <= -RADIUS + 1e-3 && vel.z == 0.0, "wall: {pos}");
        let mut pos = Vec3::new(0.0, 0.0, 3.5);
        assert!(c.collide(0.0, &mut pos, &mut Vec3::ZERO, true) && (pos.y - 0.4).abs() < 1e-4);
        let mut pos = Vec3::new(0.0, 0.0, 6.5);
        assert!(c.collide(0.0, &mut pos, &mut Vec3::ZERO, true) && (pos.y - 0.25).abs() < 1e-4);
        let mut pos = Vec3::new(0.0, 1.5, 0.5);
        assert!(c.collide(1.6, &mut pos, &mut Vec3::new(0.0, -3.0, 0.0), false) && (pos.y - 2.0).abs() < 1e-4);
    }

    fn wall(min: Vec2, max: Vec2, h: f32) -> Block {
        Block { min, max, h0: h, h1: h, along_z: false }
    }

    #[test]
    fn path_goes_around_walls_and_over_steps() {
        // Wall along x = 5 from z -10..10 with a gap at z 3..5: the route must pass through the gap.
        let c = Colliders(vec![wall(Vec2::new(4.8, -10.0), Vec2::new(5.2, 3.0), 3.0), wall(Vec2::new(4.8, 5.0), Vec2::new(5.2, 10.0), 3.0)]);
        let p = c.path(Vec2::new(0.0, 0.0), 0.0, Vec2::new(10.0, 0.0), 0.3);
        assert!(p.last().is_some_and(|l| l.distance(Vec2::new(10.0, 0.0)) < 1e-3));
        assert!(p.iter().any(|w| (w.x - 5.0).abs() < 0.6 && w.y > 3.0 && w.y < 5.0), "{p:?}");
        // A 1 m high crate (> STEP) is walked around, a 0.3 m one over.
        let c = Colliders(vec![wall(Vec2::new(4.0, -0.4), Vec2::new(6.0, 0.4), 1.0)]);
        assert!(c.path(Vec2::new(0.0, 0.0), 0.0, Vec2::new(10.0, 0.0), 0.3).iter().any(|w| w.y.abs() > 0.5));
        let c = Colliders(vec![wall(Vec2::new(4.0, -0.4), Vec2::new(6.0, 0.4), 0.3)]);
        assert!(c.path(Vec2::new(0.0, 0.0), 0.0, Vec2::new(10.0, 0.0), 0.3).iter().all(|w| w.y.abs() < 0.1));
    }
}
