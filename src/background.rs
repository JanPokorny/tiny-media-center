// Port of components/background.lua: drifting starfield with additive
// proximity lines.

use femtovg::{Canvas, Color, CompositeOperation, Paint, Path, Renderer};

struct Star {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    size: f32,
}

pub struct Background {
    speed: f32,
    stars: Vec<Star>,
}

impl Background {
    pub fn new(w: f32, h: f32) -> Background {
        let stars = (0..(w * h / 10000.0) as usize)
            .map(|_| Star {
                x: fastrand::f32() * w,
                y: fastrand::f32() * h,
                vx: 2.0 * fastrand::f32() - 1.0,
                vy: 2.0 * fastrand::f32() - 1.0,
                size: fastrand::u32(1..=3) as f32,
            })
            .collect();
        Background { speed: h / 70.0, stars }
    }

    pub fn draw<R: Renderer>(&mut self, canvas: &mut Canvas<R>, dt: f32, w: f32, h: f32) {
        let mut dots = Path::new();
        for star in &mut self.stars {
            star.x = (50.0 + star.x + star.vx * dt * self.speed).rem_euclid(w + 100.0) - 50.0;
            star.y = (50.0 + star.y + star.vy * dt * self.speed).rem_euclid(h + 100.0) - 50.0;
            dots.circle(star.x, star.y, star.size);
        }

        canvas.global_composite_operation(CompositeOperation::Lighter);
        canvas.fill_path(&dots, &Paint::color(Color::rgbaf(0.5, 0.6, 0.7, 0.4)));

        let connection_dist_sq = (h / 8.0) * (h / 8.0);
        for (i, s1) in self.stars.iter().enumerate() {
            for s2 in &self.stars[i + 1..] {
                let (dx, dy) = (s1.x - s2.x, s1.y - s2.y);
                let dist_sq = dx * dx + dy * dy;
                if dist_sq < connection_dist_sq {
                    let alpha = (1.0 - dist_sq / connection_dist_sq) * 0.2;
                    let mut line = Path::new();
                    line.move_to(s1.x, s1.y);
                    line.line_to(s2.x, s2.y);
                    canvas.stroke_path(
                        &line,
                        &Paint::color(Color::rgbaf(0.2, 0.5, 0.6, alpha)).with_line_width(1.0),
                    );
                }
            }
        }
        canvas.global_composite_operation(CompositeOperation::SourceOver);
    }
}
