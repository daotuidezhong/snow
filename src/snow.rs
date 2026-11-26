//! Snowfall simulation. Pure math, no platform code, so it can be tested anywhere.
//!
//! Each flake is a small body with inertia moving through an air field:
//! a steady wind, gust fronts that travel across the screen, and swirling
//! turbulence from a divergence-free (curl) noise field that drifts with the
//! wind. Flakes flutter and tumble as they fall; near ones show their crystal
//! shape, far ones are soft and out of focus. A fast mouse drags the air with it.

use crate::art::sprite;
use crate::pile::{clump_area, Ledge, Pile, Puff};

/// How much snow falls.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Intensity {
    Light,
    Normal,
    Heavy,
    Blizzard,
}

impl Intensity {
    pub const ALL: [Intensity; 4] = [Self::Light, Self::Normal, Self::Heavy, Self::Blizzard];

    /// Flakes per million physical pixels.
    fn density(self) -> f32 {
        match self {
            Self::Light => 70.0,
            Self::Normal => 190.0,
            Self::Heavy => 400.0,
            Self::Blizzard => 750.0,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Normal => "normal",
            Self::Heavy => "heavy",
            Self::Blizzard => "blizzard",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.key() == s)
    }
}

/// Wind strength.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Wind {
    Calm,
    Breeze,
    Strong,
}

impl Wind {
    pub const ALL: [Wind; 3] = [Self::Calm, Self::Breeze, Self::Strong];

    /// Steady horizontal speed in logical pixels per second.
    fn speed(self) -> f32 {
        match self {
            Self::Calm => 0.0,
            Self::Breeze => 35.0,
            Self::Strong => 110.0,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Calm => "calm",
            Self::Breeze => "breeze",
            Self::Strong => "strong",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.key() == s)
    }
}

/// Flake size preset.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FlakeSize {
    Small,
    Medium,
    Large,
}

impl FlakeSize {
    pub const ALL: [FlakeSize; 3] = [Self::Small, Self::Medium, Self::Large];

    fn scale(self) -> f32 {
        match self {
            Self::Small => 0.7,
            Self::Medium => 1.0,
            Self::Large => 1.45,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    pub fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.key() == s)
    }
}

/// Everything the user can change from the tray menu.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Weather {
    pub intensity: Intensity,
    pub wind: Wind,
    /// -1 = wind blows to the left, +1 = to the right.
    pub wind_dir: f32,
    pub size: FlakeSize,
}

impl Default for Weather {
    fn default() -> Self {
        Self { intensity: Intensity::Normal, wind: Wind::Breeze, wind_dir: 1.0, size: FlakeSize::Medium }
    }
}

/// Share of a landing flake's area that becomes pile depth. Lower = slower build-up.
const DEPOSIT: f32 = 0.25;

/// What a flake looks like, which also decides how it moves.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    /// Round, soft; far away or small.
    Disc,
    /// Aggregate of crystals: the common big flake.
    Fluff,
    /// A single six-armed crystal: flutters and tumbles the most.
    Crystal,
    /// Right in front of the viewer: huge, faint, blurred.
    Bokeh,
}

/// One snowflake. Position in physical pixels of the overlay; velocity in
/// logical px/s (multiplied by the DPI scale when moving).
#[derive(Clone, Copy, Debug)]
pub struct Flake {
    pub x: f32,
    pub y: f32,
    vx: f32,
    vy: f32,
    /// Body radius in physical pixels.
    pub r: f32,
    /// 0 = far away (small, slow, blurry), 1 = close.
    pub depth: f32,
    pub alpha: f32,
    pub kind: Kind,
    pub sprite: u16,
    /// Rotation in the screen plane, radians.
    pub angle: f32,
    spin: f32,
    /// Phase of the flutter/tumble cycle.
    phase: f32,
    phase_rate: f32,
    flutter: f32,
    /// Apparent width from tumbling (1 = face-on, small = edge-on).
    pub squash: f32,
    /// Fall speed in still air, logical px/s, before perspective.
    fall: f32,
    /// Seconds to catch up with the air (inertia).
    tau: f32,
    /// Passed over a window's top edge without settling there: it is in front of
    /// that window now, so it must not land on anything the window shelters.
    sheltered: bool,
}

/// Small, fast, deterministic PRNG (xorshift64*). Good enough for snow.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in [0, 1).
    pub fn f(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / (1u64 << 24) as f32
    }
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.f()
    }
}

fn hash3(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let mut v = (x as u32).wrapping_mul(0x8DA6_B343)
        ^ (y as u32).wrapping_mul(0xD816_3841)
        ^ (z as u32).wrapping_mul(0xCB1A_B31F)
        ^ seed.wrapping_mul(0x9E37_79B9);
    v ^= v >> 13;
    v = v.wrapping_mul(0x5BD1_E995);
    v ^= v >> 15;
    (v & 0xFFFF) as f32 / 65535.0
}

/// Smooth 3D value noise in [0, 1].
fn noise3(x: f32, y: f32, z: f32, seed: u32) -> f32 {
    let (ix, iy, iz) = (x.floor() as i32, y.floor() as i32, z.floor() as i32);
    let (fx, fy, fz) = (x - ix as f32, y - iy as f32, z - iz as f32);
    let s = |t: f32| t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
    let (ux, uy, uz) = (s(fx), s(fy), s(fz));
    let lerp = |a: f32, b: f32, t: f32| a + (b - a) * t;
    let mut c = [0.0f32; 2];
    for (k, cz) in c.iter_mut().enumerate() {
        let z = iz + k as i32;
        let a = lerp(hash3(ix, iy, z, seed), hash3(ix + 1, iy, z, seed), ux);
        let b = lerp(hash3(ix, iy + 1, z, seed), hash3(ix + 1, iy + 1, z, seed), ux);
        *cz = lerp(a, b, uy);
    }
    lerp(c[0], c[1], uz)
}

/// Air velocity on a coarse grid, rebuilt every frame from curl noise.
struct AirField {
    /// Grid spacing in physical px.
    step: f32,
    /// Grid origin (x of column 0) in physical px; the grid spans the spawn band.
    x0: f32,
    nx: usize,
    ny: usize,
    u: Vec<(f32, f32)>,
    psi: Vec<f32>,
}

impl AirField {
    fn new() -> Self {
        Self { step: 64.0, x0: 0.0, nx: 0, ny: 0, u: Vec::new(), psi: Vec::new() }
    }

    /// Bilinear sample, logical px/s.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        if self.nx < 2 || self.ny < 2 {
            return (0.0, 0.0);
        }
        let gx = ((x - self.x0) / self.step).clamp(0.0, (self.nx - 1) as f32 - 0.001);
        let gy = (y / self.step).clamp(0.0, (self.ny - 1) as f32 - 0.001);
        let (ix, iy) = (gx as usize, gy as usize);
        let (fx, fy) = (gx - ix as f32, gy - iy as f32);
        let g = |i: usize, j: usize| self.u[j * self.nx + i];
        let (a, b, c, d) = (g(ix, iy), g(ix + 1, iy), g(ix, iy + 1), g(ix + 1, iy + 1));
        let top = (a.0 + (b.0 - a.0) * fx, a.1 + (b.1 - a.1) * fx);
        let bot = (c.0 + (d.0 - c.0) * fx, c.1 + (d.1 - c.1) * fx);
        (top.0 + (bot.0 - top.0) * fy, top.1 + (bot.1 - top.1) * fy)
    }
}

pub struct Snow {
    pub flakes: Vec<Flake>,
    pub piles: Vec<Pile>,
    pub puffs: Vec<Puff>,
    /// Seconds since start; drives twinkles and noise.
    pub time: f32,
    width: f32,
    height: f32,
    /// Physical pixels per logical pixel (DPI / 96).
    scale: f32,
    weather: Weather,
    rng: Rng,
    /// Mean horizontal wind in logical px/s, eased toward the target.
    wind_now: f32,
    gust_amp: f32,
    /// How far the air has travelled, so eddies drift with the wind.
    advect: f32,
    /// How far gust fronts have travelled (they ride on the mean flow, gusts included).
    gust_advect: f32,
    air: AirField,
    cursor: Option<(f32, f32)>,
    prev_cursor: Option<(f32, f32)>,
    cursor_v: (f32, f32),
}

impl Snow {
    pub fn new(width: f32, height: f32, scale: f32, weather: Weather, seed: u64) -> Self {
        let mut s = Self {
            flakes: Vec::new(),
            piles: Vec::new(),
            puffs: Vec::new(),
            time: 0.0,
            width,
            height,
            scale,
            weather,
            rng: Rng::new(seed),
            wind_now: weather.wind.speed() * weather.wind_dir,
            gust_amp: 0.0,
            advect: 0.0,
            gust_advect: 0.0,
            air: AirField::new(),
            cursor: None,
            prev_cursor: None,
            cursor_v: (0.0, 0.0),
        };
        // Start with the screen already snowing instead of an empty sky.
        let n = s.target_count();
        for _ in 0..n {
            let mut f = s.spawn();
            f.y = s.rng.range(-0.1 * height, height);
            s.flakes.push(f);
        }
        s
    }

    pub fn set_weather(&mut self, w: Weather) {
        let resize = w.size != self.weather.size;
        self.weather = w;
        if resize {
            for i in 0..self.flakes.len() {
                let f = self.flakes[i];
                self.flakes[i].r = self.radius_for(f.depth, f.kind);
            }
        }
    }

    pub fn resize(&mut self, width: f32, height: f32, scale: f32) {
        let (ow, oh) = (self.width.max(1.0), self.height.max(1.0));
        for f in &mut self.flakes {
            f.x = f.x / ow * width;
            f.y = f.y / oh * height;
        }
        self.width = width;
        self.height = height;
        // Window positions are about to be re-scanned in the new coordinates.
        self.piles.clear();
        self.puffs.clear();
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            for i in 0..self.flakes.len() {
                let f = self.flakes[i];
                self.flakes[i].r = self.radius_for(f.depth, f.kind);
            }
        }
    }

    /// Replace the set of surfaces snow can land on. `interval` is the time
    /// since the previous scan, used to fling snow off windows being dragged.
    pub fn set_ledges(&mut self, ledges: &[Ledge], interval: f32) {
        let cell = 3.0 * self.scale;
        let iv = interval.max(0.016);
        let cap = 2500.0 * self.scale;
        let mut old = std::mem::take(&mut self.piles);
        for l in ledges {
            let found = old.iter().position(|p| p.id == l.id);
            let pile = match found.map(|i| old.swap_remove(i)) {
                Some(mut p) if (p.x0 - l.x0).abs() < 1.0 && (p.x1 - l.x1).abs() < 1.0 && (p.y - l.y).abs() < 1.0 => {
                    p.visible = l.visible.clone();
                    p.max_h = l.max_h;
                    p
                }
                Some(p) => {
                    // The window moved: its snow keeps the window's momentum.
                    let v = (((l.x0 - p.x0) / iv).clamp(-cap, cap), ((l.y - p.y) / iv).clamp(-cap, cap));
                    p.shake_off(&mut self.rng, self.scale, v, &mut self.puffs);
                    Pile::new(l, cell)
                }
                None => Pile::new(l, cell),
            };
            self.piles.push(pile);
        }
        for p in old {
            p.shake_off(&mut self.rng, self.scale, (0.0, 0.0), &mut self.puffs);
        }
    }

    /// Knock all settled snow off at once (menu: clear snow). It falls as clumps.
    pub fn clear_piles(&mut self) {
        for p in &mut self.piles {
            p.shake_off(&mut self.rng, self.scale, (0.0, 0.0), &mut self.puffs);
            p.h.iter_mut().for_each(|h| *h = 0.0);
            p.spill_left = 0.0;
            p.spill_right = 0.0;
        }
    }

    /// Mouse position in overlay pixels (None when unknown). Call once per frame before `step`.
    pub fn set_cursor(&mut self, pos: Option<(f32, f32)>, dt: f32) {
        self.prev_cursor = self.cursor;
        self.cursor_v = match (self.cursor, pos) {
            (Some(a), Some(b)) if dt > 0.0 => ((b.0 - a.0) / dt, (b.1 - a.1) / dt),
            _ => (0.0, 0.0),
        };
        self.cursor = pos;
    }

    /// How far, in pixels, wind carries a typical flake sideways while it falls the screen height.
    fn drift(&self) -> f32 {
        let boost = if self.weather.intensity == Intensity::Blizzard { 1.6 } else { 1.0 };
        // Generous on purpose: slow crystals drift farther than the average flake,
        // and too narrow a band leaves the upwind edge of the screen thin.
        let fall_time = self.height / (34.0 * boost * self.scale);
        ((self.wind_now.abs() + self.gust_amp * 0.8) * self.scale * fall_time).min(4.0 * self.width)
    }

    /// The x range new flakes start in: the screen plus the strip upwind of it
    /// that the wind will carry onto the screen, so snow stays even across it.
    fn band(&self) -> (f32, f32) {
        let pad = 0.05 * self.width;
        let d = self.drift();
        if self.wind_now >= 0.0 {
            (-d - pad, self.width + pad)
        } else {
            (-pad, self.width + d + pad)
        }
    }

    /// Number of flakes the current screen and intensity call for.
    pub fn target_count(&self) -> usize {
        let (a, b) = self.band();
        let mpx = (b - a) * self.height / 1.0e6;
        // Bigger DPI scale means bigger flakes, so fewer are needed for the same look.
        let n = mpx * self.weather.intensity.density() / self.scale.max(1.0);
        (n as usize).clamp(10, 20_000)
    }

    fn radius_for(&self, depth: f32, kind: Kind) -> f32 {
        // Perspective: size grows steeply for the nearest flakes, so their crystal
        // structure is visible, while the far majority stay small.
        let base = (1.2 + 2.4 * depth + 6.5 * depth.powi(4)) * self.scale * self.weather.size.scale();
        match kind {
            Kind::Bokeh => base * 3.2,
            Kind::Crystal => base * 1.35,
            Kind::Fluff => base * 1.1,
            Kind::Disc => base,
        }
    }

    fn spawn(&mut self) -> Flake {
        let rng = &mut self.rng;
        // Most flakes are far away; a rare few drift right past the "lens".
        let depth = if rng.f() < 0.004 { rng.range(0.985, 1.0) } else { rng.f().powf(1.6) * 0.98 };
        let kind = if depth >= 0.985 {
            Kind::Bokeh
        } else if depth > 0.55 {
            // Up close, real flakes show structure: never a smooth ball.
            if rng.f() < 0.5 {
                Kind::Crystal
            } else {
                Kind::Fluff
            }
        } else if depth > 0.35 && rng.f() < 0.3 {
            Kind::Fluff
        } else {
            Kind::Disc
        };
        let pick = |rng: &mut Rng, ids: &[u16]| ids[((rng.f() * ids.len() as f32) as usize).min(ids.len() - 1)];
        let sprite = match kind {
            Kind::Bokeh => sprite::BOKEH,
            Kind::Crystal => pick(rng, &sprite::CRYSTAL),
            Kind::Fluff => pick(rng, &sprite::FLUFF),
            // Depth of field: the farther, the blurrier.
            Kind::Disc => sprite::DISC[((1.0 - depth) * 3.6).clamp(0.0, 3.0) as usize],
        };
        let alpha = match kind {
            Kind::Bokeh => rng.range(0.18, 0.32),
            _ => 0.45 + 0.55 * depth,
        };
        let (flutter, phase_rate, fall_k, tau) = match kind {
            // Crystals are light and plate-like: they glide, flutter and react fast.
            Kind::Crystal => (rng.range(14.0, 30.0), rng.range(1.2, 2.6), 0.8, rng.range(0.18, 0.35)),
            Kind::Fluff => (rng.range(8.0, 18.0), rng.range(0.8, 1.8), 0.95, rng.range(0.3, 0.55)),
            Kind::Disc => (rng.range(4.0, 12.0), rng.range(0.5, 1.4), 1.0, rng.range(0.3, 0.6)),
            Kind::Bokeh => (rng.range(6.0, 14.0), rng.range(0.3, 0.8), 1.0, 0.8),
        };
        let fall = 52.0 * fall_k * rng.range(0.85, 1.15);
        let (a, b) = self.band();
        let x = self.rng.range(a, b);
        let mut f = Flake {
            x,
            y: 0.0,
            vx: self.wind_now,
            vy: fall,
            r: 0.0,
            depth,
            alpha,
            kind,
            sprite,
            angle: self.rng.range(0.0, std::f32::consts::TAU),
            spin: self.rng.range(-1.6, 1.6),
            phase: self.rng.range(0.0, std::f32::consts::TAU),
            phase_rate,
            flutter,
            squash: 1.0,
            fall,
            tau,
            sheltered: false,
        };
        f.r = self.radius_for(depth, kind);
        f.y = -f.r * 3.0 - self.rng.range(0.0, 0.08 * self.height);
        f
    }

    /// Rebuild the turbulence grid: velocity = curl of a drifting noise stream function.
    fn update_air(&mut self) {
        let s = self.scale;
        let step = 64.0 * s;
        let (a, b) = self.band();
        let nx = (((b - a) / step).ceil() as usize + 3).min(400);
        let ny = ((self.height / step).ceil() as usize + 3).min(200);
        let air = &mut self.air;
        air.step = step;
        air.x0 = a - step;
        air.nx = nx;
        air.ny = ny;
        air.psi.resize(nx * ny, 0.0);
        air.u.resize(nx * ny, (0.0, 0.0));
        // Eddy size ~ 260 logical px; they evolve slowly and ride on the wind.
        let l = 260.0 * s;
        let t = self.time * 0.11;
        let blizzard = self.weather.intensity == Intensity::Blizzard;
        let amp = 16.0 + 0.45 * self.wind_now.abs() + if blizzard { 45.0 } else { 0.0 };
        for j in 0..ny {
            for i in 0..nx {
                let x = air.x0 + i as f32 * step - self.advect * s;
                let y = j as f32 * step;
                let p = noise3(x / l, y / l, t, 7) + 0.5 * noise3(x / (l * 0.45), y / (l * 0.45), t * 1.7, 13);
                air.psi[j * nx + i] = p;
            }
        }
        // u = amp * L * (dψ/dy, -dψ/dx): divergence-free swirls of speed ~amp.
        let k = amp * l / (2.0 * step);
        for j in 0..ny {
            for i in 0..nx {
                let p = |ii: usize, jj: usize| air.psi[jj.min(ny - 1) * nx + ii.min(nx - 1)];
                let dpx = p(i + 1, j) - p(i.saturating_sub(1), j);
                let dpy = p(i, j + 1) - p(i, j.saturating_sub(1));
                air.u[j * nx + i] = (k * dpy, -k * dpx * 0.6);
            }
        }
    }

    /// Mean wind plus gust fronts that roll across the screen in the wind's direction.
    fn wind_at(&self, x: f32) -> f32 {
        let s = self.scale;
        let front = ((x / s) - self.gust_advect) / 900.0;
        let g = noise3(front, 0.0, self.time * 0.05, 29) * 2.0 - 1.0;
        self.wind_now + g.max(-0.3) * self.gust_amp * self.weather.wind_dir
    }

    /// Advance the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.1);
        if dt == 0.0 {
            return;
        }
        self.time += dt;
        let blizzard = self.weather.intensity == Intensity::Blizzard;
        let s = self.scale;
        let (w, h) = (self.width, self.height);

        let base = self.weather.wind.speed() + if blizzard { 90.0 } else { 0.0 };
        let target = base * self.weather.wind_dir;
        self.wind_now += (target - self.wind_now) * (dt * 0.8).min(1.0);
        let gust_target = match (self.weather.wind, blizzard) {
            (_, true) => 170.0,
            (Wind::Calm, _) => 10.0,
            (Wind::Breeze, _) => 45.0,
            (Wind::Strong, _) => 110.0,
        };
        self.gust_amp += (gust_target - self.gust_amp) * (dt * 0.5).min(1.0);
        self.advect += self.wind_now * dt;
        // Fronts move with the mean flow felt by flakes (wind plus average gust), so
        // flakes don't bunch up behind slower fronts.
        self.gust_advect += (self.wind_now + 0.33 * self.gust_amp * self.weather.wind_dir) * dt;
        self.update_air();

        // Cursor wake: the air along the mouse's path is dragged with it.
        let wake = match (self.prev_cursor, self.cursor) {
            (Some(a), Some(b)) => {
                let (vx, vy) = (self.cursor_v.0 / s, self.cursor_v.1 / s);
                let speed = (vx * vx + vy * vy).sqrt();
                (speed > 150.0).then_some((a, b, vx.clamp(-2500.0, 2500.0), vy.clamp(-2500.0, 2500.0)))
            }
            _ => None,
        };
        let wake_r = 60.0 * s;

        let fall_boost = if blizzard { 1.6 } else { 1.0 };
        let land_area = if blizzard { 1.3 } else { 1.0 };
        for idx in 0..self.flakes.len() {
            let mut f = self.flakes[idx];
            let prev_y = f.y;
            // Perspective: near things cross the screen faster.
            let p = 0.45 + 0.75 * f.depth;
            let turb = self.air.at(f.x, f.y);
            let wind = self.wind_at(f.x);
            f.phase += f.phase_rate * dt;
            let (sn, cs) = f.phase.sin_cos();
            // Flutter: a sideways glide that swings back and forth; flat-on flakes fall slower.
            let glide = f.flutter * sn;
            let tx = (wind + turb.0) * p + glide * p;
            let ty = (f.fall * fall_boost * (0.78 + 0.3 * sn.abs()) + turb.1) * p;
            let k = 1.0 - (-dt / f.tau).exp();
            f.vx += (tx - f.vx) * k;
            f.vy += (ty - f.vy) * k;
            if let Some((a, b, cvx, cvy)) = wake {
                let d = seg_dist(f.x, f.y, a.0, a.1, b.0, b.1);
                if d < wake_r {
                    let fo = (1.0 - d / wake_r).powi(2) * (dt * 14.0).min(1.0);
                    f.vx += (cvx * 0.7 - f.vx) * fo;
                    f.vy += (cvy * 0.7 - f.vy) * fo;
                    f.spin += (cvx.signum() * 6.0 - f.spin) * fo;
                }
            }
            f.x += f.vx * s * dt;
            f.y += f.vy * s * dt;
            // Tumbling: crystals turn edge-on and back; their spin follows the glide.
            f.squash = match f.kind {
                Kind::Crystal => 0.3 + 0.7 * cs.abs(),
                Kind::Fluff => 0.75 + 0.25 * cs.abs(),
                _ => 1.0,
            };
            f.spin += (glide * 0.04 - f.spin) * (dt * 0.5).min(1.0);
            f.angle += f.spin * dt;

            // Only nearer flakes land; far ones read as falling behind the windows.
            // A window is a roof: a flake that crosses its top edge either settles
            // there or (pile full) carries on in front of it, never reaching the
            // piles the window shelters.
            if f.depth > 0.3 && f.kind != Kind::Bokeh && !f.sheltered {
                for pile in &mut self.piles {
                    if let Some(top) = pile.surface(f.x) {
                        if prev_y < top && f.y >= top {
                            if pile.is_full_at(f.x) {
                                f.sheltered = true;
                            } else {
                                // One flake is a little snow: piles build up over minutes.
                                let m = f.r.min(4.0 * s);
                                pile.deposit(f.x, std::f32::consts::PI * m * m * land_area * DEPOSIT);
                                // Mark as gone; it is recycled below.
                                f.y = h + 10_000.0;
                            }
                            break;
                        }
                    }
                }
            }
            self.flakes[idx] = f;
        }

        self.step_piles(dt);
        self.step_puffs(dt);

        // Recycle flakes that left the screen; keep the count at the target.
        let target = self.target_count();
        let (band_a, band_b) = self.band();
        let wind = self.wind_now;
        let (lo, hi) = if wind >= 0.0 { (band_a - 0.2 * w, w + 0.1 * w) } else { (-0.1 * w, band_b + 0.2 * w) };
        let mut i = 0;
        while i < self.flakes.len() {
            let f = self.flakes[i];
            let gone = f.y - f.r * 3.0 > h || f.x < lo || f.x > hi || f.y < -0.5 * h;
            if gone {
                if self.flakes.len() > target {
                    self.flakes.swap_remove(i);
                    continue;
                }
                self.flakes[i] = self.spawn();
            }
            i += 1;
        }
        // Grow gradually when intensity goes up, so the change looks like weather, not a switch.
        let missing = target.saturating_sub(self.flakes.len());
        let add = missing.min(((target as f32) * dt * 0.5).ceil() as usize + 1);
        for _ in 0..add {
            let f = self.spawn();
            self.flakes.push(f);
        }
    }

    /// Settling, edge spill, wind erosion and brushing for every pile.
    fn step_piles(&mut self, dt: f32) {
        let s = self.scale;
        let wind = self.wind_now + self.gust_amp * 0.3 * self.weather.wind_dir;
        for pi in 0..self.piles.len() {
            self.piles[pi].relax();

            // Overflow at the rounded ends drops off the corner as small clumps.
            for left in [true, false] {
                let p = &self.piles[pi];
                let spill = if left { p.spill_left } else { p.spill_right };
                let need = clump_area(&mut self.rng, s);
                if spill < need {
                    continue;
                }
                let x = if left { p.x0 + p.cell } else { p.x1 - p.cell };
                if !p.is_visible(x) {
                    let p = &mut self.piles[pi];
                    if left { p.spill_left = 0.0 } else { p.spill_right = 0.0 }
                    continue;
                }
                let y = p.y - 1.5 * s;
                let out = if left { -1.0 } else { 1.0 };
                let v = (out * self.rng.range(10.0, 45.0) * s, self.rng.range(-5.0, 15.0) * s);
                let puff = self.clump(x, y, v, need);
                self.puffs.push(puff);
                let p = &mut self.piles[pi];
                if left { p.spill_left -= need } else { p.spill_right -= need }
            }

            // Strong wind strips powder off the top and carries it away.
            let excess = (wind.abs() - 45.0).max(0.0);
            if excess > 0.0 {
                let cells = self.piles[pi].h.len() as f32;
                let rate = excess / 50.0 * cells / 30.0;
                let mut events = (rate * dt) as usize + if self.rng.f() < (rate * dt).fract() { 1 } else { 0 };
                while events > 0 {
                    events -= 1;
                    let depth = 0.35 * s;
                    if let Some((x, y, _)) = self.piles[pi].erode(&mut self.rng, depth) {
                        let n = 1 + (self.rng.f() * 2.0) as usize;
                        for _ in 0..n {
                            let r = self.rng.range(0.9, 1.6) * s;
                            self.puffs.push(Puff {
                                x: x + self.rng.range(-2.0, 2.0) * s,
                                y,
                                vx: wind * s * self.rng.range(0.3, 0.7),
                                vy: -self.rng.range(15.0, 70.0) * s,
                                r,
                                alpha: self.rng.range(0.55, 0.85),
                                sprite: sprite::DISC[1],
                                angle: 0.0,
                                spin: 0.0,
                                tau: self.rng.range(0.15, 0.3),
                                age: 0.0,
                            });
                        }
                    }
                }
            }
        }
        self.brush();
        let max = 6000;
        if self.puffs.len() > max {
            let extra = self.puffs.len() - max;
            self.puffs.drain(0..extra);
        }
    }

    fn clump(&mut self, x: f32, y: f32, v: (f32, f32), area: f32) -> Puff {
        Puff {
            x,
            y,
            vx: v.0,
            vy: v.1,
            r: (area / std::f32::consts::PI).sqrt(),
            alpha: 1.0,
            sprite: sprite::CLUMP[((self.rng.f() * 8.0) as usize).min(7)],
            angle: self.rng.range(0.0, 6.283),
            spin: self.rng.range(-3.0, 3.0),
            tau: 2.5,
            age: 0.0,
        }
    }

    /// A fast swipe through a pile throws its snow in the direction of the swipe.
    fn brush(&mut self) {
        let Some((cx, cy)) = self.cursor else { return };
        let (vx, vy) = self.cursor_v;
        let s = self.scale;
        if (vx * vx + vy * vy).sqrt() < 600.0 * s {
            return;
        }
        let radius = 22.0 * s;
        for pi in 0..self.piles.len() {
            let p = &self.piles[pi];
            if cx < p.x0 - radius || cx > p.x1 + radius || cy < p.y - p.max_h - 14.0 * s || cy > p.y + 10.0 * s {
                continue;
            }
            let removed = self.piles[pi].brush(cx, radius);
            for (x, top, area) in removed {
                // Split the removed snow into a spray of clumps and powder.
                let mut left = area;
                while left > 2.0 * s * s {
                    let a = clump_area(&mut self.rng, s).min(left);
                    left -= a;
                    let k = self.rng.range(0.25, 0.65);
                    let v = (
                        vx.clamp(-3500.0 * s, 3500.0 * s) * k + self.rng.range(-60.0, 60.0) * s,
                        -self.rng.range(120.0, 380.0) * s + vy.clamp(-2000.0 * s, 2000.0 * s) * 0.2,
                    );
                    let (cx, cy) = (x + self.rng.range(-1.5, 1.5) * s, top + self.rng.range(0.0, 4.0) * s);
                    let mut c = self.clump(cx, cy, v, a);
                    c.spin = vx.signum() * self.rng.range(4.0, 12.0);
                    self.puffs.push(c);
                    // Fine powder that hangs in the air a moment.
                    if self.rng.f() < 0.6 {
                        self.puffs.push(Puff {
                            x: c.x,
                            y: c.y,
                            vx: v.0 * 0.6,
                            vy: v.1 * 0.5,
                            r: self.rng.range(0.8, 1.4) * s,
                            alpha: 0.7,
                            sprite: sprite::DISC[1],
                            angle: 0.0,
                            spin: 0.0,
                            tau: 0.25,
                            age: 0.0,
                        });
                    }
                }
            }
        }
    }

    /// Gravity, air drag and landing for loose clumps and powder.
    fn step_puffs(&mut self, dt: f32) {
        let s = self.scale;
        let g = 900.0 * s;
        let h = self.height;
        let mut i = 0;
        while i < self.puffs.len() {
            let mut p = self.puffs[i];
            let prev_y = p.y;
            p.age += dt;
            // Air drag pulls the clump toward the local air velocity (physical px/s);
            // heavy clumps barely notice, powder is carried away.
            let turb = self.air.at(p.x, p.y);
            let air = ((self.wind_at(p.x) + turb.0) * s, turb.1 * s);
            let k = 1.0 - (-dt / p.tau).exp();
            p.vx += (air.0 - p.vx) * k;
            p.vy += (air.1 - p.vy) * k;
            // Powder falls gently (high drag); clumps accelerate properly.
            let gravity = if p.tau < 0.5 { g * 0.12 } else { g };
            p.vy = (p.vy + gravity * dt).min(1400.0 * s);
            p.x += p.vx * dt;
            p.y += p.vy * dt;
            p.angle += p.spin * dt;
            if p.tau < 0.5 {
                // Powder fades as it disperses.
                p.alpha -= dt * 0.35;
            }
            let mut gone = p.y - p.r * 3.0 > h || p.alpha <= 0.02;
            if !gone && p.vy > 0.0 && p.age > 0.12 {
                for pile in &mut self.piles {
                    if let Some(top) = pile.surface(p.x) {
                        if prev_y < top && p.y >= top && !pile.is_full_at(p.x) {
                            pile.deposit(p.x, std::f32::consts::PI * p.r * p.r * p.alpha.min(1.0));
                            gone = true;
                            break;
                        }
                    }
                }
            }
            if gone {
                self.puffs.swap_remove(i);
            } else {
                self.puffs[i] = p;
                i += 1;
            }
        }
    }
}

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_tracks_intensity_and_area() {
        let mut w = Weather { wind: Wind::Calm, ..Weather::default() };
        let s = Snow::new(1920.0, 1080.0, 1.0, w, 7);
        let normal = s.target_count();
        assert_eq!(s.flakes.len(), normal);
        w.intensity = Intensity::Blizzard;
        let b = Snow::new(1920.0, 1080.0, 1.0, w, 7);
        assert!(b.target_count() > normal * 3);
        let big = Snow::new(3840.0, 2160.0, 1.0, Weather { wind: Wind::Calm, ..Weather::default() }, 7);
        assert!(big.target_count() > normal * 3);
    }

    #[test]
    fn flakes_fall_and_stay_bounded() {
        let mut s = Snow::new(1280.0, 720.0, 1.0, Weather::default(), 42);
        for _ in 0..120 {
            s.step(1.0 / 60.0);
        }
        let before: Vec<f32> = s.flakes.iter().map(|f| f.y).collect();
        s.step(1.0 / 60.0);
        let down = s.flakes.iter().zip(&before).filter(|(f, y)| f.y > **y).count();
        assert!(down * 100 > before.len() * 90, "almost every flake should move down each frame ({down}/{})", before.len());
        for _ in 0..60 * 120 {
            s.step(1.0 / 60.0);
        }
        let t = s.target_count();
        assert!(s.flakes.len() <= t + 1);
        assert!(s.flakes.len() >= t * 9 / 10);
        for f in &s.flakes {
            assert!(f.x.is_finite() && f.y.is_finite() && f.vx.is_finite());
            assert!(f.y < 720.0 + 200.0 && f.x > -4000.0 && f.x < 5000.0);
        }
    }

    #[test]
    fn turbulence_makes_paths_differ_and_flakes_have_inertia() {
        let w = Weather { wind: Wind::Calm, ..Weather::default() };
        let mut s = Snow::new(1600.0, 900.0, 1.0, w, 3);
        for _ in 0..300 {
            s.step(1.0 / 60.0);
        }
        // In calm air, eddies still push flakes sideways, in both directions.
        let vx: Vec<f32> = s.flakes.iter().map(|f| f.vx).collect();
        assert!(vx.iter().any(|&v| v > 5.0) && vx.iter().any(|&v| v < -5.0));
        // A sudden gust is not followed instantly (inertia).
        let f = s.flakes[0];
        let k = 1.0 - (-(1.0f32 / 60.0) / f.tau).exp();
        assert!(k < 0.2);
    }

    #[test]
    fn snow_is_even_across_the_screen_in_wind() {
        for wind in [Wind::Breeze, Wind::Strong] {
            for (dir, seed) in [(1.0, 11), (-1.0, 11), (1.0, 12), (-1.0, 12)] {
                let w = Weather { intensity: Intensity::Heavy, wind, wind_dir: dir, ..Weather::default() };
                let mut s = Snow::new(1600.0, 900.0, 1.0, w, seed);
                let mut thirds = [0usize; 3];
                for k in 0..60 * 90 {
                    s.step(1.0 / 60.0);
                    if k > 60 * 30 && k % 30 == 0 {
                        for f in &s.flakes {
                            if f.x >= 0.0 && f.x < 1600.0 && f.y > 0.0 && f.y < 900.0 {
                                thirds[(f.x / 1600.0 * 3.0) as usize] += 1;
                            }
                        }
                    }
                }
                let (mn, mx) = (*thirds.iter().min().unwrap() as f32, *thirds.iter().max().unwrap() as f32);
                assert!(mn / mx > 0.8, "{wind:?} {dir}: uneven {thirds:?}");
            }
        }
    }

    #[test]
    fn snow_builds_up_gradually_and_windows_shelter_whats_below() {
        let w = Weather { wind: Wind::Calm, ..Weather::default() };
        let mut s = Snow::new(1000.0, 800.0, 1.0, w, 5);
        let ledges = [
            // A window whose top edge is at y=300, spanning the left half.
            Ledge { id: 1, x0: 0.0, x1: 500.0, y: 300.0, visible: vec![(0.0, 500.0)], max_h: 12.0 },
            // The taskbar below everything.
            Ledge { id: 2, x0: 0.0, x1: 1000.0, y: 700.0, visible: vec![(0.0, 1000.0)], max_h: 16.0 },
        ];
        s.set_ledges(&ledges, 0.1);
        // After 20 s: some snow, but in patches, not a full-width layer.
        for _ in 0..60 * 20 {
            s.step(1.0 / 60.0);
        }
        let top = &s.piles[0];
        let covered = top.h.iter().filter(|&&h| h >= crate::pile::MIN_VISIBLE).count();
        assert!(covered > 0, "snow has started to settle");
        assert!(covered < top.h.len() * 6 / 10, "but not as a full layer yet ({covered}/{})", top.h.len());
        // After several minutes the window is snowed in, within its limit.
        for _ in 0..60 * 400 {
            s.step(1.0 / 60.0);
        }
        let top = &s.piles[0];
        let max = top.h.iter().cloned().fold(0.0, f32::max);
        assert!(max > 9.0 && max <= 12.0 + 1e-3, "fills up to the limit and no further ({max})");
        // The taskbar under the window stays nearly bare; the open half is snowy.
        let low = &s.piles[1];
        let n = low.h.len();
        let under: f32 = low.h[n / 10..n * 4 / 10].iter().sum::<f32>();
        let open: f32 = low.h[n * 6 / 10..n * 9 / 10].iter().sum::<f32>();
        assert!(under < open * 0.25, "sheltered {under} vs open {open}");
    }

    #[test]
    fn clearing_knocks_all_snow_off() {
        let w = Weather { wind: Wind::Calm, ..Weather::default() };
        let mut s = Snow::new(1000.0, 800.0, 1.0, w, 6);
        s.set_ledges(&[Ledge { id: 1, x0: 0.0, x1: 500.0, y: 300.0, visible: vec![(0.0, 500.0)], max_h: 12.0 }], 0.1);
        for k in 0..2000 {
            s.piles[0].deposit((k % 500) as f32, 30.0);
        }
        s.clear_piles();
        assert!(s.piles[0].h.iter().all(|&h| h == 0.0));
        assert!(!s.puffs.is_empty(), "the snow falls off as clumps");
    }

    #[test]
    fn dragged_window_flings_snow_that_lands_below() {
        let w = Weather { intensity: Intensity::Light, wind: Wind::Calm, ..Weather::default() };
        let mut s = Snow::new(1600.0, 900.0, 1.0, w, 8);
        let a = Ledge { id: 1, x0: 200.0, x1: 600.0, y: 200.0, visible: vec![(200.0, 600.0)], max_h: 12.0 };
        let floor = Ledge { id: 2, x0: 0.0, x1: 1600.0, y: 860.0, visible: vec![(0.0, 1600.0)], max_h: 22.0 };
        s.set_ledges(&[a.clone(), floor.clone()], 0.1);
        for k in 0..400 {
            s.piles[0].deposit(200.0 + (k % 400) as f32, 30.0);
            s.piles[0].relax();
        }
        let floor_before = s.piles.iter().find(|p| p.id == 2).unwrap().amount();
        // Drag the window 60 px to the right within one scan.
        let moved = Ledge { x0: 260.0, x1: 660.0, visible: vec![(260.0, 660.0)], ..a };
        s.set_ledges(&[moved, floor], 0.1);
        assert!(!s.puffs.is_empty());
        let mean_vx = s.puffs.iter().map(|p| p.vx).sum::<f32>() / s.puffs.len() as f32;
        assert!(mean_vx > 100.0, "snow keeps the window's momentum ({mean_vx})");
        for _ in 0..240 {
            s.step(1.0 / 60.0);
        }
        let floor_after = s.piles.iter().find(|p| p.id == 2).unwrap().amount();
        assert!(floor_after > floor_before + 1000.0, "flung snow lands on the taskbar");
    }

    #[test]
    fn fast_swipe_clears_a_pile_and_throws_snow_forward() {
        let w = Weather { intensity: Intensity::Light, wind: Wind::Calm, ..Weather::default() };
        let mut s = Snow::new(1600.0, 900.0, 1.0, w, 9);
        let a = Ledge { id: 1, x0: 200.0, x1: 1000.0, y: 300.0, visible: vec![(200.0, 1000.0)], max_h: 12.0 };
        s.set_ledges(&[a], 0.1);
        for k in 0..3000 {
            s.piles[0].deposit(200.0 + (k % 800) as f32, 30.0);
            s.piles[0].relax();
        }
        let before = s.piles[0].amount();
        for k in 0..20 {
            s.set_cursor(Some((300.0 + k as f32 * 25.0, 292.0)), 1.0 / 60.0);
            s.step(1.0 / 60.0);
        }
        let after = s.piles[0].amount();
        assert!(after < before * 0.75, "{after} vs {before}");
        let vx: f32 = s.puffs.iter().map(|p| p.vx).sum();
        assert!(vx > 0.0, "snow flies the way the cursor went");
    }

    #[test]
    fn strong_wind_blows_powder_off_piles() {
        let w = Weather { intensity: Intensity::Light, wind: Wind::Strong, ..Weather::default() };
        let mut s = Snow::new(1600.0, 900.0, 1.0, w, 10);
        let a = Ledge { id: 1, x0: 200.0, x1: 1000.0, y: 300.0, visible: vec![(200.0, 1000.0)], max_h: 12.0 };
        s.set_ledges(&[a], 0.1);
        for k in 0..3000 {
            s.piles[0].deposit(200.0 + (k % 800) as f32, 30.0);
            s.piles[0].relax();
        }
        let mut lifted = 0;
        for _ in 0..120 {
            s.step(1.0 / 60.0);
            lifted = lifted.max(s.puffs.len());
        }
        assert!(lifted > 5, "spindrift appears in strong wind ({lifted})");
    }

    #[test]
    fn keys_round_trip() {
        for v in Intensity::ALL {
            assert_eq!(Intensity::from_key(v.key()), Some(v));
        }
        for v in Wind::ALL {
            assert_eq!(Wind::from_key(v.key()), Some(v));
        }
        for v in FlakeSize::ALL {
            assert_eq!(FlakeSize::from_key(v.key()), Some(v));
        }
    }
}
