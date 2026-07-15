// Port of config.lua: reads $XDG_CONFIG_HOME/tiny-media-center/config.toml,
// falling back to defaults for anything missing or unparsable.

use serde::Deserialize;

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

#[derive(Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub media_path: String,
    pub style: Style,
}

pub fn load() -> Config {
    let config_home = std::env::var("XDG_CONFIG_HOME")
        .unwrap_or_else(|_| format!("{}/.config", std::env::var("HOME").unwrap_or_default()));
    let mut config: Config = std::fs::read_to_string(format!("{config_home}/tiny-media-center/config.toml"))
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_default();
    if config.media_path.is_empty() {
        config.media_path = ".".into();
    }
    config
}
