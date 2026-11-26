//! Snowfall simulation. Pure math, no platform code, so it can be tested anywhere.

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

/// One snowflake. Positions are in physical pixels of the overlay.
#[derive(Clone, Copy, Debug)]
pub struct Flake {
    pub x: f32,
    pub y: f32,
    /// Base radius in physical pixels.
    pub r: f32,
    /// 0 = far away (small, slow, faint), 1 = close.
    pub depth: f32,
    pub alpha: f32,
    phase: f32,
    sway_freq: f32,
    sway_amp: f32,
    fall: f32,
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

pub struct Snow {
    pub flakes: Vec<Flake>,
    width: f32,
    height: f32,
    /// Physical pixels per logical pixel (DPI / 96).
    scale: f32,
    weather: Weather,
    rng: Rng,
    /// Current horizontal wind in logical px/s, eased toward the target.
    wind_now: f32,
    gust: f32,
    gust_target: f32,
    gust_timer: f32,
    time: f32,
}

impl Snow {
    pub fn new(width: f32, height: f32, scale: f32, weather: Weather, seed: u64) -> Self {
        let mut s = Self {
            flakes: Vec::new(),
            width,
            height,
            scale,
            weather,
            rng: Rng::new(seed),
            wind_now: weather.wind.speed() * weather.wind_dir,
            gust: 0.0,
            gust_target: 0.0,
            gust_timer: 0.0,
            time: 0.0,
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
            let k = w.size.scale();
            let s = self.scale;
            for f in &mut self.flakes {
                f.r = flake_radius(f.depth, s, k);
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
        if (scale - self.scale).abs() > f32::EPSILON {
            self.scale = scale;
            let k = self.weather.size.scale();
            for f in &mut self.flakes {
                f.r = flake_radius(f.depth, scale, k);
            }
        }
    }

    /// Number of flakes the current screen and intensity call for.
    pub fn target_count(&self) -> usize {
        let mpx = self.width * self.height / 1.0e6;
        // Bigger DPI scale means bigger flakes, so fewer are needed for the same look.
        let n = mpx * self.weather.intensity.density() / self.scale.max(1.0);
        (n as usize).clamp(10, 12_000)
    }

    fn spawn(&mut self) -> Flake {
        // Bias toward far-away flakes: they are the majority in real snowfall.
        let depth = self.rng.f().powf(1.6);
        let k = self.weather.size.scale();
        let r = flake_radius(depth, self.scale, k);
        // Spawn across a band wider than the screen so wind doesn't leave an empty edge.
        let margin = 0.25 * self.width;
        Flake {
            x: self.rng.range(-margin, self.width + margin),
            y: -r * 3.0 - self.rng.range(0.0, 0.08 * self.height),
            r,
            depth,
            alpha: 0.55 + 0.45 * depth,
            phase: self.rng.range(0.0, std::f32::consts::TAU),
            sway_freq: self.rng.range(0.4, 1.3),
            sway_amp: self.rng.range(6.0, 22.0) * (0.4 + 0.6 * depth),
            fall: (22.0 + 58.0 * depth) * self.rng.range(0.85, 1.15),
        }
    }

    /// Advance the simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        let dt = dt.clamp(0.0, 0.1);
        self.time += dt;
        let blizzard = self.weather.intensity == Intensity::Blizzard;

        // Gusts: every few seconds pick a new gust strength.
        self.gust_timer -= dt;
        if self.gust_timer <= 0.0 {
            self.gust_timer = self.rng.range(2.5, 7.0);
            let max = match (self.weather.wind, blizzard) {
                (_, true) => 160.0,
                (Wind::Calm, _) => 8.0,
                (Wind::Breeze, _) => 30.0,
                (Wind::Strong, _) => 80.0,
            };
            self.gust_target = self.rng.range(-0.3, 1.0) * max;
        }
        self.gust += (self.gust_target - self.gust) * (dt * 0.8).min(1.0);
        let base = self.weather.wind.speed() + if blizzard { 90.0 } else { 0.0 };
        let target = (base + self.gust) * self.weather.wind_dir;
        self.wind_now += (target - self.wind_now) * (dt * 1.5).min(1.0);

        let s = self.scale;
        let fall_boost = if blizzard { 1.6 } else { 1.0 };
        let (w, h) = (self.width, self.height);
        let t = self.time;
        let wind = self.wind_now;
        for f in &mut self.flakes {
            let sway = (t * f.sway_freq + f.phase).cos() * f.sway_amp * f.sway_freq;
            // Near flakes feel the wind more (parallax).
            let vx = wind * (0.45 + 0.75 * f.depth) + sway;
            f.x += vx * s * dt;
            f.y += f.fall * fall_boost * s * dt;
        }

        // Recycle flakes that left the screen; keep the count at the target.
        let target = self.target_count();
        let margin = 0.3 * w;
        let mut i = 0;
        while i < self.flakes.len() {
            let f = self.flakes[i];
            let gone = f.y - f.r > h || f.x < -margin || f.x > w + margin;
            if gone {
                if self.flakes.len() > target {
                    self.flakes.swap_remove(i);
                    continue;
                }
                let mut nf = self.spawn();
                // Strong side wind: enter from the upwind edge too, not only the top.
                if wind.abs() > 40.0 && self.rng.f() < 0.35 {
                    nf.y = self.rng.range(0.0, h * 0.8);
                    nf.x = if wind > 0.0 { -nf.r * 2.0 } else { w + nf.r * 2.0 };
                }
                self.flakes[i] = nf;
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
}

fn flake_radius(depth: f32, scale: f32, size: f32) -> f32 {
    (1.3 + 3.4 * depth) * scale * size
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_tracks_intensity_and_area() {
        let mut w = Weather::default();
        let s = Snow::new(1920.0, 1080.0, 1.0, w, 7);
        let normal = s.target_count();
        assert_eq!(s.flakes.len(), normal);
        w.intensity = Intensity::Blizzard;
        let b = Snow::new(1920.0, 1080.0, 1.0, w, 7);
        assert!(b.target_count() > normal * 3);
        let big = Snow::new(3840.0, 2160.0, 1.0, Weather::default(), 7);
        assert!(big.target_count() > normal * 3);
    }

    #[test]
    fn flakes_fall_and_stay_bounded() {
        let mut s = Snow::new(1280.0, 720.0, 1.0, Weather::default(), 42);
        let before: Vec<f32> = s.flakes.iter().map(|f| f.y).collect();
        s.step(1.0 / 60.0);
        let down = s.flakes.iter().zip(&before).filter(|(f, y)| f.y > **y).count();
        assert!(down * 100 > before.len() * 95, "almost every flake should move down each frame");
        for _ in 0..60 * 120 {
            s.step(1.0 / 60.0);
        }
        let t = s.target_count();
        assert!(s.flakes.len() <= t + 1);
        assert!(s.flakes.len() >= t * 9 / 10);
        for f in &s.flakes {
            assert!(f.x.is_finite() && f.y.is_finite());
            assert!(f.y < 720.0 + 50.0 && f.x > -1280.0 && f.x < 2560.0);
        }
    }

    #[test]
    fn lowering_intensity_shrinks_population() {
        let mut w = Weather { intensity: Intensity::Blizzard, ..Weather::default() };
        let mut s = Snow::new(1280.0, 720.0, 1.0, w, 3);
        w.intensity = Intensity::Light;
        s.set_weather(w);
        for _ in 0..60 * 60 {
            s.step(1.0 / 60.0);
        }
        assert!(s.flakes.len() <= s.target_count() + 1);
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
