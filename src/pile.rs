//! Snow piles that build up on window top edges and the taskbar, plus the
//! loose clumps ("puffs") thrown off when a pile is brushed, blown, dragged
//! away or overflows at its ends.

use crate::art::sprite;
use crate::snow::Rng;

/// A horizontal surface snow can land on, as reported by the platform each scan.
/// Coordinates are physical pixels relative to the overlay.
#[derive(Clone, Debug, PartialEq)]
pub struct Ledge {
    /// Stable identity (the window handle), so a pile survives between scans.
    pub id: u64,
    pub x0: f32,
    pub x1: f32,
    /// The surface line; snow sits above it (smaller y).
    pub y: f32,
    /// Parts of [x0, x1] not covered by windows in front, sorted, non-overlapping.
    pub visible: Vec<(f32, f32)>,
    /// Tallest the pile may grow, in physical pixels.
    pub max_h: f32,
}

pub struct Pile {
    pub id: u64,
    pub x0: f32,
    pub x1: f32,
    pub y: f32,
    pub cell: f32,
    /// Snow depth per cell, in physical pixels.
    pub h: Vec<f32>,
    pub visible: Vec<(f32, f32)>,
    pub max_h: f32,
    /// Phase of the gentle wave along the top of a full pile, so piles differ.
    pub wave: f32,
    /// Snow that slid past the rounded ends and is about to drop off (area, px²).
    pub spill_left: f32,
    pub spill_right: f32,
    scratch: Vec<f32>,
}

/// A clump of snow flying free: thrown, blown or falling. Physical px and px/s.
#[derive(Clone, Copy, Debug)]
pub struct Puff {
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    /// Body radius in physical pixels.
    pub r: f32,
    pub alpha: f32,
    pub sprite: u16,
    pub angle: f32,
    pub spin: f32,
    /// How quickly the air drags it along, in seconds (small = light powder).
    pub tau: f32,
    /// Seconds since it was thrown; very young puffs don't land (avoids re-landing instantly).
    pub age: f32,
}

/// Steepest slope a pile keeps, as rise over run (snow is not a wall).
const SLOPE: f32 = 0.4;
/// Slope of the rounded ends of a pile near the window corners.
const END_SLOPE: f32 = 0.35;
/// How fast snow evens out between neighbours each frame (0..0.5): keeps mounds
/// round and soft without flattening them into a uniform strip.
const SMOOTH: f32 = 0.04;
/// How far a landing flake's snow is spread, in cells (Gaussian sigma): a small
/// soft mound around where it landed.
const SPREAD: f32 = 2.0;
/// Thinner than this (px), snow is not drawn.
pub const MIN_VISIBLE: f32 = 0.3;

impl Pile {
    pub fn new(l: &Ledge, cell: f32) -> Self {
        let n = (((l.x1 - l.x0) / cell).ceil() as usize).max(1);
        let wave = (l.id.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 40) as f32 / (1u64 << 24) as f32 * 6.283;
        Self {
            id: l.id,
            x0: l.x0,
            x1: l.x1,
            y: l.y,
            cell,
            h: vec![0.0; n],
            visible: l.visible.clone(),
            max_h: l.max_h,
            wave,
            spill_left: 0.0,
            spill_right: 0.0,
            scratch: Vec::new(),
        }
    }

    pub fn cell_at(&self, x: f32) -> Option<usize> {
        if x < self.x0 || x >= self.x1 {
            return None;
        }
        Some((((x - self.x0) / self.cell) as usize).min(self.h.len() - 1))
    }

    pub fn cell_x(&self, i: usize) -> f32 {
        self.x0 + (i as f32 + 0.5) * self.cell
    }

    pub fn is_visible(&self, x: f32) -> bool {
        self.visible.iter().any(|&(a, b)| x >= a && x < b)
    }

    /// Height limit of cell `i`, and whether the limit comes from a rounded end.
    fn cap_info(&self, i: usize) -> (f32, bool) {
        let from_end = i.min(self.h.len() - 1 - i) as f32 * self.cell + self.cell * 0.5;
        let x = i as f32 * self.cell;
        // Full piles keep a soft wave instead of a flat slab.
        let top = self.max_h * (0.86 + 0.07 * ((x * 0.021 + self.wave).sin() + (x * 0.0073 + 2.0 * self.wave).sin()));
        let end = from_end * END_SLOPE + 0.5;
        if end < top {
            (end, true)
        } else {
            (top, false)
        }
    }

    /// How much snow cell `i` catches relative to the average (about 0.1 .. 2).
    fn dune(&self, i: usize) -> f32 {
        let x = i as f32;
        let w = self.wave;
        // Ripple field in -1..1, sharpened so snow gathers into distinct mounds early
        // on (like real fresh snow on a ledge); full piles still level out at the cap.
        let r = (0.55 * (x * 0.31 + w).sin() + 0.3 * (x * 0.17 + 2.1 * w).sin() + 0.15 * (x * 0.73 + 4.3 * w).sin())
            .clamp(-1.0, 1.0);
        // Mean of this over x is about 1.
        0.12 + 2.1 * ((r + 1.0) * 0.5).powf(2.2) * 1.55
    }

    fn cap(&self, i: usize) -> f32 {
        self.cap_info(i).0
    }

    /// Surface y at `x` if snow could land there right now.
    pub fn surface(&self, x: f32) -> Option<f32> {
        let i = self.cell_at(x)?;
        if !self.is_visible(x) {
            return None;
        }
        Some(self.y - self.h[i])
    }

    /// True when the pile at `x` is full, so a flake there should keep falling.
    pub fn is_full_at(&self, x: f32) -> bool {
        self.cell_at(x).is_some_and(|i| self.h[i] >= self.cap(i) - 0.3)
    }

    /// Add `area` square pixels of snow around `x`. A flake's snow settles over a
    /// small neighbourhood (light fresh snow doesn't stack in a spike), so the
    /// surface rises as an even layer. Overflow is sorted out by `relax`.
    pub fn deposit(&mut self, x: f32, area: f32) {
        let Some(i) = self.cell_at(x) else { return };
        let n = self.h.len() as isize;
        let r = (SPREAD * 3.0) as isize;
        let lo = (i as isize - r).max(0);
        let hi = (i as isize + r).min(n - 1);
        let w = |j: isize| (-((j - i as isize) as f32).powi(2) / (2.0 * SPREAD * SPREAD)).exp();
        let total: f32 = (lo..=hi).map(w).sum();
        // Wind ripples: some spots catch more snow than others, so the layer grows
        // as rolling dunes in stable places rather than as an even strip.
        let dh = area / self.cell / total * self.dune(i);
        for j in lo..=hi {
            self.h[j as usize] += dh * w(j);
        }
    }

    /// One frame of settling: steep steps slide down, bumps diffuse, and snow
    /// pushed past the rounded ends is collected as spill to drop off the edge.
    pub fn relax(&mut self) {
        let n = self.h.len();
        if n >= 2 {
            // Gentle smoothing between neighbours keeps mounds round and soft.
            self.scratch.clear();
            self.scratch.extend_from_slice(&self.h);
            for i in 0..n - 1 {
                let (a, b) = (self.scratch[i], self.scratch[i + 1]);
                let flux = SMOOTH * 0.5 * (a - b);
                self.h[i] -= flux;
                self.h[i + 1] += flux;
            }
        }
        // Depth limit; what is pushed past the rounded ends slides off as spill.
        for i in 0..n {
            let (c, at_end) = self.cap_info(i);
            if self.h[i] > c {
                if at_end {
                    let area = (self.h[i] - c) * self.cell;
                    if i < n / 2 {
                        self.spill_left += area;
                    } else {
                        self.spill_right += area;
                    }
                }
                self.h[i] = c;
            }
            if self.h[i] < 0.0 {
                self.h[i] = 0.0;
            }
        }
        // Avalanche last, both directions, so no step is left steeper than the
        // angle of repose. It only moves snow downhill, never above a neighbour.
        let lim = SLOPE * self.cell;
        let fwd = 0..n.saturating_sub(1);
        for _ in 0..6 {
            let mut worst: f32 = 0.0;
            for i in fwd.clone().chain(fwd.clone().rev()) {
                let d = self.h[i] - self.h[i + 1];
                if d.abs() > lim {
                    // Settle slightly below the limit so the pass converges quickly.
                    let m = (d.abs() - lim * 0.98) * 0.5 * d.signum();
                    self.h[i] -= m;
                    self.h[i + 1] += m;
                    worst = worst.max(d.abs() - lim);
                }
            }
            if worst < 0.01 {
                break;
            }
        }
    }

    /// Total snow, in square pixels.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn amount(&self) -> f32 {
        self.h.iter().sum::<f32>() * self.cell
    }

    /// Clear all snow within `radius` of `x`, down to the bare edge.
    /// Returns (cell x, cell top y, removed area) per cell.
    pub fn brush(&mut self, x: f32, radius: f32) -> Vec<(f32, f32, f32)> {
        let mut out = Vec::new();
        for i in 0..self.h.len() {
            let cx = self.cell_x(i);
            if (cx - x).abs() <= radius && self.is_visible(cx) && self.h[i] > 0.0 {
                out.push((cx, self.y - self.h[i], self.h[i] * self.cell));
                self.h[i] = 0.0;
            }
        }
        out
    }

    /// Runs of neighbouring visible cells deep enough to draw, as inclusive (first, last) cell indices.
    pub fn runs(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start: Option<usize> = None;
        for i in 0..=self.h.len() {
            let on = i < self.h.len() && self.h[i] >= MIN_VISIBLE && self.is_visible(self.cell_x(i));
            match (on, start) {
                (true, None) => start = Some(i),
                (false, Some(a)) => {
                    out.push((a, i - 1));
                    start = None;
                }
                _ => {}
            }
        }
        out
    }

    /// Lift a thin layer of snow from a random exposed cell (wind erosion).
    /// Returns the (x, y, area) of what was lifted.
    pub fn erode(&mut self, rng: &mut Rng, depth: f32) -> Option<(f32, f32, f32)> {
        let i = ((rng.f() * self.h.len() as f32) as usize).min(self.h.len() - 1);
        let cx = self.cell_x(i);
        if self.h[i] < 2.0 || !self.is_visible(cx) {
            return None;
        }
        let take = depth.min(self.h[i] - 1.0);
        self.h[i] -= take;
        Some((cx, self.y - self.h[i], take * self.cell))
    }

    /// Break the whole pile into clumps (its window moved, closed or was minimized).
    /// `fling` is the window's velocity; clumps inherit part of it.
    pub fn shake_off(&self, rng: &mut Rng, scale: f32, fling: (f32, f32), out: &mut Vec<Puff>) {
        let mut acc = 0.0;
        let mut next = clump_area(rng, scale);
        let mut made = 0;
        for (i, &h) in self.h.iter().enumerate() {
            let cx = self.cell_x(i);
            if h <= 0.2 || !self.is_visible(cx) {
                continue;
            }
            acc += h * self.cell;
            while acc >= next && made < 600 {
                acc -= next;
                let r = (next / std::f32::consts::PI).sqrt();
                let k = rng.range(0.35, 0.7);
                out.push(Puff {
                    x: cx + rng.range(-0.5, 0.5) * self.cell,
                    y: self.y - rng.range(0.0, h),
                    vx: fling.0 * k + rng.range(-30.0, 30.0) * scale,
                    vy: fling.1 * k + rng.range(-40.0, 20.0) * scale,
                    r,
                    alpha: 1.0,
                    sprite: sprite::CLUMP[(rng.f() * 8.0) as usize % 8],
                    angle: rng.range(0.0, 6.283),
                    spin: rng.range(-4.0, 4.0),
                    tau: 2.5,
                    age: 0.0,
                });
                made += 1;
                next = clump_area(rng, scale);
            }
        }
    }
}

/// Area (px²) of a random clump, mostly small, sometimes chunky.
pub fn clump_area(rng: &mut Rng, scale: f32) -> f32 {
    let r = (1.6 + 3.2 * rng.f().powf(2.2)) * scale;
    std::f32::consts::PI * r * r
}

/// Subtract covered x-ranges from [x0, x1].
pub fn visible_parts(x0: f32, x1: f32, mut covers: Vec<(f32, f32)>) -> Vec<(f32, f32)> {
    covers.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out = Vec::new();
    let mut x = x0;
    for (a, b) in covers {
        if b <= x {
            continue;
        }
        if a >= x1 {
            break;
        }
        if a > x {
            out.push((x, a));
        }
        x = x.max(b);
        if x >= x1 {
            break;
        }
    }
    if x < x1 {
        out.push((x, x1));
    }
    out.retain(|(a, b)| b - a >= 4.0);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ledge(x0: f32, x1: f32) -> Ledge {
        Ledge { id: 1, x0, x1, y: 500.0, visible: vec![(x0, x1)], max_h: 12.0 }
    }

    #[test]
    fn subtracts_covers() {
        assert_eq!(visible_parts(0.0, 100.0, vec![]), vec![(0.0, 100.0)]);
        assert_eq!(visible_parts(0.0, 100.0, vec![(20.0, 40.0), (30.0, 60.0)]), vec![(0.0, 20.0), (60.0, 100.0)]);
        assert_eq!(visible_parts(0.0, 100.0, vec![(-10.0, 200.0)]), vec![]);
        assert_eq!(visible_parts(0.0, 100.0, vec![(98.0, 120.0)]), vec![(0.0, 98.0)]);
    }

    #[test]
    fn deposits_grow_smoothly_respect_caps_and_spill_at_ends() {
        let mut p = Pile::new(&ledge(0.0, 300.0), 3.0);
        for k in 0..20_000 {
            p.deposit((k % 300) as f32, 20.0);
            p.relax();
        }
        let max = p.h.iter().cloned().fold(0.0, f32::max);
        assert!(max > 10.0 && max <= 12.0 + 1e-3, "max {max}");
        assert!(p.h[0] < 2.0 && p.h[p.h.len() - 1] < 2.0, "ends stay rounded");
        for w in p.h.windows(3) {
            assert!((w[0] + w[2] - 2.0 * w[1]).abs() < 1.5, "jagged: {w:?}");
        }
        assert!(p.spill_left > 0.0 && p.spill_right > 0.0, "overflow slides off both ends");
    }

    #[test]
    fn falling_snow_builds_smooth_rolling_dunes() {
        let mut p = Pile::new(&ledge(0.0, 900.0), 3.0);
        let mut rng = Rng::new(3);
        // Flakes land at random all along the edge, like real snowfall.
        let mut seen_mounds = false;
        for step in 0..6000 {
            for _ in 0..2 {
                p.deposit(rng.range(0.0, 900.0), 6.0);
            }
            p.relax();
            if step == 600 {
                // Early on: separate soft mounds with bare gaps between some of them.
                let peaks = (1..p.h.len() - 1).filter(|&i| p.h[i] > 0.5 && p.h[i] >= p.h[i - 1] && p.h[i] > p.h[i + 1]).count();
                seen_mounds = peaks >= 5;
            }
            for w in p.h.windows(2) {
                assert!((w[0] - w[1]).abs() < SLOPE * 3.0 + 0.1, "no steps or spikes: {w:?}");
            }
        }
        assert!(seen_mounds, "snow starts as individual mounds");
        // Later: a continuous layer that still rolls (dunes), within the limit.
        let inner = &p.h[30..p.h.len() - 30];
        let (mn, mx) = inner.iter().fold((f32::MAX, 0.0f32), |(a, b), &h| (a.min(h), b.max(h)));
        assert!(mn > 1.0, "continuous layer ({mn})");
        assert!(mx - mn > 1.5, "with rolling dunes ({mn}..{mx})");
        assert!(mx <= 12.0 + 1e-3);
    }

    #[test]
    fn fallen_snow_never_disappears_by_itself() {
        let mut q = Pile::new(&ledge(0.0, 300.0), 3.0);
        q.deposit(150.0, 3.0);
        let before = q.amount();
        for _ in 0..6000 {
            q.relax();
        }
        assert!((q.amount() - before).abs() < before * 0.01, "{} vs {before}", q.amount());
    }

    #[test]
    fn hidden_parts_take_no_snow() {
        let mut l = ledge(0.0, 300.0);
        l.visible = vec![(0.0, 100.0), (200.0, 300.0)];
        let p = Pile::new(&l, 3.0);
        assert!(p.surface(150.0).is_none());
        assert_eq!(p.surface(50.0), Some(500.0));
        assert!(p.surface(-1.0).is_none() && p.surface(300.0).is_none());
    }

    #[test]
    fn brushing_removes_snow_and_shake_off_conserves_it_roughly() {
        let mut p = Pile::new(&ledge(0.0, 300.0), 3.0);
        for k in 0..5000 {
            p.deposit((k % 300) as f32, 20.0);
            p.relax();
        }
        let before = p.amount();
        let removed: f32 = p.brush(150.0, 20.0).iter().map(|c| c.2).sum();
        assert!(removed > 0.0);
        assert!((p.amount() - (before - removed)).abs() < 1.0);
        let i0 = p.cell_at(131.0).unwrap();
        let i1 = p.cell_at(169.0).unwrap();
        assert!(p.h[i0..=i1].iter().all(|&h| h == 0.0), "the swipe clears right down to the edge");
        let mut puffs = Vec::new();
        p.shake_off(&mut Rng::new(5), 1.0, (400.0, 0.0), &mut puffs);
        let area: f32 = puffs.iter().map(|q| std::f32::consts::PI * q.r * q.r).sum();
        assert!(area > p.amount() * 0.7 && area <= p.amount() * 1.01, "{area} vs {}", p.amount());
        assert!(puffs.iter().map(|q| q.vx).sum::<f32>() > 0.0, "clumps fly the way the window moved");
    }
}

#[cfg(test)]
mod dune_norm {
    use super::*;
    #[test]
    fn dune_field_averages_about_one() {
        let p = Pile::new(&Ledge { id: 77, x0: 0.0, x1: 30000.0, y: 0.0, visible: vec![(0.0, 30000.0)], max_h: 10.0 }, 3.0);
        let mean: f32 = (0..p.h.len()).map(|i| p.dune(i)).sum::<f32>() / p.h.len() as f32;
        assert!((mean - 1.0).abs() < 0.15, "mean {mean}");
    }
}
