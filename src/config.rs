// Constants for this TV box, plus the subtitle credentials read from
// $XDG_CONFIG_HOME/tiny-media-center/config.toml (defaults when missing or unparsable).

use serde::Deserialize;
use std::path::PathBuf;

pub fn color(rgb: [f32; 3], alpha: f32) -> femtovg::Color {
    femtovg::Color::rgbaf(rgb[0], rgb[1], rgb[2], alpha)
}

pub fn config_dir() -> PathBuf {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    PathBuf::from(config_home).join("tiny-media-center")
}

pub const MEDIA_PATH: &str = "/media";
pub const FONT_SIZE: f32 = 72.0;
pub const BACKGROUND_COLOR: [f32; 3] = [0.0, 0.0, 0.0];
pub const TEXT_COLOR: [f32; 3] = [1.0, 1.0, 1.0];
pub const ACCENT_COLOR: [f32; 3] = [1.0, 0.8, 0.0];
pub const DIM_COLOR: [f32; 3] = [0.5, 0.5, 0.5];

// Subtitle downloads (see subtitles.rs). OpenSubtitles needs an API key
// (opensubtitles.com profile > API consumers); the account is optional and
// raises the daily download limit from 5 to 20.
#[derive(Deserialize, Default, Clone)]
#[serde(default)]
pub struct Subtitles {
    pub opensubtitles_api_key: String,
    pub opensubtitles_username: String,
    pub opensubtitles_password: String,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Config {
    subtitles: Subtitles,
}

pub fn load() -> Subtitles {
    let path = config_dir().join("config.toml");
    // A missing config is normal (defaults); a broken one gets reported
    // instead of silently falling back.
    let Ok(s) = std::fs::read_to_string(&path) else { return Subtitles::default() };
    toml::from_str::<Config>(&s)
        .unwrap_or_else(|e| {
            eprintln!("tiny-media-center: ignoring unparsable config {}: {e}", path.display());
            Config::default()
        })
        .subtitles
}
