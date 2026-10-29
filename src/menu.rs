// The scrolling text menu. Fade falloff is evaluated per item (at its
// center), which reads identically at these fade band sizes.

use crate::config::{color, ACCENT_COLOR, DIM_COLOR, FONT_SIZE, TEXT_COLOR};
use femtovg::{Baseline, Canvas, Color, FontId, Paint, Path, Renderer};

pub fn text_width<R: Renderer>(canvas: &Canvas<R>, text: &str, paint: &Paint) -> f32 {
    canvas.measure_text(0.0, 0.0, text, paint).map_or(0.0, |m| m.width())
}

// Text with a dark drop shadow (alpha: the paint's), readable over bright video.
pub fn draw_text<R: Renderer>(canvas: &mut Canvas<R>, x: f32, y: f32, text: &str, paint: &Paint, alpha: f32) {
    let shadow = paint.clone().with_color(Color::rgbaf(0.0, 0.0, 0.0, 0.6 * alpha));
    let _ = canvas.fill_text(x + 3.0, y + 3.0, text, &shadow);
    let _ = canvas.fill_text(x, y, text, paint);
}

#[derive(Clone)]
pub struct MenuItem<A> {
    pub label: String,
    // Lowercased label, the key for jump-to-letter and sorting.
    pub jump: String,
    pub dim: bool,
    // A small section title: never selected, jumped over by every navigation.
    pub heading: bool,
    pub action: A,
    squish: f32,
}

impl<A> MenuItem<A> {
    pub fn new(label: String, action: A) -> MenuItem<A> {
        let jump = label.to_lowercase();
        MenuItem { label, jump, dim: false, heading: false, action, squish: 1.0 }
    }
}

impl<A: Default> MenuItem<A> {
    pub fn heading(label: &str) -> MenuItem<A> {
        MenuItem { heading: true, jump: String::new(), ..MenuItem::new(label.into(), A::default()) }
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
        let mut menu = Menu { items: vec![], selected, scroll_offset: f32::NAN };
        menu.update_items(items, selected);
        menu
    }

    pub fn reset_scroll(&mut self) {
        self.scroll_offset = f32::NAN;
    }

    pub fn set_items(&mut self, items: Vec<MenuItem<A>>, selected: usize) {
        self.update_items(items, selected);
        self.reset_scroll();
    }

    // Replace the items in place (background refresh), keeping the scroll
    // position instead of replaying the slide-in. A selection on a heading
    // moves to the nearest item after it, else before.
    pub fn update_items(&mut self, items: Vec<MenuItem<A>>, selected: usize) {
        self.items = items;
        self.selected = selected;
        if self.items.get(selected).is_some_and(|i| i.heading) {
            let ok = |i: &usize| !self.items[*i].heading;
            let after = (selected..self.items.len()).find(ok);
            self.selected = after.or_else(|| (0..selected).rev().find(ok)).unwrap_or(selected);
        }
    }

    // wrap: step past either end to the other (fresh presses only).
    pub fn navigate(&mut self, down: bool, wrap: bool) {
        let ok = |i: &usize| !self.items[*i].heading;
        let (before, after) = (0..self.selected, self.selected + 1..self.items.len());
        let next = if down {
            after.clone().find(ok).or_else(|| before.clone().find(ok).filter(|_| wrap))
        } else {
            before.clone().rev().find(ok).or_else(|| after.rev().find(ok).filter(|_| wrap))
        };
        self.selected = next.unwrap_or(self.selected);
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
        let ok = |i: &usize| !self.items[*i].heading;
        let n = self.items.len();
        let (Some(first), Some(last)) = ((0..n).find(ok), (0..n).rev().find(ok)) else { return };
        let start = self.group_start(self.selected);
        self.selected = if forward {
            let key = |i: usize| self.items[i].jump.chars().next();
            match (self.selected..=last).find(|&i| ok(&i) && key(i) != key(self.selected)) {
                Some(next) => next,
                None if wrap => first,
                None => self.selected,
            }
        } else if self.selected > start {
            start
        } else if let Some(prev) = (0..start).rev().find(ok) {
            self.group_start(prev)
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
        let heights: Vec<f32> = self.items.iter().map(|i| if i.heading { line_h * 0.75 } else { line_h }).collect();
        let tops: Vec<f32> = heights.iter().scan(0.0, |top, h| Some(std::mem::replace(top, *top + h))).collect();
        let list_height: f32 = heights.iter().sum();
        let scrolling = list_height > h * 0.8;
        let target = if scrolling {
            tops.get(self.selected).copied().unwrap_or(0.0)
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
            let (item_y, item_h) = (h / 2.0 - self.scroll_offset + tops[i], heights[i]);
            if item_y <= -line_h || item_y >= h {
                continue;
            }
            let focused = i == selected;

            let fade = if scrolling {
                let y_mid = item_y + item_h / 2.0;
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
            if item.heading {
                // Small dim title at the bottom of its row, a thin line after it.
                let small = paint.clone().with_font_size(FONT_SIZE * 0.45).with_color(color(DIM_COLOR, fade));
                let text_x = x + text_width(canvas, prefix, &paint);
                let text_y = item_y + item_h - FONT_SIZE * 0.45 * 1.15;
                draw_text(canvas, text_x, text_y, &item.label, &small, fade);
                let line_x = text_x + text_width(canvas, &item.label, &small) + 20.0;
                let mut line = Path::new();
                line.rect(line_x, text_y + FONT_SIZE * 0.45 * 0.55, (w - x - line_x).max(0.0), 2.0);
                canvas.fill_path(&line, &Paint::color(color(DIM_COLOR, 0.4 * fade)));
                continue;
            }
            draw_text(canvas, x, item_y, prefix, &paint, alpha);

            // Squish the focused label horizontally when it overflows.
            let text_x = x + text_width(canvas, prefix, &paint);
            let max_width = w - text_x;
            let label_width = text_width(canvas, &item.label, &paint);
            let target_scale = if focused && label_width > max_width { max_width / label_width } else { 1.0 };
            item.squish += (target_scale - item.squish) * (8.0 * dt).min(1.0);

            canvas.save();
            canvas.translate(text_x, item_y);
            canvas.scale(item.squish, 1.0);
            draw_text(canvas, 0.0, 0.0, &item.label, &paint, alpha);
            canvas.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // "-" makes a heading.
    fn menu(names: &[&str]) -> Menu<()> {
        let item = |n: &&str| if *n == "-" { MenuItem::heading("h") } else { MenuItem::new(n.to_string(), ()) };
        Menu::new(names.iter().map(item).collect(), 0)
    }

    #[test]
    fn navigation_wraps_only_when_asked() {
        let mut m = menu(&["a", "b", "c"]);
        m.navigate(false, false);
        assert_eq!(m.selected, 0);
        m.navigate(false, true);
        assert_eq!(m.selected, 2);
        m.navigate(true, false);
        assert_eq!(m.selected, 2);
        m.navigate(true, true);
        assert_eq!(m.selected, 0);
        let mut empty = menu(&[]);
        empty.navigate(false, true);
        empty.navigate(true, true);
        assert_eq!(empty.selected, 0);
    }

    #[test]
    fn navigation_skips_headings() {
        let mut m = menu(&["a", "-", "b", "-", "c"]);
        m.navigate(true, false);
        assert_eq!(m.selected, 2);
        m.navigate(true, false);
        m.navigate(true, false);
        assert_eq!(m.selected, 4); // no wrap
        m.navigate(true, true);
        assert_eq!(m.selected, 0);
        m.navigate(false, false);
        assert_eq!(m.selected, 0);
        m.navigate(false, true);
        assert_eq!(m.selected, 4);
        m.navigate(false, false);
        assert_eq!(m.selected, 2);
        // Leading/trailing headings are skipped when wrapping too.
        let mut m = menu(&["-", "a", "b", "-"]);
        assert_eq!(m.selected, 1); // the initial selection settles
        m.navigate(false, true);
        assert_eq!(m.selected, 2);
        m.navigate(true, true);
        assert_eq!(m.selected, 1);
        // Nothing selectable: stays put.
        let mut m = menu(&["-", "-"]);
        m.navigate(true, true);
        m.navigate(false, true);
        m.jump_group(true, true);
        m.jump_to_letter('a');
        assert_eq!(m.selected, 0);
        // Selection that lands on a heading after an update moves on.
        let mut m = menu(&["a", "b"]);
        m.update_items(menu(&["a", "-", "b"]).items, 1);
        assert_eq!(m.selected, 2);
        m.update_items(menu(&["a", "-"]).items, 1);
        assert_eq!(m.selected, 0);
    }

    #[test]
    fn jump_group_skips_headings() {
        let mut m = menu(&["a1", "-", "b1", "-", "c1"]);
        m.jump_group(true, true);
        assert_eq!(m.selected, 2);
        m.jump_group(false, true);
        assert_eq!(m.selected, 0); // the previous group, not the heading
        m.jump_group(false, true);
        assert_eq!(m.selected, 4);
        m.jump_group(true, true);
        assert_eq!(m.selected, 0);
        m.jump_to_letter('c');
        assert_eq!(m.selected, 4);
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
