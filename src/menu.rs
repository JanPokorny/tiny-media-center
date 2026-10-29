// The scrolling text menu. Fade falloff is evaluated per item (at its
// center), which reads identically at these fade band sizes.

use crate::config::{color, ACCENT_COLOR, FONT_SIZE, TEXT_COLOR};
use femtovg::{Baseline, Canvas, Color, FontId, Paint, Renderer};

pub fn text_width<R: Renderer>(canvas: &Canvas<R>, text: &str, paint: &Paint) -> f32 {
    canvas.measure_text(0.0, 0.0, text, paint).map_or(0.0, |m| m.width())
}

#[derive(Clone)]
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
        self.update_items(items, selected);
        self.reset_scroll();
    }

    // Replace the items in place (background refresh), keeping the scroll
    // position instead of replaying the slide-in.
    pub fn update_items(&mut self, items: Vec<MenuItem<A>>, selected: usize) {
        self.items = items;
        self.selected = selected;
    }

    // wrap: step past either end to the other (fresh presses only).
    pub fn navigate_up(&mut self, wrap: bool) {
        self.selected = match self.selected {
            0 if wrap && !self.items.is_empty() => self.items.len() - 1,
            n => n.saturating_sub(1),
        };
    }

    pub fn navigate_down(&mut self, wrap: bool) {
        let last = self.items.len().saturating_sub(1);
        self.selected = if self.selected >= last && wrap { 0 } else { (self.selected + 1).min(last) };
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

    // Index of the first item of the run of items sharing i's first letter.
    fn group_start(&self, i: usize) -> usize {
        let key = |i: usize| self.items[i].jump.chars().next();
        (0..=i).rev().take_while(|&j| key(j) == key(i)).last().unwrap_or(i)
    }

    // Right: first item of the next letter group. Left: first item of the
    // current group, or of the previous one when already on it. wrap: step
    // past either end to the other (fresh presses only).
    pub fn jump_group(&mut self, forward: bool, wrap: bool) {
        let Some(last) = self.items.len().checked_sub(1) else { return };
        let start = self.group_start(self.selected);
        self.selected = if forward {
            let key = |i: usize| self.items[i].jump.chars().next();
            match (self.selected..=last).find(|&i| key(i) != key(self.selected)) {
                Some(next) => next,
                None if wrap => 0,
                None => self.selected,
            }
        } else if self.selected > start {
            start
        } else if start > 0 {
            self.group_start(start - 1)
        } else if wrap {
            self.group_start(last)
        } else {
            self.selected
        };
    }

    pub fn draw<R: Renderer>(
        &mut self,
        canvas: &mut Canvas<R>,
        font: FontId,
        dt: f32,
        w: f32,
        h: f32,
    ) {
        let line_h = FONT_SIZE * 1.125;

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

        // Fade bands: transparent above the header,
        // fading in down to 30% of screen height, fading out over the bottom
        // 30%. Applied only to lists that actually scroll -- a short centered
        // list has no off-screen continuation to hint at.
        let header_h = line_h * 1.2;
        let (fade_top, fade_bot) = (h * 0.3 - header_h, h * 0.3);

        let paint = Paint::color(Color::white())
            .with_font(&[font])
            .with_font_size(FONT_SIZE)
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

            let rgb = if focused { ACCENT_COLOR } else { TEXT_COLOR };
            let alpha = if item.dim && !focused { 0.3 } else { 1.0 } * fade;
            let paint = paint.clone().with_color(color(rgb, alpha));

            let x = 50.0;
            let prefix = if focused { "> " } else { "  " };
            let _ = canvas.fill_text(x, item_y, prefix, &paint);

            // Squish the focused label horizontally when it overflows.
            let text_x = x + text_width(canvas, prefix, &paint);
            let max_width = w - text_x;
            let label_width = text_width(canvas, &item.label, &paint);
            let target_scale = if focused && label_width > max_width { max_width / label_width } else { 1.0 };
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
    fn navigation_wraps_only_when_asked() {
        let mut m = menu(&["a", "b", "c"]);
        m.navigate_up(false);
        assert_eq!(m.selected, 0);
        m.navigate_up(true);
        assert_eq!(m.selected, 2);
        m.navigate_down(false);
        assert_eq!(m.selected, 2);
        m.navigate_down(true);
        assert_eq!(m.selected, 0);
        let mut empty = menu(&[]);
        empty.navigate_up(true);
        empty.navigate_down(true);
        assert_eq!(empty.selected, 0);
    }

    #[test]
    fn left_right_jump_between_letter_groups() {
        let mut m = menu(&["a1", "a2", "b1", "b2", "b3", "c1"]);
        m.items.iter_mut().for_each(|i| i.jump = i.label.clone());
        m.jump_group(true, true);
        assert_eq!(m.selected, 2); // a -> b
        m.selected = 3;
        m.jump_group(false, true);
        assert_eq!(m.selected, 2); // inside a group: its first item
        m.jump_group(false, true);
        assert_eq!(m.selected, 0); // on the first item: previous group
        m.jump_group(false, false);
        assert_eq!(m.selected, 0); // autorepeat doesn't wrap
        m.jump_group(false, true);
        assert_eq!(m.selected, 5); // fresh press wraps to the last group
        m.jump_group(true, false);
        assert_eq!(m.selected, 5);
        m.jump_group(true, true);
        assert_eq!(m.selected, 0);
        menu(&[]).jump_group(true, true);
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
}
