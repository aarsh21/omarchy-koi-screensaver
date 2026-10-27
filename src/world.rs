//! Koi pond simulation, ported from the canvas "koi pond" on dao-ho's portfolio
//! (github.com/Dao-Ho/portfolio, src/app/components/koi-pond). The physics, flocking, depth,
//! feeding and tail-beat behaviour are the original's; only the drawing target changed, from a
//! `<canvas>` to a terminal cell grid.

pub const TAU: f64 = std::f64::consts::TAU;
pub const KOI_NODES: usize = 20;
pub const RIPPLE_SPEED: f64 = 52.0;

/// Terminal grid unit scale: the pond is measured in "pixels", and a cell is CELL_W x CELL_H of them.
pub const CELL_W: f64 = 10.0;
pub const CELL_H: f64 = 20.0;

pub const PELLET_LIFE: f64 = 14.0;
pub const BITE_LIFE: f64 = 0.45;
/// Uneaten pellets start to sink this many seconds in, fading out by PELLET_LIFE.
pub const PELLET_SINK: f64 = 9.0;

const RUSH_RANGE: f64 = 650.0;
const MAX_PELLETS: usize = 24;
const THROW_RADIUS: f64 = 34.0;

pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

pub fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

pub fn angle_diff(a: f64, b: f64) -> f64 {
    let mut d = (b - a) % TAU;
    if d > std::f64::consts::PI {
        d -= TAU;
    }
    if d < -std::f64::consts::PI {
        d += TAU;
    }
    d
}

/// Deterministic integer hash to `[0, 1)`, so textures stay pinned to the body. Matches the
/// JavaScript original, which relies on Math.imul's 32-bit wrapping multiply.
pub fn hash3(x: i64, y: i64, z: i64) -> f64 {
    let mut h = (x as i32 as u32).wrapping_mul(374_761_393)
        ^ (y as i32 as u32).wrapping_mul(668_265_263)
        ^ (z as i32 as u32).wrapping_mul(1_442_695_041);
    h = (h ^ (h >> 13)).wrapping_mul(1_274_126_177);
    h ^= h >> 16;
    h as f64 / 4_294_967_296.0
}

/// Small xorshift generator so a run is reproducible from its seed.
pub struct Rng {
    state: u64,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            state: if seed == 0 {
                0x9E37_79B9_7F4A_7C15
            } else {
                seed
            },
        }
    }

    pub fn next_f64(&mut self) -> f64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        (x >> 11) as f64 / 9_007_199_254_740_992.0
    }

    pub fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + self.next_f64() * (hi - lo)
    }

    fn index(&mut self, n: usize) -> usize {
        (self.next_f64() * n as f64) as usize
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Variety {
    Kohaku,
    Sanke,
    Tancho,
}

impl Variety {
    pub fn name(self) -> &'static str {
        match self {
            Variety::Kohaku => "kohaku",
            Variety::Sanke => "sanke",
            Variety::Tancho => "tancho",
        }
    }
}

#[derive(Clone)]
pub struct Koi {
    pub id: u32,
    pub variety: Variety,
    pub length: f64,
    /// Spine positions, snout first: [x0, y0, x1, y1, ...].
    pub nodes: [f64; KOI_NODES * 2],
    pub heading: f64,
    pub speed: f64,
    pub phase: f64,
    /// Smoothed angular velocity, used to splay the fins into turns.
    pub turn: f64,
    pub wander: f64,
    /// 0.5 deep and dim to 1 just under the surface.
    pub depth: f64,
    pub depth_target: f64,
    pub depth_max: f64,
    pub next_dive: f64,
    pub next_surface: f64,
    /// When a koi swimming near the surface next sends a small ring off its tail.
    pub next_wake: f64,
    /// 1 just after food hits the water nearby, fading: the koi sprints and turns harder while it lasts.
    pub rush: f64,
    /// The pellet this koi is going for. It sticks with it so a burst of food doesn't yank it around.
    pub food: Option<u64>,
    /// Tail-beat intensity: koi swim in bursts of beats, then coast with the tail still.
    pub beat: f64,
    pub burst_until: f64,
    pub next_burst: f64,
    /// Phase of the pectoral fins, which scull gently while the koi coasts or hovers.
    pub fin_phase: f64,
    /// How far the mouth is open, as the koi closes in on food.
    pub gulp: f64,
}

impl Koi {
    fn placeholder() -> Self {
        Self {
            id: 0,
            variety: Variety::Kohaku,
            length: 1.0,
            nodes: [0.0; KOI_NODES * 2],
            heading: 0.0,
            speed: 0.0,
            phase: 0.0,
            turn: 0.0,
            wander: 0.0,
            depth: 1.0,
            depth_target: 1.0,
            depth_max: 1.0,
            next_dive: 0.0,
            next_surface: 0.0,
            next_wake: 0.0,
            rush: 0.0,
            food: None,
            beat: 0.0,
            burst_until: 0.0,
            next_burst: 0.0,
            fin_phase: 0.0,
            gulp: 0.0,
        }
    }
}

#[derive(Clone)]
pub struct Pellet {
    pub id: u64,
    pub x: f64,
    pub y: f64,
    pub born: f64,
    /// Seconds after `born` that it hits the water; until then it's falling and koi can't eat it.
    pub land_at: f64,
    pub landed: bool,
    /// Slow drift once it's floating.
    pub vx: f64,
    pub vy: f64,
}

/// The moment a koi takes a pellet, drawn as a brief flash where it happened.
#[derive(Clone)]
pub struct Bite {
    pub x: f64,
    pub y: f64,
    pub born: f64,
}

#[derive(Clone)]
pub struct Ripple {
    pub x: f64,
    pub y: f64,
    pub born: f64,
    pub strength: f64,
}

pub fn ripple_life(r: &Ripple) -> f64 {
    2.2 + r.strength * 1.6
}

pub struct World {
    pub width: f64,
    pub height: f64,
    pub time: f64,
    pub koi: Vec<Koi>,
    pub pellets: Vec<Pellet>,
    pub bites: Vec<Bite>,
    pub ripples: Vec<Ripple>,
    rng: Rng,
    next_pellet_id: u64,
}

impl World {
    pub fn new(seed: u64) -> Self {
        Self {
            width: 0.0,
            height: 0.0,
            time: 0.0,
            koi: Vec::new(),
            pellets: Vec::new(),
            bites: Vec::new(),
            ripples: Vec::new(),
            rng: Rng::new(seed),
            next_pellet_id: 1,
        }
    }

    /// Place a fresh shoal for the current pond size.
    pub fn reseed(&mut self) {
        self.koi.clear();
        self.pellets.clear();
        self.bites.clear();
        self.ripples.clear();
        self.seed();
    }

    pub fn resize(&mut self, width: f64, height: f64) {
        let first = self.width == 0.0;
        self.width = width;
        self.height = height;
        if first {
            self.seed();
        }
    }

    fn seed(&mut self) {
        let (w, h) = (self.width, self.height);
        let min_side = w.min(h);
        let base_length = clamp(min_side * 0.5, 170.0, 420.0);
        let count = clamp((w * h / 240_000.0).round(), 3.0, 6.0) as usize;
        // Wide ponds also get a couple of young koi that keep to the deep water, for a sense of depth.
        let young = if w >= 1024.0 { 2 } else { 0 };

        self.koi.clear();
        for i in 0..count + young {
            let is_young = i >= count;
            let length = base_length
                * if is_young {
                    self.rng.range(0.32, 0.45)
                } else {
                    self.rng.range(0.6, 1.0)
                };
            let depth_max = if is_young { 0.72 } else { 1.0 };
            let heading = self.rng.range(0.0, TAU);
            let x = self.rng.range(w * 0.1, w * 0.9);
            let y = self.rng.range(h * 0.1, h * 0.9);
            let mut nodes = [0.0; KOI_NODES * 2];
            let seg = length / (KOI_NODES - 1) as f64;
            for n in 0..KOI_NODES {
                nodes[n * 2] = x - heading.cos() * seg * n as f64;
                nodes[n * 2 + 1] = y - heading.sin() * seg * n as f64;
            }
            let variety = match self.rng.index(6) {
                0..=2 => Variety::Kohaku,
                3..=4 => Variety::Sanke,
                _ => Variety::Tancho,
            };
            self.koi.push(Koi {
                id: i as u32 + 1,
                variety,
                length,
                nodes,
                heading,
                speed: 20.0,
                phase: self.rng.range(0.0, TAU),
                turn: 0.0,
                wander: 0.0,
                depth: self.rng.range(0.5, depth_max),
                depth_target: self.rng.range(0.5, depth_max),
                depth_max,
                next_dive: self.rng.range(6.0, 18.0),
                next_surface: self.rng.range(3.0, 14.0),
                next_wake: self.rng.range(0.0, 2.0),
                rush: 0.0,
                food: None,
                beat: 0.0,
                burst_until: 0.0,
                next_burst: self.rng.range(0.0, 3.0),
                fin_phase: self.rng.range(0.0, TAU),
                gulp: 0.0,
            });
        }
    }

    /// A loose handful thrown at (x, y): each pellet lands a beat apart with its own splash, then drifts and sinks.
    pub fn feed(&mut self, x: f64, y: f64) {
        self.ripples.push(Ripple {
            x,
            y,
            born: self.time,
            strength: 0.35,
        });
        // A full pond takes no more food, rather than dropping old pellets out from under koi already going for them.
        let room = MAX_PELLETS.saturating_sub(self.pellets.len());
        let count = (5 + self.rng.index(3)).min(room);
        for _ in 0..count {
            let a = self.rng.range(0.0, TAU);
            let r = self.rng.next_f64().sqrt() * THROW_RADIUS;
            let drift = self.rng.range(2.0, 6.0);
            let id = self.next_pellet_id;
            self.next_pellet_id += 1;
            self.pellets.push(Pellet {
                id,
                x: x + a.cos() * r * 1.3,
                y: y + a.sin() * r,
                born: self.time,
                land_at: self.rng.range(0.15, 0.45),
                landed: false,
                vx: a.cos() * drift,
                vy: a.sin() * drift,
            });
        }
    }

    pub fn step(&mut self, dt: f64) {
        self.time += dt;
        for i in 0..self.koi.len() {
            let mut k = std::mem::replace(&mut self.koi[i], Koi::placeholder());
            self.step_koi(&mut k, dt);
            self.koi[i] = k;
        }
        let t = self.time;
        self.pellets.retain(|p| t - p.born < PELLET_LIFE);
        let damp = (-dt * 0.2).exp();
        for p in &mut self.pellets {
            if !p.landed {
                if t - p.born < p.land_at {
                    continue;
                }
                p.landed = true;
                let (px, py) = (p.x, p.y);
                self.ripples.push(Ripple {
                    x: px,
                    y: py,
                    born: t,
                    strength: 0.1,
                });
                // The splash is what koi react to: every koi in range turns and bolts for it at once.
                for k in self.koi.iter_mut() {
                    if (k.nodes[0] - px).hypot(k.nodes[1] - py) < RUSH_RANGE {
                        k.rush = 1.0;
                    }
                }
                continue;
            }
            p.vx *= damp;
            p.vy *= damp;
            p.x += p.vx * dt;
            p.y += p.vy * dt;
        }
        self.ripples.retain(|r| t - r.born < ripple_life(r));
        self.bites.retain(|b| t - b.born < BITE_LIFE);
    }

    fn step_koi(&mut self, k: &mut Koi, dt: f64) {
        let (w, h, time) = (self.width, self.height, self.time);
        let l = k.length;
        let hx = k.nodes[0];
        let hy = k.nodes[1];

        let mut dx = k.heading.cos();
        let mut dy = k.heading.sin();

        k.wander = clamp(k.wander + (self.rng.next_f64() - 0.5) * dt * 2.4, -0.9, 0.9);
        dx += (k.heading + k.wander).cos() * 0.6;
        dy += (k.heading + k.wander).sin() * 0.6;

        let range = (l * 1.6 + 220.0).max(if k.rush > 0.0 { RUSH_RANGE } else { 0.0 });
        let food = self.pick_food(k, hx, hy, range);

        let mut chasing = false;
        let mut closing = 0.0;
        if let Some((fx, fy)) = food {
            let tx = fx - hx;
            let ty = fy - hy;
            let d = tx.hypot(ty).max(1e-6);
            dx += (tx / d) * 3.0;
            dy += (ty / d) * 3.0;
            chasing = true;
            closing = clamp(1.0 - d / (l * 1.2), 0.0, 1.0);
        }

        for other in self.koi.iter() {
            // The koi being stepped is swapped for a placeholder while it is out of the vec.
            if other.id == k.id || other.id == 0 {
                continue;
            }
            let (ohx, ohy) = (other.nodes[0], other.nodes[1]);
            if (ohx - hx).abs() > l * 2.0 || (ohy - hy).abs() > l * 2.0 {
                continue;
            }
            for n in (0..KOI_NODES).step_by(3) {
                let ox = hx - other.nodes[n * 2];
                let oy = hy - other.nodes[n * 2 + 1];
                let d = ox.hypot(oy);
                // Koi going for food jostle in close, so personal space shrinks and never outweighs the pull.
                let range = (l + other.length) * if chasing { 0.12 } else { 0.28 };
                if d < range && d > 1e-3 {
                    let weight = (1.0 - d / range) * 2.4;
                    dx += (ox / d) * weight;
                    dy += (oy / d) * weight;
                }
            }
            // Loose shoaling: nearby koi drift the same way.
            if !chasing && (ohx - hx).hypot(ohy - hy) < l * 1.6 {
                dx += other.heading.cos() * 0.18;
                dy += other.heading.sin() * 0.18;
            }
        }

        // Let the body slip off-screen now and then, but always turn back.
        let over = (-hx).max(hx - w).max(-hy).max(hy - h);
        if over > -l * 0.25 {
            let weight = clamp((over + l * 0.25) / (l * 0.5), 0.0, 1.0) * 3.5;
            let to_x = w / 2.0 - hx;
            let to_y = h / 2.0 - hy;
            let d = to_x.hypot(to_y).max(1e-6);
            dx += (to_x / d) * weight;
            dy += (to_y / d) * weight;
        }

        let desired = dy.atan2(dx);
        // Long bodies can't pivot on the spot: the turning circle grows with length.
        let agility = (1.0 + k.rush * 0.6)
            * (if chasing { 1.9 + closing } else { 0.7 }).min((k.speed / l) * 4.0);
        let max_turn = agility * dt;
        let turned = clamp(angle_diff(k.heading, desired), -max_turn, max_turn);
        k.heading += turned;
        k.turn += (turned / dt - k.turn) * (dt * 3.0).min(1.0);

        // Burst and coast: a few strong tail beats, then a long glide that bleeds off speed.
        // Chasing koi beat without pause.
        if time > k.next_burst && time > k.burst_until {
            k.burst_until = time + self.rng.range(0.7, 1.6);
            k.next_burst = k.burst_until + self.rng.range(1.2, 4.0);
        }
        let bursting = chasing || time < k.burst_until;
        k.beat += ((if bursting { 1.0 } else { 0.0 }) - k.beat) * (dt * 3.0).min(1.0);
        // Tail beats near the surface push small rings out behind the koi; gliding or deep koi leave none.
        if k.beat > 0.6 && k.depth > 0.65 && time > k.next_wake {
            let tail = (KOI_NODES - 3) * 2;
            self.ripples.push(Ripple {
                x: k.nodes[tail],
                y: k.nodes[tail + 1],
                born: time,
                strength: 0.25 + 0.2 * k.depth,
            });
            k.next_wake = time + self.rng.range(0.8, 1.6) / k.beat;
        }
        k.rush = (k.rush - dt * 0.7).max(0.0);
        let cruise = 12.0 + l * 0.05;
        let target = (1.0 + k.rush * 0.8)
            * if chasing {
                cruise * (1.9 + closing)
            } else if bursting {
                cruise * 1.35
            } else {
                cruise * 0.45
            };
        k.speed +=
            (target - k.speed) * (dt * if bursting { 1.6 + k.rush * 3.0 } else { 0.35 }).min(1.0);
        k.phase += dt * (0.6 + k.beat * (2.2 + k.speed / 11.0));
        k.fin_phase += dt * 2.4;
        k.gulp +=
            ((if chasing && closing > 0.55 { 1.0 } else { 0.0 }) - k.gulp) * (dt * 6.0).min(1.0);

        // Koi drift between the deep and the surface, and rise for food.
        if time > k.next_dive {
            k.depth_target = self.rng.range(0.5, k.depth_max);
            k.next_dive = time + self.rng.range(8.0, 20.0);
        }
        k.depth += ((if chasing { 1.0 } else { k.depth_target }) - k.depth)
            * (dt * if chasing { 0.8 } else { 0.15 }).min(1.0);

        k.nodes[0] += k.heading.cos() * k.speed * dt;
        k.nodes[1] += k.heading.sin() * k.speed * dt;

        let seg = l / (KOI_NODES - 1) as f64;
        let max_bend = 0.16;
        let mut prev_angle = k.heading + std::f64::consts::PI;
        for n in 1..KOI_NODES {
            let px = k.nodes[(n - 1) * 2];
            let py = k.nodes[(n - 1) * 2 + 1];
            let mut angle = (k.nodes[n * 2 + 1] - py).atan2(k.nodes[n * 2] - px);
            angle = prev_angle + clamp(angle_diff(prev_angle, angle), -max_bend, max_bend);
            k.nodes[n * 2] = px + angle.cos() * seg;
            k.nodes[n * 2 + 1] = py + angle.sin() * seg;
            prev_angle = angle;
        }

        let mouth_r = l * 0.08 + 6.0;
        let before = self.pellets.len();
        let (nx, ny) = (k.nodes[0], k.nodes[1]);
        self.pellets
            .retain(|p| !p.landed || (p.x - nx).powi(2) + (p.y - ny).powi(2) > mouth_r * mouth_r);
        if self.pellets.len() < before {
            self.ripples.push(Ripple {
                x: nx,
                y: ny,
                born: time,
                strength: 0.6,
            });
            self.bites.push(Bite {
                x: nx,
                y: ny,
                born: time,
            });
            k.next_surface = time + self.rng.range(6.0, 16.0);
        } else if time > k.next_surface {
            self.ripples.push(Ripple {
                x: nx,
                y: ny,
                born: time,
                strength: 0.3,
            });
            k.next_surface = time + self.rng.range(8.0, 20.0);
        }
    }

    /// Keep the current target while it lasts; only switch for a pellet less than half as far away.
    fn pick_food(&mut self, k: &mut Koi, x: f64, y: f64, range: f64) -> Option<(f64, f64)> {
        if let Some(id) = k.food {
            if !self.pellets.iter().any(|p| p.id == id) {
                k.food = None;
            }
        }
        let nearest = self.nearest_pellet(x, y, range);
        let current = match k.food {
            None => {
                k.food = nearest;
                return self.food_position(k.food);
            }
            Some(id) => {
                let p = self.pellets.iter().find(|p| p.id == id)?;
                (p.x - x).hypot(p.y - y)
            }
        };
        if current > range * 1.3 {
            k.food = nearest;
        } else if let Some(id) = nearest {
            if let Some(p) = self.pellets.iter().find(|p| p.id == id) {
                if (p.x - x).hypot(p.y - y) < current * 0.5 {
                    k.food = Some(id);
                }
            }
        }
        self.food_position(k.food)
    }

    fn food_position(&self, id: Option<u64>) -> Option<(f64, f64)> {
        let id = id?;
        self.pellets
            .iter()
            .find(|p| p.id == id)
            .filter(|p| p.landed)
            .map(|p| (p.x, p.y))
    }

    fn nearest_pellet(&self, x: f64, y: f64, range: f64) -> Option<u64> {
        let mut best: Option<u64> = None;
        let mut best_d = range * range;
        for p in self.pellets.iter() {
            if !p.landed {
                continue;
            }
            let d = (p.x - x).powi(2) + (p.y - y).powi(2);
            if d < best_d {
                best_d = d;
                best = Some(p.id);
            }
        }
        best
    }
}
