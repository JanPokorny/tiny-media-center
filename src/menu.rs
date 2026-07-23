// The scrolling text menu. The GLSL fade shader of the Lua version is
// replaced by the same falloff formula evaluated per item (at its center),
// which reads identically at these fade band sizes.

use crate::config::{color, Config};
use femtovg::{Baseline, Canvas, Color, FontId, Paint, Renderer};

pub struct MenuItem<A> {
    pub label: String,
    // Lowercased key used for jump-to-letter and sorting; defaults to the
    // label, but e.g. the "* " in-progress prefix must not take part.
    pub jump: String,
    pub dim: bool,
    pub action: A,
    squish: f32,
}

impl<A> MenuItem<A> {
    pub fn new(label: String, action: A) -> MenuItem<A> {
        let jump = label.to_lowercase();
        MenuItem { label, jump, dim: false, action, squish: 1.0 }
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

    // Replace the items in place (background refresh), keeping the scroll
    // position instead of replaying the slide-in.
    pub fn update_items(&mut self, items: Vec<MenuItem<A>>, selected: usize) {
        self.items = items;
        self.selected = selected;
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
            if self.items[index].jump.starts_with(key) {
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
        let scrolling = list_height > h * 0.8;
        let target = if scrolling {
            self.selected as f32 * line_h
        } else {
            (list_height - line_h) / 2.0
        };
        if self.scroll_offset.is_nan() {
            self.scroll_offset = target - line_h * 0.5;
        }
        self.scroll_offset += (target - self.scroll_offset) * (10.0 * dt).min(1.0);

        // Fade bands as in the menu.lua shader: transparent above the header,
        // fading in down to 30% of screen height, fading out over the bottom
        // 30%. Applied only to lists that actually scroll -- a short centered
        // list has no off-screen continuation to hint at.
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

            let fade = if scrolling {
                let y_mid = item_y + line_h / 2.0;
                let mut fade = ((y_mid - header_h) / fade_top).min(1.0);
                if y_mid > h - fade_bot {
                    fade = fade.min((h - y_mid) / fade_bot);
                }
                fade.clamp(0.0, 1.0)
            } else {
                1.0
            };

            let rgb = if focused { config.style.accent_color } else { config.style.text_color };
            let alpha = if item.dim && !focused { 0.3 } else { 1.0 } * fade;
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

#[cfg(test)]
mod tests {
    use super::*;

    fn menu(names: &[&str]) -> Menu<()> {
        Menu::new(names.iter().map(|n| MenuItem::new(n.to_string(), ())).collect(), 0)
    }

    #[test]
    fn jump_ignores_progress_prefix() {
        let mut m = menu(&["Bravo", "* Alien"]);
        // The jump key is set from the display name, not the "* " label.
        m.items[1].jump = "alien".into();
        m.jump_to_letter('a');
        assert_eq!(m.selected, 1);
    }

    #[test]
    fn jump_wraps_and_is_case_insensitive() {
        let mut m = menu(&["Alpha", "Bravo", "Avocado"]);
        m.jump_to_letter('a');
        assert_eq!(m.selected, 2); // starts searching after the selection
        m.jump_to_letter('a');
        assert_eq!(m.selected, 0); // wraps around
        m.jump_to_letter('x');
        assert_eq!(m.selected, 0); // no match: stay put
    }

    #[test]
    fn navigation_stays_in_bounds() {
        let mut m = menu(&["a", "b"]);
        m.navigate_up();
        assert_eq!(m.selected, 0);
        m.navigate_down();
        m.navigate_down();
        assert_eq!(m.selected, 1);

        let mut empty = menu(&[]);
        empty.navigate_down();
        assert_eq!(empty.selected, 0);
    }
}
