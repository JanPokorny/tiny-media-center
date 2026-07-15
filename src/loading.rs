// Port of components/loading_screen.lua.

use crate::config::Config;
use femtovg::{Baseline, Canvas, Color, FontId, Paint, Renderer};

pub fn draw<R: Renderer>(
    canvas: &mut Canvas<R>,
    font: FontId,
    config: &Config,
    text: &str,
    subtext: &str,
    w: f32,
    h: f32,
) {
    let size = config.style.font_size;
    let c = config.style.text_color;
    let paint = Paint::color(Color::rgbf(c[0], c[1], c[2]))
        .with_font(&[font])
        .with_font_size(size)
        .with_text_baseline(Baseline::Top);
    let text_w = canvas.measure_text(0.0, 0.0, text, &paint).map_or(0.0, |m| m.width());
    let _ = canvas.fill_text((w - text_w) / 2.0, (h - size) / 2.0 - size * 0.7, text, &paint);

    if !subtext.is_empty() {
        let d = config.style.dim_color;
        let paint = paint
            .with_color(Color::rgbf(d[0], d[1], d[2]))
            .with_font_size(size * 0.5);
        let sub_w = canvas.measure_text(0.0, 0.0, subtext, &paint).map_or(0.0, |m| m.width());
        let _ = canvas.fill_text((w - sub_w) / 2.0, (h - size) / 2.0 + size * 0.7, subtext, &paint);
    }
}
