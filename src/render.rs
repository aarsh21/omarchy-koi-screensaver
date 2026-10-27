//! Terminal renderer for the koi pond. It keeps the original's rasterisation idea -- sample the
//! koi outline per cell, then paint a skin "tile" behind a dense glyph -- but tiles are ANSI
//! background colours and glyphs are terminal characters instead of canvas fillRect/fillText.

use std::collections::HashMap;

use crate::shape::{body_half_width, classify, ShapeHit, FLAG_BARBEL, FLAG_EYE, FLAG_FIN};
use crate::world::{
    clamp, hash3, ripple_life, smoothstep, Koi, Variety, World, BITE_LIFE, CELL_H, CELL_W,
    KOI_NODES, PELLET_LIFE, PELLET_SINK, RIPPLE_SPEED,
};

pub type Rgb = [u8; 3];

#[derive(Clone, Copy)]
pub struct Palette {
    /// Pond background, which tiles are mixed toward.
    pub bg: Rgb,
    pub white: Rgb,
    pub red: Rgb,
    pub black: Rgb,
    pub fin: Rgb,
    pub eye: Rgb,
    pub water: Rgb,
    pub pellet: Rgb,
    pub shadow: Rgb,
    pub shadow_alpha: f64,
    pub alpha: f64,
    /// How much a deep koi fades, as the alpha it keeps at the bottom of the pond.
    pub depth_floor: f64,
    /// How much skin colour a tile carries, from shadow (0) to lit (1); the rest is pond background.
    pub tile: (f64, f64),
    /// Hi (the red-orange of koi markings) from deep shadow to a lit highlight, so patches shade across hues.
    pub hi: [Rgb; 6],
    /// Glyph brightness relative to the tile under it.
    pub ink: f64,
}

pub const DARK: Palette = Palette {
    bg: [0, 0, 0],
    white: [236, 228, 218],
    red: [232, 96, 56],
    black: [92, 90, 108],
    fin: [214, 206, 198],
    eye: [20, 20, 26],
    water: [255, 247, 240],
    pellet: [236, 170, 92],
    shadow: [0, 0, 0],
    shadow_alpha: 0.55,
    alpha: 0.8,
    depth_floor: 0.4,
    tile: (0.68, 0.98),
    hi: [
        [112, 22, 24],
        [168, 34, 28],
        [214, 58, 30],
        [240, 96, 34],
        [250, 138, 52],
        [255, 178, 96],
    ],
    ink: 0.42,
};

// Dense glyphs etched into each skin tile, sparse where it catches the light.
const TILE_GLYPHS: &[u8] = b"t%SX8@#";
const SUBSAMPLES: [(f64, f64); 4] = [(0.25, 0.25), (0.75, 0.25), (0.25, 0.75), (0.75, 0.75)];

pub const fn pack(c: Rgb) -> u32 {
    ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
}

pub fn unpack(v: u32) -> Rgb {
    [(v >> 16) as u8, (v >> 8) as u8, v as u8]
}

pub fn mix(a: Rgb, b: Rgb, k: f64) -> Rgb {
    [
        (a[0] as f64 + (b[0] as f64 - a[0] as f64) * k) as u8,
        (a[1] as f64 + (b[1] as f64 - a[1] as f64) * k) as u8,
        (a[2] as f64 + (b[2] as f64 - a[2] as f64) * k) as u8,
    ]
}

fn shade(c: Rgb, k: f64) -> Rgb {
    [
        (c[0] as f64 * k).min(255.0) as u8,
        (c[1] as f64 * k).min(255.0) as u8,
        (c[2] as f64 * k).min(255.0) as u8,
    ]
}

/// Sample a colour ramp at t in [0, 1].
fn ramp(stops: &[Rgb], t: f64) -> Rgb {
    let x = clamp(t, 0.0, 1.0) * (stops.len() - 1) as f64;
    let i = ((stops.len() - 2) as f64).min(x.floor()) as usize;
    mix(stops[i], stops[i + 1], x - i as f64)
}

/// Blend the packed `color` over `base` by `a`, returning packed 24-bit.
fn blend(base: Rgb, color: u32, a: f64) -> u32 {
    let k = clamp(a, 0.0, 1.0);
    let c = unpack(color);
    let r = base[0] as f64 + (c[0] as f64 - base[0] as f64) * k;
    let g = base[1] as f64 + (c[1] as f64 - base[1] as f64) * k;
    let b = base[2] as f64 + (c[2] as f64 - base[2] as f64) * k;
    (((r as u32) & 255) << 16) | (((g as u32) & 255) << 8) | ((b as u32) & 255)
}

/// A line glyph that follows a direction in screen space, for fin rays and ripple rings.
fn line_glyph(dx: f64, dy: f64) -> u8 {
    let d = dx.hypot(dy).max(1e-9);
    if dy.abs() < 0.38 * d {
        b'-'
    } else if dx.abs() < 0.38 * d {
        b'|'
    } else if dx * dy > 0.0 {
        b'\\'
    } else {
        b'/'
    }
}

fn value_noise(x: f64, y: f64, seed: i64) -> f64 {
    let xi = x.floor();
    let yi = y.floor();
    let u = smoothstep(0.0, 1.0, x - xi);
    let v = smoothstep(0.0, 1.0, y - yi);
    let a = hash3(xi as i64, yi as i64, seed);
    let b = hash3(xi as i64 + 1, yi as i64, seed);
    let c = hash3(xi as i64, yi as i64 + 1, seed);
    let d = hash3(xi as i64 + 1, yi as i64 + 1, seed);
    a + (b - a) * u + (c - a) * v + (a - b - c + d) * u * v
}

/// A marking in body space: centred at (s, v) along and across the spine, as fractions of length.
#[derive(Clone, Copy)]
struct Patch {
    s: f64,
    v: f64,
    rs: f64,
    rv: f64,
}

#[derive(Default)]
struct Markings {
    red: Vec<Patch>,
    black: Vec<Patch>,
}

fn in_patch(m: &Patch, s: f64, vn: f64, wobble: f64) -> bool {
    ((s - m.s) / m.rs).powi(2) + ((vn - m.v) / m.rv).powi(2) < 1.0 + wobble
}

/// Each koi gets a fixed set of markings, seeded by its id, so every fish carries a clear pattern:
/// kohaku a red head and two or three red saddles on white, sanke the same plus small black spots,
/// tancho a single red disc on the crown.
fn koi_markings(k: &Koi) -> Markings {
    let r = |j: i64| hash3(k.id as i64, j, 41);
    if k.variety == Variety::Tancho {
        return Markings {
            red: vec![Patch {
                s: 0.11,
                v: 0.0,
                rs: 0.05,
                rv: 0.055,
            }],
            black: Vec::new(),
        };
    }

    let mut red = Vec::new();
    if r(0) < 0.75 {
        red.push(Patch {
            s: 0.1,
            v: (r(1) - 0.5) * 0.03,
            rs: 0.055 + r(2) * 0.03,
            rv: 0.11,
        });
    }
    let saddles = 2 + (r(3) * 2.0) as i32;
    for j in 0..saddles {
        let j = j as i64;
        let span = 0.42 / saddles as f64;
        red.push(Patch {
            s: 0.25 + span * (j as f64 + 0.5) + (r(10 + j) - 0.5) * span * 0.3,
            v: (r(20 + j) - 0.5) * 0.05,
            rs: span * (0.32 + r(30 + j) * 0.12),
            rv: 0.09 + r(40 + j) * 0.05,
        });
    }

    let mut black = Vec::new();
    if k.variety == Variety::Sanke {
        let spots = 4 + (r(50) * 3.0) as i32;
        for j in 0..spots {
            let j = j as i64;
            black.push(Patch {
                s: 0.24 + r(60 + j) * 0.44,
                v: (r(70 + j) - 0.5) * 0.16,
                rs: 0.022,
                rv: 0.026,
            });
        }
    }
    Markings { red, black }
}

/// One rendered terminal frame: a glyph and two colours per cell.
#[derive(Clone)]
pub struct Frame {
    pub cols: usize,
    pub rows: usize,
    /// ASCII character per cell; a space is a blank cell.
    pub glyph: Vec<u8>,
    pub fg: Vec<u32>,
    pub bg: Vec<u32>,
}

impl Frame {
    fn new(cols: usize, rows: usize) -> Self {
        Self {
            cols,
            rows,
            glyph: vec![b' '; cols * rows],
            fg: vec![0; cols * rows],
            bg: vec![0; cols * rows],
        }
    }

    /// Plain-text rendering (glyphs only), used by tests and the `KOI_DUMP_FRAMES` debug mode.
    pub fn to_text(&self) -> String {
        let mut out = String::with_capacity((self.cols + 1) * self.rows);
        for r in 0..self.rows {
            let start = r * self.cols;
            for c in 0..self.cols {
                out.push(self.glyph[start + c] as char);
            }
            out.push('\n');
        }
        out
    }
}

pub struct PondRenderer {
    cols: usize,
    rows: usize,
    cw: f64,
    ch: f64,
    palette: Palette,

    cell_glyph: Vec<u8>,
    cell_fg: Vec<u32>,
    cell_fg_a: Vec<f32>,
    cell_bg: Vec<u32>,
    cell_bg_a: Vec<f32>,
    /// Cells under a koi's shadow on the pond floor.
    cell_shadow: Vec<u8>,

    energy: Vec<f32>,
    energy_tmp: Vec<f32>,

    spine: [f64; KOI_NODES * 2],
    arc: [f64; KOI_NODES],
    arc_total: f64,
    flare_left: f64,
    flare_right: f64,
    // Scratch outputs of sample(), to avoid allocating per sample.
    hit: ShapeHit,
    hit_s: f64,
    hit_v: f64,
    hit_dist: f64,
    hit_tx: f64,
    hit_ty: f64,
    hit_seg: usize,

    markings: HashMap<u32, Markings>,

    pub frame: Frame,
}

impl Default for PondRenderer {
    fn default() -> Self {
        Self::new()
    }
}

impl PondRenderer {
    pub fn new() -> Self {
        Self {
            cols: 0,
            rows: 0,
            cw: CELL_W,
            ch: CELL_H,
            palette: DARK,
            cell_glyph: Vec::new(),
            cell_fg: Vec::new(),
            cell_fg_a: Vec::new(),
            cell_bg: Vec::new(),
            cell_bg_a: Vec::new(),
            cell_shadow: Vec::new(),
            energy: Vec::new(),
            energy_tmp: Vec::new(),
            spine: [0.0; KOI_NODES * 2],
            arc: [0.0; KOI_NODES],
            arc_total: 1.0,
            flare_left: 1.0,
            flare_right: 1.0,
            hit: ShapeHit::default(),
            hit_s: 0.0,
            hit_v: 0.0,
            hit_dist: 0.0,
            hit_tx: 1.0,
            hit_ty: 0.0,
            hit_seg: 0,
            markings: HashMap::new(),
            frame: Frame::new(0, 0),
        }
    }

    pub fn set_cell_aspect(&mut self, aspect: f64) {
        self.ch = self.cw * aspect;
    }

    pub fn cell_aspect(&self) -> f64 {
        self.ch / self.cw
    }

    /// The portfolio's canvas grid, in logical pixels, independent of the desktop's text size.
    pub fn resize_canvas(&mut self, width: f64, height: f64) {
        self.cw = (width / 125.0).round().clamp(8.0, 12.0);
        self.ch = self.cw + 2.0;
        self.resize(
            (width / self.cw).ceil() as usize,
            (height / self.ch).ceil() as usize,
        );
    }

    pub fn cell_size(&self) -> (f64, f64) {
        (self.cw, self.ch)
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        let n = cols * rows;
        self.cell_glyph = vec![0; n];
        self.cell_fg = vec![0; n];
        self.cell_fg_a = vec![0.0; n];
        self.cell_bg = vec![0; n];
        self.cell_bg_a = vec![0.0; n];
        self.cell_shadow = vec![0; n];
        self.energy = vec![0.0; n];
        self.energy_tmp = vec![0.0; n];
        self.frame = Frame::new(cols, rows);
        self.markings.clear();
    }

    pub fn width(&self) -> f64 {
        self.cols as f64 * self.cw
    }

    pub fn height(&self) -> f64 {
        self.rows as f64 * self.ch
    }

    fn put_glyph(&mut self, i: usize, glyph: u8, rgb: Rgb, alpha: f64) {
        if alpha <= 0.004 {
            return;
        }
        self.cell_glyph[i] = glyph;
        self.cell_fg[i] = pack(rgb);
        self.cell_fg_a[i] = alpha as f32;
    }

    #[allow(clippy::too_many_arguments)]
    fn put_tile(&mut self, i: usize, glyph: u8, fg: Rgb, fg_a: f64, bg: Rgb, bg_a: f64) {
        self.cell_glyph[i] = glyph;
        self.cell_fg[i] = pack(fg);
        self.cell_fg_a[i] = fg_a as f32;
        self.cell_bg[i] = pack(bg);
        self.cell_bg_a[i] = bg_a as f32;
    }

    fn cell_index(&self, x: f64, y: f64) -> Option<usize> {
        let c = (x / self.cw).floor();
        let r = (y / self.ch).floor();
        if c < 0.0 || r < 0.0 || c >= self.cols as f64 || r >= self.rows as f64 {
            return None;
        }
        Some(r as usize * self.cols + c as usize)
    }

    /// Rasterise the pond into `self.frame`. `dt` is the simulation step, used by the wake decay.
    pub fn render(&mut self, world: &World, dt: f64) {
        if self.cols == 0 {
            return;
        }
        self.cell_glyph.fill(0);
        self.cell_fg_a.fill(0.0);
        self.cell_bg_a.fill(0.0);
        self.cell_shadow.fill(0);

        // Deeper koi first, so the ones near the surface glide over them.
        let mut order: Vec<usize> = (0..world.koi.len()).collect();
        order.sort_by(|a, b| {
            world.koi[*a]
                .depth
                .partial_cmp(&world.koi[*b].depth)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for idx in order {
            let koi = world.koi[idx].clone();
            self.rasterize_koi(&koi, world.time);
        }
        self.deposit_wake(world);
        self.diffuse(dt);

        self.draw_shadows();
        self.draw_wake();
        self.draw_ripples(world);
        self.draw_pellets(world);
        self.draw_bites(world);
        self.compose();
    }

    /// Blend every layer down to two colours per cell: the tile behind the glyph, and the glyph ink.
    fn compose(&mut self) {
        let bg_base = pack(self.palette.bg);
        for i in 0..self.frame.glyph.len() {
            let bg_a = self.cell_bg_a[i] as f64;
            let bg = if bg_a > 0.0 {
                blend(self.palette.bg, self.cell_bg[i], bg_a)
            } else {
                bg_base
            };
            self.frame.bg[i] = bg;
            let glyph = self.cell_glyph[i];
            if glyph == 0 {
                self.frame.glyph[i] = b' ';
                self.frame.fg[i] = bg_base;
                continue;
            }
            self.frame.glyph[i] = glyph;
            self.frame.fg[i] = blend(unpack(bg), self.cell_fg[i], self.cell_fg_a[i] as f64);
        }
    }

    fn sample(&mut self, px: f64, py: f64, l: f64, from: usize, to: usize) -> bool {
        let mut best = f64::INFINITY;
        let mut best_s = 0.0;
        let mut best_sign = 1.0;
        let mut past = 0.0;
        let mut ahead = 0.0;
        let mut lateral = 0.0;
        for n in from..=to {
            let ax = self.spine[n * 2];
            let ay = self.spine[n * 2 + 1];
            let sx = self.spine[(n + 1) * 2] - ax;
            let sy = self.spine[(n + 1) * 2 + 1] - ay;
            let len2 = (sx * sx + sy * sy).max(1e-9);
            let raw = ((px - ax) * sx + (py - ay) * sy) / len2;
            let u = clamp(raw, 0.0, 1.0);
            let d2 = (px - ax - sx * u).powi(2) + (py - ay - sy * u).powi(2);
            if d2 < best {
                let len = len2.sqrt();
                best = d2;
                best_s = (self.arc[n] + u * len) / self.arc_total;
                best_sign = if sx * (py - ay) - sy * (px - ax) >= 0.0 {
                    1.0
                } else {
                    -1.0
                };
                past = if n == KOI_NODES - 2 {
                    (raw - 1.0).max(0.0) * len
                } else {
                    0.0
                };
                ahead = if n == 0 { (-raw).max(0.0) * len } else { 0.0 };
                lateral = (sx * (py - ay) - sy * (px - ax)).abs() / len;
                self.hit_tx = sx / len;
                self.hit_ty = sy / len;
                self.hit_seg = n;
            }
        }

        let a = best.sqrt();
        self.hit_dist = a;
        self.hit_s = best_s;
        self.hit_v = a * best_sign;
        let flare = if best_sign > 0.0 {
            self.flare_left
        } else {
            self.flare_right
        };
        let mut hit = std::mem::take(&mut self.hit);
        let inside = classify(
            best_s,
            a / l,
            past / l,
            flare,
            &mut hit,
            ahead / l,
            lateral / l,
        );
        self.hit = hit;
        inside
    }

    /// Skin colour for a body point, from the koi's own markings pinned to body coordinates.
    fn skin(&mut self, k: &Koi, s: f64, v: f64, across: f64) -> Rgb {
        if across > 0.9 || s > 0.74 {
            return self.palette.white;
        }
        let vn = v / k.length;
        // Ragged edges, so patches read as painted markings rather than ellipses.
        let wobble = (value_noise(s * 14.0 + k.id as f64, vn * 30.0, k.id as i64 + 3) - 0.5) * 0.6;
        let markings = self.markings.entry(k.id).or_insert_with(|| koi_markings(k));
        for m in &markings.black {
            if in_patch(m, s, vn, wobble) {
                return self.palette.black;
            }
        }
        for m in &markings.red {
            if in_patch(m, s, vn, wobble) {
                return self.palette.red;
            }
        }
        self.palette.white
    }

    fn rasterize_koi(&mut self, k: &Koi, t: f64) {
        let (cw, ch, cols, rows) = (self.cw, self.ch, self.cols, self.rows);
        let palette = self.palette;
        let l = k.length;
        let nodes = &k.nodes;
        // The body only undulates while the tail is beating; coasting koi hold almost straight.
        let swim = 0.2 + 1.1 * k.beat;

        // Travelling body wave that grows toward the tail.
        for n in 0..KOI_NODES {
            let s = n as f64 / (KOI_NODES - 1) as f64;
            let a = n.saturating_sub(1);
            let b = (n + 1).min(KOI_NODES - 1);
            let mut tx = nodes[a * 2] - nodes[b * 2];
            let mut ty = nodes[a * 2 + 1] - nodes[b * 2 + 1];
            let tl = tx.hypot(ty).max(1e-9);
            tx /= tl;
            ty /= tl;
            let amp = l * 0.04 * (0.05 + 0.95 * s.powf(2.2)) * swim;
            let off = amp * (k.phase - s * 4.6).sin();
            self.spine[n * 2] = nodes[n * 2] - ty * off;
            self.spine[n * 2 + 1] = nodes[n * 2 + 1] + tx * off;
        }
        self.arc[0] = 0.0;
        for n in 1..KOI_NODES {
            self.arc[n] = self.arc[n - 1]
                + (self.spine[n * 2] - self.spine[(n - 1) * 2])
                    .hypot(self.spine[n * 2 + 1] - self.spine[(n - 1) * 2 + 1]);
        }
        self.arc_total = if self.arc[KOI_NODES - 1] > 0.0 {
            self.arc[KOI_NODES - 1]
        } else {
            1.0
        };
        // Pectorals tuck in during a burst, scull in turn while coasting, and the inner one spreads to brake on a turn.
        let spread = 1.0 - 0.25 * k.beat;
        let scull = 0.18 * (1.0 - k.beat) * k.fin_phase.sin();
        let bank = clamp(k.turn * 0.6, -0.35, 0.35);
        self.flare_left = spread + scull - bank;
        self.flare_right = spread - scull + bank;

        let (mut min_x, mut min_y) = (f64::INFINITY, f64::INFINITY);
        let (mut max_x, mut max_y) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for n in 0..KOI_NODES {
            min_x = min_x.min(self.spine[n * 2]);
            max_x = max_x.max(self.spine[n * 2]);
            min_y = min_y.min(self.spine[n * 2 + 1]);
            max_y = max_y.max(self.spine[n * 2 + 1]);
        }
        let reach = l * 0.26;
        let c0 = ((min_x - reach) / cw).floor().max(0.0) as usize;
        let c1 = (((max_x + reach) / cw).floor() as isize).min(cols as isize - 1);
        let r0 = ((min_y - reach) / ch).floor().max(0.0) as usize;
        let r1 = (((max_y + reach) / ch).floor() as isize).min(rows as isize - 1);
        if c1 < c0 as isize || r1 < r0 as isize {
            return;
        }
        let (c1, r1) = (c1 as usize, r1 as usize);

        let diag = cw.hypot(ch);
        // Near the surface a koi is brighter and its shadow falls farther away on the floor.
        let shadow_dc = ((l * 0.07 * k.depth) / cw).round() as usize + 1;
        let shadow_dr = ((l * 0.05 * k.depth) / ch).round() as usize + 1;
        let dim = palette.alpha * (palette.depth_floor + (1.0 - palette.depth_floor) * k.depth);
        let (tile0, tile1) = palette.tile;

        for r in r0..=r1 {
            let mut c = c0;
            while c <= c1 {
                let x = c as f64 * cw;
                let y = r as f64 * ch;
                let center_hit = self.sample(x + cw / 2.0, y + ch / 2.0, l, 0, KOI_NODES - 2);
                // Nothing reaches far from the spine at the waist, where the fins end.
                if !center_hit
                    && self.hit_dist
                        > l * if self.hit_s > 0.4 && self.hit_s < 0.62 {
                            0.15
                        } else {
                            0.25
                        } + diag
                {
                    // Distance to the spine changes by at most a cell per cell, so skip cells that can't reach it.
                    let skip = ((self.hit_dist - l * 0.25 - diag) / cw).floor().max(0.0) as usize;
                    c += skip;
                    if skip == 0 {
                        c += 1;
                    }
                    continue;
                }
                // Sub-samples sit within a cell of the centre, so only the neighbouring segments can be nearest.
                let from = self.hit_seg.saturating_sub(2);
                let to = (self.hit_seg + 2).min(KOI_NODES - 2);
                let mut flag = self.hit.flag;
                let mut across = self.hit.across;
                let mut s = self.hit_s;
                let mut v = self.hit_v;
                let mut tx = self.hit_tx;
                let mut ty = self.hit_ty;

                let mut hits = if center_hit { 1 } else { 0 };
                // Points from the cell centre toward the covered side, i.e. across the outline into the koi.
                let (mut gx, mut gy) = (0.0, 0.0);
                // Cells well inside the body can't have a sub-sample outside it, so skip straight to full coverage.
                let deep_inside = center_hit
                    && flag != FLAG_FIN
                    && flag != FLAG_BARBEL
                    && s > 0.04
                    && s < 0.72
                    && body_half_width(s) * l - self.hit_dist > diag * 0.35;
                if deep_inside {
                    hits = 5;
                }
                if !deep_inside {
                    for &(qx, qy) in SUBSAMPLES.iter() {
                        if !self.sample(x + qx * cw, y + qy * ch, l, from, to) {
                            continue;
                        }
                        gx += (qx - 0.5) * cw;
                        gy += (qy - 0.5) * ch;
                        if hits == 0 {
                            flag = self.hit.flag;
                            across = self.hit.across;
                            s = self.hit_s;
                            v = self.hit_v;
                            tx = self.hit_tx;
                            ty = self.hit_ty;
                        }
                        hits += 1;
                    }
                }
                if hits == 0 {
                    c += 1;
                    continue;
                }
                self.hit_tx = tx;
                self.hit_ty = ty;
                let coverage = hits as f64 / 5.0;
                let side = if v >= 0.0 { 1.0 } else { -1.0 };

                let i = r * cols + c;
                // Body-anchored key so each glyph rides along with the skin.
                let key = (hash3(
                    (s * 90.0).round() as i64,
                    (v / (cw * 0.8)).round() as i64,
                    k.id as i64,
                ) * 4_294_967_295.0) as u32;
                let solid = center_hit && coverage >= 0.6;

                let mut glyph = b' ';
                let mut rgb = palette.fin;
                let mut alpha = dim;
                let mut tile: Option<Rgb> = None;
                let mut tile_alpha = 0.0;
                if flag == FLAG_BARBEL {
                    // Drawn along the whisker: forward from the mouth and angled out to its side.
                    glyph = line_glyph(-tx - side * ty * 0.7, -ty + side * tx * 0.7);
                    rgb = shade(mix(palette.bg, palette.fin, 0.9), palette.ink + 0.3);
                } else if flag == FLAG_EYE && solid {
                    tile = Some(mix(palette.bg, palette.eye, 0.85));
                    tile_alpha = dim;
                    glyph = b'o';
                    rgb = mix(palette.bg, palette.white, 0.9);
                } else if flag == FLAG_FIN {
                    // The original pale tile veil defines the fins, even between the dot flecks.
                    tile = Some(mix(palette.bg, palette.fin, 0.55));
                    tile_alpha = dim * 0.45 * coverage;
                    if key.is_multiple_of(2) {
                        glyph = b'.';
                        alpha *= 0.6 * coverage;
                    }
                } else {
                    // Each skin cell is a tile washed in the skin colour, shaded like a rounded back lit from
                    // the top-left, with a dense glyph etched on top. Edge cells get a paler, empty tile.
                    let skin = self.skin(k, s, v, across);
                    let facing = side * (0.6 * ty - 0.8 * tx) * across;
                    let lum = clamp(
                        0.22 + 0.5 * (1.0 - across * across).sqrt() + 0.18 * facing,
                        0.0,
                        1.0,
                    );
                    let tile_color = if skin == palette.red {
                        // Red patches run the full hi ramp: crimson in shadow, vermilion across the back, orange
                        // to amber where the light hits. Each koi leans a little redder or more orange, and a
                        // slow body-pinned noise varies the hue within a patch, like real pigment.
                        let lean = (hash3(k.id as i64, 5, 17) - 0.5) * 0.3;
                        let grain =
                            (value_noise(s * 9.0 + k.id as f64, (v / l) * 18.0, k.id as i64 + 21)
                                - 0.5)
                                * 0.25;
                        // A band of shine travels down the body, like light sliding over wet scales.
                        let sheen = ((s * 11.0 - t * 1.8 + k.id as f64).sin() - 0.7).max(0.0) / 0.3;
                        ramp(&palette.hi, 0.15 + 0.8 * lum + lean + grain + sheen * 0.35)
                    } else {
                        mix(palette.bg, skin, tile0 + (tile1 - tile0) * lum)
                    };
                    tile = Some(tile_color);
                    if hits == 5 {
                        let sc = c + shadow_dc;
                        let sr = r + shadow_dr;
                        if sc < cols && sr < rows {
                            self.cell_shadow[sr * cols + sc] = 1;
                        }
                        tile_alpha = dim;
                        if s < 0.035 && across < 0.6 && k.gulp > 0.5 {
                            glyph = b'o';
                            rgb = palette.eye;
                        } else {
                            let jitter = (key >> 4) % 3;
                            let idx = clamp(
                                ((1.0 - lum) * 4.0).round() + jitter as f64,
                                0.0,
                                (TILE_GLYPHS.len() - 1) as f64,
                            ) as usize;
                            glyph = TILE_GLYPHS[idx];
                            rgb = shade(tile_color, palette.ink);
                            // White skin is etched lightly so the red and black patches carry the pattern.
                            if skin == palette.white {
                                alpha *= 0.6;
                            }
                            // Scales glint on a staggered clock.
                            let tick = (t * 1.4 + (key % 997) as f64 / 97.0).floor() as i64;
                            if hash3(key as i64, tick, 7) < 0.03 {
                                tile = Some(mix(tile_color, palette.bg, 0.45));
                            }
                        }
                    } else {
                        tile_alpha = dim * coverage * 0.55;
                        if gx.hypot(gy) > cw * 0.2 && key.is_multiple_of(3) {
                            glyph = b'.';
                            rgb = shade(tile_color, palette.ink);
                        }
                    }
                }
                if glyph == b' ' && tile_alpha == 0.0 {
                    c += 1;
                    continue;
                }
                self.put_tile(i, glyph, rgb, alpha, tile.unwrap_or(palette.bg), tile_alpha);
                c += 1;
            }
        }
    }

    // ── water ───────────────────────────────────────────────────────────

    fn deposit_wake(&mut self, world: &World) {
        for k in world.koi.iter() {
            for n in KOI_NODES - 4..KOI_NODES {
                if let Some(i) = self.cell_index(k.nodes[n * 2], k.nodes[n * 2 + 1]) {
                    self.energy[i] = (self.energy[i] + 0.035 + k.speed as f32 * 0.0012).min(1.5);
                }
            }
        }
    }

    /// Blur and decay so wakes spread like ripples and fade.
    fn diffuse(&mut self, dt: f64) {
        let (cols, rows) = (self.cols, self.rows);
        let decay = (-dt * 1.1).exp() as f32;
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                let e = &self.energy;
                let left = if c > 0 { e[i - 1] } else { e[i] };
                let right = if c < cols - 1 { e[i + 1] } else { e[i] };
                let up = if r > 0 { e[i - cols] } else { e[i] };
                let down = if r < rows - 1 { e[i + cols] } else { e[i] };
                self.energy_tmp[i] = (e[i] * 4.0 + left + right + up + down) * 0.125 * decay;
            }
        }
        std::mem::swap(&mut self.energy, &mut self.energy_tmp);
    }

    fn draw_shadows(&mut self) {
        let (shadow, alpha) = (self.palette.shadow, self.palette.shadow_alpha);
        for i in 0..self.cell_shadow.len() {
            if self.cell_shadow[i] == 0 || self.cell_glyph[i] != 0 {
                continue;
            }
            self.put_glyph(i, b':', shadow, alpha);
        }
    }

    fn draw_wake(&mut self) {
        let water = self.palette.water;
        for i in 0..self.energy.len() {
            let e = self.energy[i] as f64;
            if e < 0.22 || self.cell_glyph[i] != 0 || self.cell_shadow[i] != 0 {
                continue;
            }
            self.put_glyph(
                i,
                if e > 0.5 { b'-' } else { b'.' },
                water,
                ((e - 0.22) * 1.4).min(1.0) * 0.35,
            );
        }
    }

    /// Expanding rings drawn as ASCII circles: the glyph follows the ring's tangent.
    fn draw_ripples(&mut self, world: &World) {
        let (cw, ch, cols, rows) = (self.cw, self.ch, self.cols, self.rows);
        let water = self.palette.water;
        for rp in world.ripples.iter() {
            let age = world.time - rp.born;
            let life = ripple_life(rp);
            let fade = (1.0 - age / life).max(0.0).powf(1.5) * rp.strength;
            for ring in 0..2 {
                let radius =
                    (age - ring as f64 * 0.45).max(0.0) * RIPPLE_SPEED * (0.7 + 0.5 * rp.strength);
                if radius < 2.0 {
                    continue;
                }
                let band = cw * 0.6;
                let c0 = ((rp.x - radius - cw) / cw).floor().max(0.0) as usize;
                let c1 = (((rp.x + radius + cw) / cw).floor() as isize).min(cols as isize - 1);
                let r0 = ((rp.y - radius - ch) / ch).floor().max(0.0) as usize;
                let r1 = (((rp.y + radius + ch) / ch).floor() as isize).min(rows as isize - 1);
                if c1 < c0 as isize || r1 < r0 as isize {
                    continue;
                }
                let (c1, r1) = (c1 as usize, r1 as usize);
                let alpha = fade * if ring == 0 { 0.55 } else { 0.3 };
                for r in r0..=r1 {
                    for c in c0..=c1 {
                        let i = r * cols + c;
                        if self.cell_glyph[i] != 0 {
                            continue;
                        }
                        let dx = c as f64 * cw + cw / 2.0 - rp.x;
                        let dy = r as f64 * ch + ch / 2.0 - rp.y;
                        let d = dx.hypot(dy);
                        if (d - radius).abs() > band {
                            continue;
                        }
                        let glyph = if ring == 1 { b'.' } else { line_glyph(-dy, dx) };
                        self.put_glyph(i, glyph, water, alpha);
                    }
                }
            }
        }
    }

    /// A bite flashes * over the koi's mouth, settling to + then a dot as the splash takes over.
    fn draw_bites(&mut self, world: &World) {
        let pellet = self.palette.pellet;
        let alpha_scale = (self.palette.alpha * 1.6).min(1.0);
        for b in world.bites.iter() {
            let i = match self.cell_index(b.x, b.y) {
                Some(i) => i,
                None => continue,
            };
            let t = (world.time - b.born) / BITE_LIFE;
            let glyph = if t < 0.35 {
                b'*'
            } else if t < 0.7 {
                b'+'
            } else {
                b'.'
            };
            self.put_glyph(i, glyph, pellet, alpha_scale * (1.0 - t * t));
        }
    }

    /// Pellets fall in (' then .), land as o with a splash, bob while floating, then dim to . as they sink.
    fn draw_pellets(&mut self, world: &World) {
        let (ch, pellet, alpha) = (self.ch, self.palette.pellet, self.palette.alpha);
        for p in world.pellets.iter() {
            let age = world.time - p.born;
            if !p.landed {
                let fall = age / p.land_at;
                // Drawn a little above where it will land, closing in as it drops.
                let i = match self.cell_index(p.x, p.y - (1.0 - fall) * ch * 1.5) {
                    Some(i) => i,
                    None => continue,
                };
                if self.cell_glyph[i] != 0 {
                    continue;
                }
                self.put_glyph(
                    i,
                    if fall < 0.5 { b'\'' } else { b'.' },
                    pellet,
                    alpha * (0.5 + 0.4 * fall),
                );
                continue;
            }
            let i = match self.cell_index(p.x, p.y) {
                Some(i) => i,
                None => continue,
            };
            if self.cell_glyph[i] != 0 {
                continue;
            }
            let sink = smoothstep(PELLET_SINK, PELLET_LIFE, age);
            let bob = 0.8 + 0.2 * (age * 3.0 + p.x).sin();
            self.put_glyph(
                i,
                if sink > 0.55 { b'.' } else { b'o' },
                pellet,
                alpha * bob * (1.0 - 0.85 * sink),
            );
        }
    }
}
