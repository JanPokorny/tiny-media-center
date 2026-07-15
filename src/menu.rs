// Port of components/menu.lua + components/menu_item.lua. The GLSL fade
// shader is replaced by the same falloff formula evaluated per item
// (at its center), which reads identically at these fade band sizes.

use crate::config::{color, Config};
use femtovg::{Baseline, Canvas, Color, FontId, Paint, Renderer};

pub struct MenuItem<A> {
    pub label: String,
    pub dim: bool,
    pub action: A,
    squish: f32,
}

impl<A> MenuItem<A> {
    pub fn new(label: String, action: A) -> MenuItem<A> {
        MenuItem { label, dim: false, action, squish: 1.0 }
    }
}

pub struct Menu<A> {
    pub items: Vec<MenuItem<A>>,
    pub selected: usize,
    // NAN marks a pending reset_scroll, resolved on the next draw once
    // layout (line height, screen height) is at hand.
    scroll_offset: f32,
}

impl<A> Menu<A> {
    pub fn new(items: Vec<MenuItem<A>>, selected: usize) -> Menu<A> {
        Menu { items, selected, scroll_offset: f32::NAN }
    }

    pub fn reset_scroll(&mut self) {
        self.scroll_offset = f32::NAN;
    }

    pub fn set_items(&mut self, items: Vec<MenuItem<A>>, selected: usize) {
        self.items = items;
        self.selected = selected;
        self.scroll_offset = f32::NAN;
    }

    pub fn navigate_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn navigate_down(&mut self) {
        if !self.items.is_empty() {
            self.selected = (self.selected + 1).min(self.items.len() - 1);
        }
    }

    pub fn jump_to_letter(&mut self, key: char) {
        for i in 0..self.items.len() {
            let index = (self.selected + 1 + i) % self.items.len();
            if self.items[index].label.to_lowercase().starts_with(key) {
                self.selected = index;
                break;
            }
        }
    }

    pub fn draw<R: Renderer>(
        &mut self,
        canvas: &mut Canvas<R>,
        font: FontId,
        config: &Config,
        dt: f32,
        w: f32,
        h: f32,
    ) {
        let line_h = config.style.font_size * 1.125;

        // Scroll towards the selected item (or the centered list position for
        // short lists); a pending reset starts half a line above the target
        // for the slide-in effect.
        let list_height = self.items.len() as f32 * line_h;
        let target = if list_height <= h * 0.8 {
            (list_height - line_h) / 2.0
        } else {
            self.selected as f32 * line_h
        };
        if self.scroll_offset.is_nan() {
            self.scroll_offset = target - line_h * 0.5;
        }
        self.scroll_offset += (target - self.scroll_offset) * 10.0 * dt;

        // Fade bands as in the menu.lua shader: transparent above the header,
        // fading in down to 30% of screen height, fading out over the bottom 30%.
        let header_h = line_h * 1.2;
        let (fade_top, fade_bot) = (h * 0.3 - header_h, h * 0.3);

        let paint = Paint::color(Color::white())
            .with_font(&[font])
            .with_font_size(config.style.font_size)
            .with_text_baseline(Baseline::Top);

        let selected = self.selected;
        for (i, item) in self.items.iter_mut().enumerate() {
            let item_y = h / 2.0 - self.scroll_offset + i as f32 * line_h;
            if item_y <= -line_h || item_y >= h {
                continue;
            }
            let focused = i == selected;

            let y_mid = item_y + line_h / 2.0;
            let mut fade = ((y_mid - header_h) / fade_top).min(1.0);
            if y_mid > h - fade_bot {
                fade = fade.min((h - y_mid) / fade_bot);
            }

            let rgb = if focused { config.style.accent_color } else { config.style.text_color };
            let alpha = if item.dim && !focused { 0.3 } else { 1.0 } * fade.clamp(0.0, 1.0);
            let paint = paint.clone().with_color(color(rgb, alpha));

            let x = 50.0;
            let prefix = if focused { "> " } else { "  " };
            let prefix_width = canvas.measure_text(0.0, 0.0, prefix, &paint).map_or(0.0, |m| m.width());
            let _ = canvas.fill_text(x, item_y, prefix, &paint);

            // Squish the focused label horizontally when it overflows.
            let text_x = x + prefix_width;
            let max_width = w - text_x;
            let text_width = canvas.measure_text(0.0, 0.0, &item.label, &paint).map_or(0.0, |m| m.width());
            let target_scale = if focused && text_width > max_width { max_width / text_width } else { 1.0 };
            item.squish += (target_scale - item.squish) * (8.0 * dt).min(1.0);

            canvas.save();
            canvas.translate(text_x, item_y);
            canvas.scale(item.squish, 1.0);
            let _ = canvas.fill_text(0.0, 0.0, &item.label, &paint);
            canvas.restore();
        }
    }
}
