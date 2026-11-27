//! Visitors on the snow: penguins that waddle along the surfaces (and fall off
//! window edges onto whatever is below), and a snowplow that drives along the
//! taskbar pushing the snow ahead of its blade. Pure simulation, no platform code.

use crate::art::sprite;
use crate::pile::{clump_area, Pile, Puff};
use crate::snow::Rng;

/// Rendered penguin height in logical pixels.
const PENGUIN_H: f32 = 26.0;
/// The penguin fills this many of its 128-px atlas cell vertically.
const PENGUIN_CELL_FILL: f32 = 105.0;
/// Rendered snowplow length (cab to blade tip) in logical pixels.
const PLOW_LEN: f32 = 66.0;
/// The plow fills this many of its 128-px atlas cell horizontally.
const PLOW_CELL_FILL: f32 = 108.0;

/// One sprite to draw. `half` is half the quad size in px; `sx` < 0 mirrors.
#[derive(Clone, Copy, Debug)]
pub struct Sprite {
    pub id: u16,
    pub x: f32,
    pub y: f32,
    pub half: f32,
    pub angle: f32,
    pub sx: f32,
    pub alpha: f32,
    pub tint: [f32; 3],
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum PState {
    Walk,
    Idle,
    Slide,
    Fall,
}

pub struct Penguin {
    pub x: f32,
    /// Feet position (bottom of the penguin).
    pub y: f32,
    vx: f32,
    vy: f32,
    dir: f32,
    speed: f32,
    state: PState,
    timer: f32,
    /// Seconds left before it heads for the exit (on the taskbar).
    life: f32,
    phase: f32,
    /// Pile it stands on (by id), if any.
    on: Option<u64>,
    scarf: [f32; 3],
    leaving: bool,
}

pub struct Plow {
    pub x: f32,
    dir: f32,
    pile: u64,
    front: f32,
    wheel: f32,
    smoke_t: f32,
    pub done: bool,
}

/// A puff of exhaust: rises, spreads, fades.
#[derive(Clone, Copy)]
pub struct Smoke {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    r: f32,
    age: f32,
}

pub struct Visitors {
    pub penguins: Vec<Penguin>,
    pub plows: Vec<Plow>,
    smoke: Vec<Smoke>,
    pub penguins_on: bool,
    pub plows_on: bool,
    next_penguin: f32,
    next_plow: f32,
    want_penguin: bool,
    want_plow: bool,
    time: f32,
}

const SCARVES: [[f32; 3]; 5] =
    [[0.86, 0.16, 0.18], [0.16, 0.45, 0.85], [0.1, 0.62, 0.35], [0.98, 0.72, 0.1], [0.62, 0.25, 0.75]];

/// Index of the "floor": the lowest wide pile (normally the taskbar).
fn floor_index(piles: &[Pile], width: f32) -> Option<usize> {
    piles
        .iter()
        .enumerate()
        .filter(|(_, p)| p.x1 - p.x0 >= width * 0.5)
        .max_by(|a, b| a.1.y.total_cmp(&b.1.y))
        .map(|(i, _)| i)
}

fn surface_at(p: &Pile, x: f32) -> f32 {
    match p.cell_at(x) {
        Some(i) => p.y - p.h[i],
        None => p.y,
    }
}

impl Default for Visitors {
    fn default() -> Self {
        Self::new()
    }
}

impl Visitors {
    pub fn new() -> Self {
        Self {
            penguins: Vec::new(),
            plows: Vec::new(),
            smoke: Vec::new(),
            penguins_on: true,
            plows_on: true,
            next_penguin: 20.0,
            next_plow: 45.0,
            want_penguin: false,
            want_plow: false,
            time: 0.0,
        }
    }

    pub fn summon_penguin(&mut self) {
        self.want_penguin = true;
    }

    pub fn summon_plow(&mut self) {
        self.want_plow = true;
    }

    fn spawn_penguin(&mut self, piles: &[Pile], rng: &mut Rng, scale: f32, width: f32) -> bool {
        let floor = floor_index(piles, width);
        // Mostly along the taskbar; sometimes out from behind on a window top.
        let windows: Vec<usize> = (0..piles.len())
            .filter(|&i| Some(i) != floor && piles[i].visible.iter().any(|(a, b)| b - a >= 160.0 * scale))
            .collect();
        let use_window = !windows.is_empty() && (floor.is_none() || rng.f() < 0.35);
        let dir = if rng.f() < 0.5 { 1.0 } else { -1.0 };
        let (pile, x) = if use_window {
            let i = windows[((rng.f() * windows.len() as f32) as usize).min(windows.len() - 1)];
            let spans: Vec<&(f32, f32)> = piles[i].visible.iter().filter(|(a, b)| b - a >= 160.0 * scale).collect();
            let &(a, b) = spans[((rng.f() * spans.len() as f32) as usize).min(spans.len() - 1)];
            // Step out from the end of the visible stretch, walking inward.
            (i, if dir > 0.0 { a + 6.0 * scale } else { b - 6.0 * scale })
        } else if let Some(i) = floor {
            let p = &piles[i];
            (i, if dir > 0.0 { p.x0 - 20.0 * scale } else { p.x1 + 20.0 * scale })
        } else {
            return false;
        };
        let p = &piles[pile];
        self.penguins.push(Penguin {
            x,
            y: surface_at(p, x),
            vx: 0.0,
            vy: 0.0,
            dir,
            speed: 22.0 * scale * rng.range(0.8, 1.2),
            state: PState::Walk,
            timer: rng.range(3.0, 8.0),
            life: rng.range(25.0, 60.0),
            phase: 0.0,
            on: Some(p.id),
            scarf: SCARVES[((rng.f() * SCARVES.len() as f32) as usize).min(SCARVES.len() - 1)],
            leaving: false,
        });
        true
    }

    fn spawn_plow(&mut self, piles: &[Pile], rng: &mut Rng, scale: f32, width: f32) -> bool {
        let Some(i) = floor_index(piles, width) else { return false };
        let p = &piles[i];
        let dir = if rng.f() < 0.5 { 1.0 } else { -1.0 };
        let half = PLOW_LEN * scale;
        let x = if dir > 0.0 { p.x0 - half } else { p.x1 + half };
        let front = x + dir * self.blade_offset(scale);
        self.plows.push(Plow { x, dir, pile: p.id, front, wheel: 0.0, smoke_t: 0.0, done: false });
        true
    }

    /// Quad size of the plow sprite in px.
    fn plow_quad(scale: f32) -> f32 {
        PLOW_LEN * scale * 128.0 / PLOW_CELL_FILL
    }

    /// Distance from the plow's centre to its blade tip.
    fn blade_offset(&self, scale: f32) -> f32 {
        (124.0 - 64.0) / 128.0 * Self::plow_quad(scale)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        dt: f32,
        piles: &mut [Pile],
        puffs: &mut Vec<Puff>,
        rng: &mut Rng,
        scale: f32,
        width: f32,
        height: f32,
    ) {
        self.time += dt;
        let s = scale;

        // Arrivals.
        self.next_penguin -= dt;
        if self.want_penguin || (self.penguins_on && self.next_penguin <= 0.0 && self.penguins.len() < 3) {
            if self.spawn_penguin(piles, rng, s, width) {
                self.want_penguin = false;
            }
            self.next_penguin = rng.range(35.0, 80.0);
        }
        self.next_plow -= dt;
        if self.want_plow || (self.plows_on && self.next_plow <= 0.0 && self.plows.is_empty()) {
            // Only bother when there is a fair bit of snow to clear (unless summoned).
            let worth = floor_index(piles, width).is_some_and(|i| {
                let p = &piles[i];
                p.amount() > 0.25 * (p.x1 - p.x0) * p.max_h * 0.5
            });
            if self.want_plow || worth {
                if self.spawn_plow(piles, rng, s, width) {
                    self.want_plow = false;
                }
                self.next_plow = rng.range(70.0, 140.0);
            } else {
                self.next_plow = 10.0;
            }
        }

        for p in piles.iter_mut() {
            p.boost = None;
            p.wall = None;
        }
        self.step_plows(dt, piles, puffs, rng, s);
        self.step_penguins(dt, piles, puffs, rng, s, width, height);

        // Exhaust.
        for m in &mut self.smoke {
            m.age += dt;
            m.x += m.vx * dt;
            m.y += m.vy * dt;
            m.vy *= (-0.8 * dt).exp();
            m.r += 6.0 * s * dt;
        }
        self.smoke.retain(|m| m.age < 1.6);
    }

    fn step_plows(&mut self, dt: f32, piles: &mut [Pile], puffs: &mut Vec<Puff>, rng: &mut Rng, s: f32) {
        let quad = Self::plow_quad(s);
        let blade = self.blade_offset(s);
        let speed = 50.0 * s;
        for plow in &mut self.plows {
            let Some(p) = piles.iter_mut().find(|p| p.id == plow.pile) else {
                plow.done = true;
                continue;
            };
            plow.x += plow.dir * speed * dt;
            let front = plow.x + plow.dir * blade;
            // Blade sweeps from last frame's tip to this frame's; a little extra
            // behind it so nothing that slid back stays under the truck.
            let back = plow.front - plow.dir * 3.0 * p.cell;
            let lost = p.plow_push(back, front);
            plow.front = front;
            // The bank ahead of the blade may stand tall.
            if front >= p.x0 && front < p.x1 {
                p.wall = p.cell_at(front);
            }
            if let Some(i) = p.cell_at(front.clamp(p.x0, p.x1 - 0.01)) {
                let n = p.h.len();
                let reach = (40.0 * s / p.cell) as usize;
                p.boost = Some(if plow.dir > 0.0 { (i, (i + reach).min(n - 1)) } else { (i.saturating_sub(reach), i) });
            }
            // Snow pushed over the end of the taskbar tumbles off.
            let mut off = lost;
            while off > 0.0 {
                let a = clump_area(rng, s).min(off);
                off -= a;
                if a < 2.0 * s * s {
                    break;
                }
                let x = if plow.dir > 0.0 { p.x1 } else { p.x0 };
                let y = p.y - rng.range(2.0, 10.0) * s;
                let v = (plow.dir * rng.range(40.0, 110.0) * s, rng.range(-60.0, 10.0) * s);
                puffs.push(clump(rng, x, y, v, a));
            }
            // What the bank can't hold is thrown up over the blade.
            while p.boost_excess > 0.0 {
                let a = clump_area(rng, s).min(p.boost_excess);
                p.boost_excess -= a;
                if a < 1.0 * s * s {
                    p.boost_excess = 0.0;
                    break;
                }
                let x = front + plow.dir * rng.range(2.0, 14.0) * s;
                let y = surface_at(p, x) - rng.range(0.0, 4.0) * s;
                let v = (plow.dir * rng.range(60.0, 160.0) * s, -rng.range(120.0, 260.0) * s);
                puffs.push(clump(rng, x, y, v, a));
            }
            plow.wheel += plow.dir * speed * dt / (14.0 / 128.0 * quad);
            plow.smoke_t -= dt;
            if plow.smoke_t <= 0.0 {
                plow.smoke_t = rng.range(0.1, 0.2);
                let (ex, ey) = plow_point(plow, s, 28.0, 22.0, p.y);
                self.smoke.push(Smoke {
                    x: ex,
                    y: ey,
                    vx: -plow.dir * rng.range(5.0, 20.0) * s,
                    vy: -rng.range(25.0, 45.0) * s,
                    r: 2.0 * s,
                    age: 0.0,
                });
            }
            let gone = if plow.dir > 0.0 { plow.x - quad > p.x1 } else { plow.x + quad < p.x0 };
            if gone {
                plow.done = true;
            }
        }
        self.plows.retain(|p| !p.done);
    }

    #[allow(clippy::too_many_arguments)]
    fn step_penguins(
        &mut self,
        dt: f32,
        piles: &mut [Pile],
        puffs: &mut Vec<Puff>,
        rng: &mut Rng,
        s: f32,
        width: f32,
        height: f32,
    ) {
        let floor_id = floor_index(piles, width).map(|i| piles[i].id);
        let g = 900.0 * s;
        let plows: Vec<(u64, f32, f32)> = self.plows.iter().map(|p| (p.pile, p.x, p.dir)).collect();
        for pg in &mut self.penguins {
            let pile_i = pg.on.and_then(|id| piles.iter().position(|p| p.id == id));
            if pg.state != PState::Fall && pile_i.is_none() {
                // The window it stood on moved or closed.
                pg.state = PState::Fall;
                pg.vx = pg.dir * pg.speed * 0.5;
                pg.vy = 0.0;
                pg.on = None;
            }
            pg.timer -= dt;
            pg.life -= dt;
            let on_floor = pg.on.is_some() && pg.on == floor_id;
            if on_floor && pg.life <= 0.0 && !pg.leaving {
                // Time to go: head for the nearer end of the taskbar.
                let p = &piles[pile_i.unwrap()];
                pg.dir = if pg.x - p.x0 < p.x1 - pg.x { -1.0 } else { 1.0 };
                pg.leaving = true;
                pg.state = PState::Walk;
            }
            match pg.state {
                PState::Walk => {
                    let i = pile_i.unwrap();
                    // Step aside for a plow coming this way: turn and walk ahead of it.
                    for &(pid, px, pdir) in &plows {
                        if Some(pid) == pg.on && (px - pg.x) * pdir < 0.0 && (px - pg.x).abs() < 120.0 * s {
                            pg.dir = pdir;
                            pg.leaving = true;
                        }
                    }
                    let before = pg.phase;
                    pg.phase += dt * 7.0;
                    let nx = pg.x + pg.dir * pg.speed * dt;
                    let p = &mut piles[i];
                    let outside = nx < p.x0 || nx >= p.x1;
                    if outside && Some(p.id) == floor_id {
                        // Off the end of the taskbar: keep walking off screen.
                        pg.x = nx;
                        pg.y = p.y;
                    } else if outside {
                        // Off the edge of a window: tumble down.
                        pg.state = PState::Fall;
                        pg.vx = pg.dir * pg.speed;
                        pg.vy = -40.0 * s;
                        pg.on = None;
                        pg.x = nx;
                    } else if !p.is_visible(nx) {
                        // Another window is in the way: turn back.
                        pg.dir = -pg.dir;
                    } else {
                        pg.x = nx;
                        // A footprint at every step; then stand in it.
                        if (before / std::f32::consts::PI).floor() != (pg.phase / std::f32::consts::PI).floor() {
                            let foot = pg.x + pg.dir * if (pg.phase / std::f32::consts::PI) as i64 % 2 == 0 { 3.0 } else { -3.0 } * s;
                            p.press(foot, 2.5 * s, 0.75);
                        }
                        pg.y = surface_at(p, nx);
                    }
                    if pg.timer <= 0.0 && !pg.leaving {
                        let depth = p.cell_at(pg.x).map(|c| p.h[c]).unwrap_or(0.0);
                        let r = rng.f();
                        if depth > 3.0 * s && r < 0.3 {
                            pg.state = PState::Slide;
                            pg.vx = pg.dir * 75.0 * s;
                            pg.timer = 4.0;
                        } else if r < 0.75 {
                            pg.state = PState::Idle;
                            pg.timer = rng.range(1.5, 4.0);
                        } else {
                            pg.dir = -pg.dir;
                            pg.timer = rng.range(3.0, 8.0);
                        }
                    }
                }
                PState::Idle => {
                    pg.phase = 0.0;
                    if let Some(i) = pile_i {
                        pg.y = surface_at(&piles[i], pg.x);
                    }
                    // Looks around now and then.
                    if rng.f() < dt * 0.4 {
                        pg.dir = -pg.dir;
                    }
                    if pg.timer <= 0.0 {
                        pg.state = PState::Walk;
                        pg.timer = rng.range(3.0, 8.0);
                    }
                }
                PState::Slide => {
                    let i = pile_i.unwrap();
                    let p = &mut piles[i];
                    pg.vx *= (-0.6 * dt).exp();
                    let nx = pg.x + pg.vx * dt;
                    if nx < p.x0 || nx >= p.x1 || !p.is_visible(nx) {
                        // Bump the end: stand back up.
                        pg.state = PState::Idle;
                        pg.timer = 1.0;
                    } else {
                        pg.x = nx;
                        // The belly carves a shallow trough.
                        p.press(pg.x, 6.0 * s, 0.93);
                        pg.y = surface_at(p, nx);
                        if rng.f() < dt * 12.0 {
                            let v = (-pg.vx * 0.2, -rng.range(20.0, 60.0) * s);
                            puffs.push(powder(rng, pg.x - pg.dir * 6.0 * s, pg.y - 1.0 * s, v, s));
                        }
                    }
                    if pg.vx.abs() < 15.0 * s || pg.timer <= 0.0 {
                        pg.state = PState::Walk;
                        pg.timer = rng.range(3.0, 8.0);
                    }
                }
                PState::Fall => {
                    let prev = pg.y;
                    pg.vy += g * dt;
                    pg.x += pg.vx * dt;
                    pg.y += pg.vy * dt;
                    // Land on the first visible surface it crosses.
                    for p in piles.iter_mut() {
                        if pg.x >= p.x0 && pg.x < p.x1 && p.is_visible(pg.x) {
                            let top = surface_at(p, pg.x);
                            if prev <= top && pg.y >= top && pg.vy > 0.0 {
                                pg.y = top;
                                pg.on = Some(p.id);
                                pg.state = PState::Idle;
                                pg.timer = rng.range(0.8, 1.6);
                                // A little poof of snow where it lands.
                                for _ in 0..6 {
                                    let v = (rng.range(-60.0, 60.0) * s, -rng.range(30.0, 90.0) * s);
                                    puffs.push(powder(rng, pg.x, top, v, s));
                                }
                                p.press(pg.x, 5.0 * s, 0.7);
                                break;
                            }
                        }
                    }
                }
            }
        }
        // Gone once off screen or fallen past the bottom.
        let margin = 60.0 * s;
        self.penguins.retain(|p| p.x > -margin && p.x < width + margin && p.y < height + margin);
    }

    /// Everything to draw, in back-to-front order.
    pub fn sprites(&self, piles: &[Pile], scale: f32, out: &mut Vec<Sprite>) {
        let s = scale;
        let white = [1.0, 1.0, 1.0];
        let visible = |id: Option<u64>, x: f32| {
            id.and_then(|id| piles.iter().find(|p| p.id == id)).is_none_or(|p| x < p.x0 || x >= p.x1 || p.is_visible(x))
        };
        for m in &self.smoke {
            let a = 0.45 * (1.0 - m.age / 1.6);
            out.push(Sprite { id: sprite::DISC[2], x: m.x, y: m.y, half: m.r / 0.3, angle: 0.0, sx: 1.0, alpha: a, tint: [0.62, 0.64, 0.68] });
        }
        for plow in &self.plows {
            let Some(p) = piles.iter().find(|p| p.id == plow.pile) else { continue };
            if !visible(Some(p.id), plow.x) {
                continue;
            }
            let quad = Self::plow_quad(s);
            let cy = p.y - (112.0 - 64.0) / 128.0 * quad;
            out.push(Sprite { id: sprite::PLOW, x: plow.x, y: cy, half: quad / 2.0, angle: 0.0, sx: plow.dir, alpha: 1.0, tint: white });
            let wr = 14.0 / 128.0 * quad;
            for wx in [38.0, 86.0] {
                let (x, y) = plow_point(plow, s, wx, 98.0, p.y);
                out.push(Sprite { id: sprite::WHEEL, x, y, half: wr * 64.0 / 58.0, angle: plow.wheel, sx: 1.0, alpha: 1.0, tint: white });
            }
            // Amber beacon, blinking.
            let blink = ((self.time * 6.0).sin() * 0.5 + 0.5).powi(3);
            let (bx, by) = plow_point(plow, s, 48.0, 30.0, p.y);
            out.push(Sprite { id: sprite::DISC[2], x: bx, y: by, half: 7.0 * s / 0.3 * 0.5, angle: 0.0, sx: 1.0, alpha: 0.35 + 0.6 * blink, tint: [1.0, 0.62, 0.1] });
            out.push(Sprite { id: sprite::DISC[0], x: bx, y: by, half: 2.2 * s / 0.42, angle: 0.0, sx: 1.0, alpha: 1.0, tint: [1.0, 0.75, 0.25] });
        }
        for pg in &self.penguins {
            if !visible(pg.on, pg.x) {
                continue;
            }
            let quad = PENGUIN_H * s * 128.0 / PENGUIN_CELL_FILL;
            let foot = (121.0 - 64.0) / 128.0 * quad;
            let (pose, angle, bob) = match pg.state {
                PState::Walk => {
                    let sn = pg.phase.sin();
                    let pose = if sn > 0.35 { 1 } else if sn < -0.35 { 2 } else { 0 };
                    (pose, 0.12 * sn, -1.2 * s * sn.abs())
                }
                // Belly down, head first.
                PState::Slide => (0, pg.dir * 1.45, 0.0),
                PState::Fall => (0, pg.dir * 0.35, 0.0),
                PState::Idle => (0, 0.0, 0.0),
            };
            // Rotate about the feet, so the body tips over its feet.
            let (sa, ca) = angle.sin_cos();
            let (cx, cy) = if pg.state == PState::Slide {
                (pg.x, pg.y - 0.25 * quad)
            } else {
                (pg.x + sa * foot, pg.y - ca * foot + bob)
            };
            let id = sprite::PENGUIN[pose];
            out.push(Sprite { id, x: cx, y: cy, half: quad / 2.0, angle, sx: pg.dir, alpha: 1.0, tint: white });
            out.push(Sprite { id: sprite::SCARF, x: cx, y: cy, half: quad / 2.0, angle, sx: pg.dir, alpha: 1.0, tint: pg.scarf });
        }
    }
}

/// A point given in the plow sprite's cell coordinates (facing right), in world px.
fn plow_point(plow: &Plow, scale: f32, cx: f32, cy: f32, ground: f32) -> (f32, f32) {
    let quad = Visitors::plow_quad(scale);
    let centre_y = ground - (112.0 - 64.0) / 128.0 * quad;
    (plow.x + plow.dir * (cx - 64.0) / 128.0 * quad, centre_y + (cy - 64.0) / 128.0 * quad)
}

fn clump(rng: &mut Rng, x: f32, y: f32, v: (f32, f32), area: f32) -> Puff {
    Puff {
        x,
        y,
        vx: v.0,
        vy: v.1,
        r: (area / std::f32::consts::PI).sqrt(),
        alpha: 1.0,
        sprite: sprite::CLUMP[((rng.f() * 8.0) as usize).min(7)],
        angle: rng.range(0.0, 6.283),
        spin: rng.range(-6.0, 6.0),
        tau: 2.5,
        age: 0.0,
    }
}

fn powder(rng: &mut Rng, x: f32, y: f32, v: (f32, f32), s: f32) -> Puff {
    Puff {
        x,
        y,
        vx: v.0,
        vy: v.1,
        r: rng.range(0.8, 1.4) * s,
        alpha: 0.7,
        sprite: sprite::DISC[1],
        angle: 0.0,
        spin: 0.0,
        tau: 0.25,
        age: 0.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pile::Ledge;

    fn piles() -> Vec<Pile> {
        let floor = Ledge { id: 100, x0: 0.0, x1: 1600.0, y: 860.0, visible: vec![(0.0, 1600.0)], max_h: 16.0 };
        let win = Ledge { id: 1, x0: 400.0, x1: 900.0, y: 300.0, visible: vec![(400.0, 900.0)], max_h: 12.0 };
        let mut v = vec![Pile::new(&floor, 3.0), Pile::new(&win, 3.0)];
        for p in &mut v {
            for h in p.h.iter_mut() {
                *h = 8.0;
            }
        }
        v
    }

    fn run(v: &mut Visitors, piles: &mut [Pile], puffs: &mut Vec<Puff>, rng: &mut Rng, secs: f32) {
        for _ in 0..(secs * 60.0) as usize {
            v.step(1.0 / 60.0, piles, puffs, rng, 1.0, 1600.0, 900.0);
            for p in piles.iter_mut() {
                p.relax();
            }
        }
    }

    #[test]
    fn plow_clears_the_taskbar_and_pushes_the_snow_off_the_end() {
        let mut v = Visitors::new();
        v.penguins_on = false;
        let mut ps = piles();
        let before = ps[0].amount();
        let (mut puffs, mut rng) = (Vec::new(), Rng::new(1));
        v.summon_plow();
        run(&mut v, &mut ps, &mut puffs, &mut rng, 1.0);
        assert_eq!(v.plows.len(), 1);
        // Halfway: behind the plow is bare, ahead of it a tall bank.
        run(&mut v, &mut ps, &mut puffs, &mut rng, 15.0);
        let plow_x = v.plows[0].x;
        let dir = v.plows[0].dir;
        let behind = ps[0].cell_at(plow_x - dir * 80.0).unwrap();
        assert!(ps[0].h[behind] < 0.5, "cleared behind ({})", ps[0].h[behind]);
        let bank = ps[0].h.iter().cloned().fold(0.0, f32::max);
        assert!(bank > 16.0, "a bank builds up ahead of the blade ({bank})");
        // All the way across: the taskbar is (nearly) bare, the plow has left.
        run(&mut v, &mut ps, &mut puffs, &mut rng, 30.0);
        assert!(v.plows.is_empty(), "the plow drives off");
        assert!(ps[0].amount() < before * 0.1, "{} of {before} left", ps[0].amount());
        assert!(ps[1].amount() > 0.0, "windows are left alone");
    }

    #[test]
    fn penguins_walk_on_the_surface_and_fall_off_window_edges_onto_the_taskbar() {
        let mut v = Visitors::new();
        v.plows_on = false;
        v.penguins_on = false;
        let mut ps = piles();
        let (mut puffs, mut rng) = (Vec::new(), Rng::new(4));
        // Put a penguin on the window, walking right towards its edge.
        v.penguins.push(Penguin {
            x: 850.0,
            y: 292.0,
            vx: 0.0,
            vy: 0.0,
            dir: 1.0,
            speed: 25.0,
            state: PState::Walk,
            timer: 100.0,
            life: 100.0,
            phase: 0.0,
            on: Some(1),
            scarf: SCARVES[0],
            leaving: false,
        });
        let mut saw_fall = false;
        for _ in 0..60 * 6 {
            v.step(1.0 / 60.0, &mut ps, &mut puffs, &mut rng, 1.0, 1600.0, 900.0);
            let p = &v.penguins[0];
            if p.state == PState::Walk && p.on == Some(1) {
                let top = surface_at(&ps[1], p.x);
                assert!((p.y - top).abs() < 0.01, "feet on the snow");
            }
            saw_fall |= p.state == PState::Fall;
        }
        assert!(saw_fall, "walked off the window edge");
        assert_eq!(v.penguins[0].on, Some(100), "landed on the taskbar");
        // It left footprints on the window.
        assert!(ps[1].h.iter().any(|&h| h < 7.5));
    }

    #[test]
    fn penguins_turn_back_at_a_covering_window_and_eventually_leave() {
        let mut v = Visitors::new();
        v.plows_on = false;
        v.penguins_on = false;
        let mut ps = piles();
        ps[1].visible = vec![(400.0, 650.0)];
        let (mut puffs, mut rng) = (Vec::new(), Rng::new(5));
        v.penguins.push(Penguin {
            x: 600.0,
            y: 292.0,
            vx: 0.0,
            vy: 0.0,
            dir: 1.0,
            speed: 25.0,
            state: PState::Walk,
            timer: 100.0,
            life: 100.0,
            phase: 0.0,
            on: Some(1),
            scarf: SCARVES[1],
            leaving: false,
        });
        for _ in 0..60 * 4 {
            v.step(1.0 / 60.0, &mut ps, &mut puffs, &mut rng, 1.0, 1600.0, 900.0);
            assert!(v.penguins[0].x < 650.0, "never walks into the covered part");
        }
        assert!(v.penguins[0].dir < 0.0, "turned around");
        // A penguin on the taskbar leaves after a while.
        let mut w = Visitors::new();
        w.plows_on = false;
        w.penguins_on = false;
        w.summon_penguin();
        let mut ps = piles();
        ps.truncate(1);
        run(&mut w, &mut ps, &mut puffs, &mut rng, 1.0);
        assert_eq!(w.penguins.len(), 1);
        run(&mut w, &mut ps, &mut puffs, &mut rng, 240.0);
        assert!(w.penguins.is_empty(), "wandered off again");
    }

    #[test]
    fn nothing_breaks_without_any_surfaces() {
        let mut v = Visitors::new();
        v.summon_plow();
        v.summon_penguin();
        let (mut puffs, mut rng) = (Vec::new(), Rng::new(6));
        let mut none: Vec<Pile> = Vec::new();
        run(&mut v, &mut none, &mut puffs, &mut rng, 5.0);
        assert!(v.plows.is_empty() && v.penguins.is_empty());
        let mut out = Vec::new();
        v.sprites(&none, 1.0, &mut out);
    }
}
