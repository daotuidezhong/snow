//! Procedurally drawn sprite atlas: soft flakes at several focus levels, fluffy
//! aggregate flakes, six-fold ice crystals, snow clumps and a sparkle glint.
//! Pure pixel math (no platform code), so it can be inspected anywhere.

use crate::snow::Rng;

/// Side of one atlas cell in pixels.
pub const CELL: u32 = 128;
pub const COLS: u32 = 8;
pub const ROWS: u32 = 4;
pub const WIDTH: u32 = CELL * COLS;
pub const HEIGHT: u32 = CELL * ROWS;

/// Which sprites live where. Indices are cell numbers, row-major.
pub mod sprite {
    /// Soft round flakes, from in-focus (0) to very blurry (3).
    pub const DISC: [u16; 4] = [0, 1, 2, 3];
    /// Big, faint, out-of-focus flake right in front of the "lens".
    pub const BOKEH: u16 = 4;
    /// Sparkle on the surface of a pile.
    pub const GLINT: u16 = 5;
    /// Fluffy aggregate flakes (several crystals stuck together).
    pub const FLUFF: [u16; 2] = [6, 7];
    /// Six-armed ice crystals.
    pub const CRYSTAL: [u16; 8] = [8, 9, 10, 11, 12, 13, 14, 15];
    /// Irregular clumps of packed snow (thrown or falling off piles).
    pub const CLUMP: [u16; 8] = [16, 17, 18, 19, 20, 21, 22, 23];
    /// Lumps that sit on a pile's surface: like clumps, but no outline halo.
    pub const LUMP: [u16; 8] = [24, 25, 26, 27, 28, 29, 30, 31];
}

/// For each sprite: the fraction of the cell's half-width that the visible
/// "body" fills. Quads are sized so the body matches the flake radius.
pub fn body_fraction(id: u16) -> f32 {
    match id {
        0 => 0.42,
        1 => 0.36,
        2 => 0.30,
        3 => 0.24,
        sprite::BOKEH => 0.55,
        sprite::GLINT => 0.30,
        6 | 7 => 0.62,
        8..=15 => 0.88,
        _ => 0.62,
    }
}

pub fn cell_rect(id: u16) -> (u32, u32, u32, u32) {
    let (cx, cy) = (id as u32 % COLS, id as u32 / COLS);
    (cx * CELL, cy * CELL, cx * CELL + CELL, cy * CELL + CELL)
}

/// Premultiplied RGBA accumulator for one cell, in [0, 1].
struct Canvas {
    px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new() -> Self {
        Self { px: vec![[0.0; 4]; (CELL * CELL) as usize] }
    }
    /// Composite a straight-alpha colour over the pixel.
    fn over(&mut self, x: usize, y: usize, rgb: [f32; 3], a: f32) {
        let a = a.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let p = &mut self.px[y * CELL as usize + x];
        for k in 0..3 {
            p[k] = rgb[k] * a + p[k] * (1.0 - a);
        }
        p[3] = a + p[3] * (1.0 - a);
    }
    /// A dark, soft halo under everything so white flakes still read on white windows.
    fn add_shadow(&mut self, radius_px: f32, strength: f32) {
        let n = CELL as usize;
        let src: Vec<f32> = self.px.iter().map(|p| p[3]).collect();
        let r = radius_px.ceil() as i32;
        let mut blur = vec![0.0f32; n * n];
        // Box blur twice ~ cheap Gaussian.
        for pass in 0..2 {
            let input = if pass == 0 { src.clone() } else { blur.clone() };
            let mut tmp = vec![0.0f32; n * n];
            for y in 0..n {
                for x in 0..n {
                    let mut s = 0.0;
                    let mut c = 0.0;
                    for d in -r..=r {
                        let xx = x as i32 + d;
                        if xx >= 0 && (xx as usize) < n {
                            s += input[y * n + xx as usize];
                            c += 1.0;
                        }
                    }
                    tmp[y * n + x] = s / c;
                }
            }
            for y in 0..n {
                for x in 0..n {
                    let mut s = 0.0;
                    let mut c = 0.0;
                    for d in -r..=r {
                        let yy = y as i32 + d;
                        if yy >= 0 && (yy as usize) < n {
                            s += tmp[yy as usize * n + x];
                            c += 1.0;
                        }
                    }
                    blur[y * n + x] = s / c;
                }
            }
        }
        let shade = [0.20, 0.26, 0.36];
        for i in 0..n * n {
            let sa = (blur[i] * strength).min(0.5);
            let p = &mut self.px[i];
            // Shadow goes *under* the existing pixel.
            let a_under = sa * (1.0 - p[3]);
            for k in 0..3 {
                p[k] += shade[k] * a_under;
            }
            p[3] += a_under;
        }
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smooth value noise in [0, 1], for snow grain.
fn noise2(x: f32, y: f32, seed: u32) -> f32 {
    fn h(ix: i32, iy: i32, seed: u32) -> f32 {
        let mut v = (ix as u32).wrapping_mul(0x8DA6_B343) ^ (iy as u32).wrapping_mul(0xD816_3841) ^ seed.wrapping_mul(0xCB1A_B31F);
        v ^= v >> 13;
        v = v.wrapping_mul(0x5BD1_E995);
        v ^= v >> 15;
        (v & 0xFFFF) as f32 / 65535.0
    }
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let a = h(ix, iy, seed) + (h(ix + 1, iy, seed) - h(ix, iy, seed)) * ux;
    let b = h(ix, iy + 1, seed) + (h(ix + 1, iy + 1, seed) - h(ix, iy + 1, seed)) * ux;
    a + (b - a) * uy
}

/// Fractal noise: a few octaves of `noise2`.
fn fbm(x: f32, y: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut f = 1.0;
    for o in 0..4 {
        sum += amp * noise2(x * f, y * f, seed + o);
        f *= 2.03;
        amp *= 0.5;
    }
    sum / 0.9375
}

fn seg_dist(px: f32, py: f32, ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let (dx, dy) = (bx - ax, by - ay);
    let t = (((px - ax) * dx + (py - ay) * dy) / (dx * dx + dy * dy).max(1e-6)).clamp(0.0, 1.0);
    ((px - ax - t * dx).powi(2) + (py - ay - t * dy).powi(2)).sqrt()
}

/// Soft round flake; `blur` 0 = crisp edge, 1 = very soft.
fn disc(c: &mut Canvas, body: f32, blur: f32) {
    let n = CELL as f32;
    let half = n / 2.0;
    let r = body * half;
    let soft = 1.0 + blur * r * 1.2;
    for y in 0..CELL as usize {
        for x in 0..CELL as usize {
            let d = ((x as f32 + 0.5 - half).powi(2) + (y as f32 + 0.5 - half).powi(2)).sqrt();
            let a = 1.0 - smoothstep(r - soft, r + soft, d);
            // Slight sheen: lighter toward the upper left.
            let lit = 1.0 - 0.06 * ((x as f32 - half * 0.7) + (y as f32 - half * 0.7)) / n;
            c.over(x, y, [0.93 * lit, 0.96 * lit, 1.0], a * (1.0 - 0.25 * blur));
        }
    }
}

fn bokeh(c: &mut Canvas) {
    let half = CELL as f32 / 2.0;
    let r = 0.55 * half;
    for y in 0..CELL as usize {
        for x in 0..CELL as usize {
            let d = ((x as f32 + 0.5 - half).powi(2) + (y as f32 + 0.5 - half).powi(2)).sqrt();
            // Out-of-focus disc: flat inside, brighter rim, soft edge.
            let inside = 1.0 - smoothstep(r - 6.0, r + 6.0, d);
            let rim = (-((d - r * 0.92) / 5.0).powi(2)).exp() * 0.35;
            c.over(x, y, [0.95, 0.97, 1.0], (0.35 * inside + rim) * inside.max(rim));
        }
    }
}

fn glint(c: &mut Canvas) {
    let half = CELL as f32 / 2.0;
    for y in 0..CELL as usize {
        for x in 0..CELL as usize {
            let (dx, dy) = ((x as f32 + 0.5 - half) / half, (y as f32 + 0.5 - half) / half);
            let d = (dx * dx + dy * dy).sqrt();
            // Four-point star with a bright core.
            let star = (-(dx.abs() * 9.0) - dy.abs() * 1.6).exp() + (-(dy.abs() * 9.0) - dx.abs() * 1.6).exp();
            let core = (-(d * d) * 60.0).exp();
            let fade = 1.0 - smoothstep(0.6, 0.95, d);
            c.over(x, y, [1.0, 1.0, 1.0], ((star * 0.9 + core) * fade).min(1.0));
        }
    }
}

/// Six-fold dendrite (or plate) crystal with random branching.
fn crystal(c: &mut Canvas, rng: &mut Rng) {
    let half = CELL as f32 / 2.0;
    let arm = 0.84 * half;
    let width = rng.range(1.6, 2.6);
    let plate = if rng.f() < 0.45 { rng.range(0.12, 0.3) * arm } else { 0.0 };
    // Branch positions/lengths, shared by all six arms (real crystals grow symmetrically).
    let nb = 2 + (rng.f() * 3.0) as usize;
    let branches: Vec<(f32, f32)> = (0..nb)
        .map(|k| {
            let t = 0.3 + 0.6 * (k as f32 + rng.range(0.1, 0.9)) / nb as f32;
            (t, rng.range(0.15, 0.42) * (1.1 - t) * arm)
        })
        .collect();
    let tip_fork = rng.f() < 0.5;
    let mut segs: Vec<(f32, f32, f32, f32, f32)> = Vec::new();
    for a in 0..6 {
        let th = a as f32 * std::f32::consts::FRAC_PI_3 + std::f32::consts::FRAC_PI_6;
        let (ux, uy) = (th.cos(), th.sin());
        segs.push((0.0, 0.0, ux * arm, uy * arm, width));
        for &(t, len) in &branches {
            let (bx, by) = (ux * arm * t, uy * arm * t);
            for s in [-1.0f32, 1.0] {
                let bt = th + s * std::f32::consts::FRAC_PI_3;
                segs.push((bx, by, bx + bt.cos() * len, by + bt.sin() * len, width * 0.8));
            }
        }
        if tip_fork {
            let (bx, by) = (ux * arm * 0.92, uy * arm * 0.92);
            for s in [-1.0f32, 1.0] {
                let bt = th + s * 0.5;
                segs.push((bx, by, bx + bt.cos() * arm * 0.1, by + bt.sin() * arm * 0.1, width * 0.7));
            }
        }
    }
    for y in 0..CELL as usize {
        for x in 0..CELL as usize {
            let (px, py) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
            // Signed coverage from the nearest stroke.
            let mut best = f32::MAX;
            for &(ax, ay, bx, by, w) in &segs {
                best = best.min(seg_dist(px, py, ax, ay, bx, by) - w);
            }
            if plate > 0.0 {
                // Hexagonal plate: max of projections onto the six edge normals.
                let mut hex: f32 = 0.0;
                for a in 0..3 {
                    let th = a as f32 * std::f32::consts::FRAC_PI_3;
                    hex = hex.max((px * th.cos() + py * th.sin()).abs());
                }
                best = best.min(hex - plate);
            }
            let a = 1.0 - smoothstep(-0.7, 0.7, best);
            // Ice sheen: the middle of each stroke is brightest, edges a touch blue.
            let core = (-best.min(0.0) / 2.0).clamp(0.0, 1.0);
            let rgb = [0.86 + 0.14 * core, 0.92 + 0.08 * core, 1.0];
            c.over(x, y, rgb, a);
        }
    }
}

/// Fluffy aggregate or packed clump: a union of soft blobs, with grainy
/// broken edges and granular shading lit from the upper left.
fn blobs(c: &mut Canvas, rng: &mut Rng, body: f32, count: usize, grain: f32) {
    let half = CELL as f32 / 2.0;
    let r_all = body * half;
    let seed = rng.next_u64() as u32;
    let mut parts = Vec::new();
    for k in 0..count {
        let ang = rng.range(0.0, std::f32::consts::TAU);
        let dist = if k == 0 { 0.0 } else { rng.range(0.15, 0.65) * r_all };
        let r = rng.range(0.25, 0.55) * r_all * if k == 0 { 1.3 } else { 1.0 };
        parts.push((ang.cos() * dist, ang.sin() * dist, r));
    }
    for y in 0..CELL as usize {
        for x in 0..CELL as usize {
            let (px, py) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
            // Smooth union of circles (metaball-ish).
            let mut field = 0.0;
            for &(cx, cy, r) in &parts {
                let d2 = ((px - cx).powi(2) + (py - cy).powi(2)) / (r * r);
                field += (-d2 * 1.6).exp();
            }
            // Grain: coarse noise eats into the edge, fine noise speckles the surface.
            let coarse = fbm(px / 9.0, py / 9.0, seed);
            let fine = noise2(px / 2.2, py / 2.2, seed ^ 0xABCD);
            let f = field - grain * (coarse - 0.5) * 0.6;
            let a = smoothstep(0.30, 0.40, f);
            if a <= 0.0 {
                continue;
            }
            // Lit from the upper left; shaded blue-grey on the lower right; sparkly grains.
            let l = (1.0 - 0.22 * ((px + py) / (2.0 * r_all)).clamp(-1.0, 1.0)).clamp(0.74, 1.08);
            let l = l * (0.93 + 0.1 * fine);
            c.over(x, y, [(0.89 * l).min(1.0), (0.93 * l).min(1.0), (1.0 * l).min(1.0)], a);
        }
    }
}

/// Aggregate of a few tiny crystals and grains: what most real flakes look like up close.
fn fluff(c: &mut Canvas, rng: &mut Rng) {
    blobs(c, rng, 0.42, 9, 1.0);
    let half = CELL as f32 / 2.0;
    // Little crystal arms poking out of the fluff.
    let mut segs = Vec::new();
    for _ in 0..9 {
        let th = rng.range(0.0, std::f32::consts::TAU);
        let r0 = rng.range(0.18, 0.32) * half;
        let len = rng.range(0.12, 0.24) * half;
        let (sx, sy) = (th.cos() * r0, th.sin() * r0);
        segs.push((sx, sy, sx + th.cos() * len, sy + th.sin() * len));
        let b = th + if rng.f() < 0.5 { 1.0 } else { -1.0 };
        let (mx, my) = (sx + th.cos() * len * 0.6, sy + th.sin() * len * 0.6);
        segs.push((mx, my, mx + b.cos() * len * 0.35, my + b.sin() * len * 0.35));
    }
    for y in 0..CELL as usize {
        for x in 0..CELL as usize {
            let (px, py) = (x as f32 + 0.5 - half, y as f32 + 0.5 - half);
            let d = segs.iter().map(|&(ax, ay, bx, by)| seg_dist(px, py, ax, ay, bx, by)).fold(f32::MAX, f32::min);
            let a = 1.0 - smoothstep(1.0, 2.4, d);
            c.over(x, y, [0.92, 0.96, 1.0], a);
        }
    }
}

/// Build the whole atlas as premultiplied BGRA bytes (Direct2D's preferred layout).
pub fn atlas_bgra() -> Vec<u8> {
    let mut rng = Rng::new(0x5_0_0_5_1_7);
    let mut out = vec![0u8; (WIDTH * HEIGHT * 4) as usize];
    for id in 0..(COLS * ROWS) as u16 {
        let mut c = Canvas::new();
        match id {
            0..=3 => {
                disc(&mut c, body_fraction(id), id as f32 / 3.0);
                if id <= 1 {
                    c.add_shadow(3.0, 1.1);
                }
            }
            sprite::BOKEH => bokeh(&mut c),
            sprite::GLINT => glint(&mut c),
            6 | 7 => {
                fluff(&mut c, &mut rng);
                c.add_shadow(3.0, 1.0);
            }
            8..=15 => {
                crystal(&mut c, &mut rng);
                c.add_shadow(2.5, 1.4);
            }
            16..=23 => {
                let count = 4 + (rng.f() * 5.0) as usize;
                blobs(&mut c, &mut rng, body_fraction(id), count, 1.0);
                c.add_shadow(3.0, 1.2);
            }
            _ => {
                let count = 3 + (rng.f() * 4.0) as usize;
                blobs(&mut c, &mut rng, body_fraction(id), count, 0.7);
            }
        }
        let (x0, y0, _, _) = cell_rect(id);
        for y in 0..CELL {
            for x in 0..CELL {
                let p = c.px[(y * CELL + x) as usize];
                let i = (((y0 + y) * WIDTH + x0 + x) * 4) as usize;
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                out[i] = q(p[2]);
                out[i + 1] = q(p[1]);
                out[i + 2] = q(p[0]);
                out[i + 3] = q(p[3]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atlas_is_premultiplied_and_cells_are_not_empty() {
        let a = atlas_bgra();
        assert_eq!(a.len(), (WIDTH * HEIGHT * 4) as usize);
        for px in a.chunks(4) {
            // Premultiplied: no channel brighter than alpha.
            assert!(px[0] <= px[3] && px[1] <= px[3] && px[2] <= px[3]);
        }
        for id in 0..(COLS * ROWS) as u16 {
            let (x0, y0, x1, y1) = cell_rect(id);
            let mut cover = 0u32;
            for y in y0..y1 {
                for x in x0..x1 {
                    cover += a[((y * WIDTH + x) * 4 + 3) as usize] as u32;
                }
            }
            assert!(cover > 255 * 40, "sprite {id} is empty");
            // Nothing touches the cell border, so neighbours never bleed in.
            for x in x0..x1 {
                assert!(a[((y0 * WIDTH + x) * 4 + 3) as usize] < 8, "sprite {id} touches top edge");
            }
        }
    }

    /// `SNOW_ATLAS_OUT=/path/atlas.rgba cargo test dump_atlas -- --ignored` writes raw RGBA for inspection.
    #[test]
    #[ignore]
    fn dump_atlas() {
        let a = atlas_bgra();
        let rgba: Vec<u8> = a.chunks(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
        std::fs::write(std::env::var("SNOW_ATLAS_OUT").unwrap(), rgba).unwrap();
    }
}
