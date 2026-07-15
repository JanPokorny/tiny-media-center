// Port of config.lua: reads $XDG_CONFIG_HOME/tiny-media-center/config.toml,
// falling back to defaults for anything missing or unparsable.

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

#[derive(Deserialize)]
#[serde(default)]
pub struct Style {
    pub background_color: [f32; 3],
    pub text_color: [f32; 3],
    pub accent_color: [f32; 3],
    pub dim_color: [f32; 3],
    pub font_size: f32,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            background_color: [0.0, 0.0, 0.0],
            text_color: [1.0, 1.0, 1.0],
            accent_color: [1.0, 0.8, 0.0],
            dim_color: [0.5, 0.5, 0.5],
            font_size: 72.0,
        }
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub media_path: String,
    pub style: Style,
}

impl Default for Config {
    fn default() -> Self {
        Config { media_path: ".".into(), style: Style::default() }
    }
}

pub fn load() -> Config {
    std::fs::read_to_string(config_dir().join("config.toml"))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default()
}
