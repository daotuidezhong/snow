//! Settings saved between runs, as a small `key=value` text file.

use crate::snow::{FlakeSize, Intensity, Weather, Wind};

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Config {
    pub weather: Weather,
    pub paused: bool,
}

impl Config {
    pub fn parse(text: &str) -> Self {
        let mut c = Config::default();
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "intensity" => {
                    if let Some(x) = Intensity::from_key(v) {
                        c.weather.intensity = x
                    }
                }
                "wind" => {
                    if let Some(x) = Wind::from_key(v) {
                        c.weather.wind = x
                    }
                }
                "wind_dir" => c.weather.wind_dir = if v == "left" { -1.0 } else { 1.0 },
                "size" => {
                    if let Some(x) = FlakeSize::from_key(v) {
                        c.weather.size = x
                    }
                }
                "paused" => c.paused = v == "true",
                _ => {}
            }
        }
        c
    }

    pub fn serialize(&self) -> String {
        format!(
            "# Snow settings\nintensity={}\nwind={}\nwind_dir={}\nsize={}\npaused={}\n",
            self.weather.intensity.key(),
            self.weather.wind.key(),
            if self.weather.wind_dir < 0.0 { "left" } else { "right" },
            self.weather.size.key(),
            self.paused,
        )
    }
}

fn path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("APPDATA")?;
    Some(std::path::Path::new(&base).join("Snow").join("config.txt"))
}

pub fn load() -> Config {
    path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Config::parse(&t)).unwrap_or_default()
}

pub fn save(c: &Config) {
    if let Some(p) = path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, c.serialize());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let c = Config {
            weather: Weather { intensity: Intensity::Blizzard, wind: Wind::Strong, wind_dir: -1.0, size: FlakeSize::Large },
            paused: true,
        };
        assert_eq!(Config::parse(&c.serialize()), c);
    }

    #[test]
    fn garbage_falls_back_to_defaults() {
        assert_eq!(Config::parse("intensity=lots\n???\nwind\n"), Config::default());
    }
}
