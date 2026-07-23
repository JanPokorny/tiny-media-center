// SDL/mpv setup, the main loop, and all drawing (menus, loading screen, seek
// overlay, watch progress). Navigation and playback state live in app.rs.

mod app;
mod background;
mod config;
mod input;
mod media;
mod menu;
mod player;

use app::{App, Mode};
use background::Background;
use config::color;
use femtovg::{renderer::OpenGl, Baseline, Canvas, FontId, Paint, Path, Renderer};
use menu::text_width;
use player::Player;
use sdl3::event::Event;
use sdl3::keyboard::{Keycode, Mod};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

const FONT: &[u8] = include_bytes!("../attachments/mpv/subfont.ttf");

// Video timestamp as h:mm:ss (or m:ss under an hour) for the seek overlay.
fn fmt_time(secs: f64) -> String {
    let s = secs.max(0.0) as i64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

// Draw text horizontally centered, with its top edge at y.
fn draw_centered<R: Renderer>(canvas: &mut Canvas<R>, text: &str, y: f32, wf: f32, paint: &Paint) {
    let text_w = text_width(canvas, text, paint);
    let _ = canvas.fill_text((wf - text_w) / 2.0, y, text, paint);
}

// Bottom progress bar with a centered line of text above it: the seek
// overlay while playing, the watch progress + remaining time while browsing.
#[allow(clippy::too_many_arguments)]
fn draw_progress<R: Renderer>(
    canvas: &mut Canvas<R>,
    style: &config::Style,
    font: FontId,
    frac: f32,
    text: &str,
    text_color: [f32; 3],
    alpha: f32,
    wf: f32,
    hf: f32,
) {
    let bar_height = 4.0;
    let mut bar = Path::new();
    bar.rect(0.0, hf - bar_height, wf, bar_height);
    canvas.fill_path(&bar, &Paint::color(color(style.text_color, 0.2 * alpha)));
    if frac > 0.0 {
        let mut fill = Path::new();
        fill.rect(0.0, hf - bar_height, wf * frac.min(1.0), bar_height);
        canvas.fill_path(&fill, &Paint::color(color(style.accent_color, alpha)));
    }
    if !text.is_empty() {
        let paint = Paint::color(color(text_color, alpha))
            .with_font(&[font])
            .with_font_size(style.font_size)
            .with_text_baseline(Baseline::Top);
        draw_centered(canvas, text, hf - bar_height - style.font_size * 1.2, wf, &paint);
    }
}

fn main() {
    let config = config::load();

    // Write the embedded mpv attachment files to the config dir. mpv.conf is
    // only written when missing, so user edits to it survive restarts; the
    // script and font are app internals and stay in sync with the binary.
    let mpv_dir = config::config_dir().join("mpv");
    let _ = std::fs::create_dir_all(&mpv_dir);
    let mpv_conf = mpv_dir.join("mpv.conf");
    if !mpv_conf.exists() {
        let _ = std::fs::write(&mpv_conf, include_bytes!("../attachments/mpv/mpv.conf"));
    }
    let _ = std::fs::write(mpv_dir.join("subfont.ttf"), FONT);
    // Left behind by older versions (the audio visualiser script).
    let _ = std::fs::remove_file(mpv_dir.join("visualiser.lua"));

    let sdl = sdl3::init().unwrap();
    let video = sdl.video().unwrap();
    // femtovg fills concave paths (e.g. the starfield dots) via the stencil
    // buffer; without one the fill floods the path's bounding box, tinting
    // the whole background. SDL defaults to no stencil bits.
    video.gl_attr().set_stencil_size(8);
    let gamepad_subsystem = sdl.gamepad().unwrap();
    let window = video
        .window("tiny media center", 1920, 1080)
        .fullscreen()
        .opengl()
        .build()
        .unwrap();
    let _gl_context = window.gl_create_context().unwrap();
    let _ = video.gl_set_swap_interval(1);
    sdl.mouse().show_cursor(false);

    let renderer = unsafe {
        OpenGl::new_from_function(|s| player::get_proc_address(&video, s).cast_const())
            .expect("femtovg renderer")
    };
    let mut canvas = Canvas::new(renderer).expect("femtovg canvas");
    let font = canvas.add_font_mem(FONT).expect("font");

    let player = Player::new(&video, &mpv_dir.to_string_lossy()).expect("mpv");

    let (tx, rx) = channel();
    media::spawn_scan(config.media_path.clone(), tx);
    let mut app = App::new(config, player, rx);

    let (w, h) = window.size();
    let mut background = Background::new(w as f32, h as f32);
    let mut scroll_buffer = 0.0_f32;
    let mut gamepads = vec![];
    let mut event_pump = sdl.event_pump().unwrap();
    let mut last_frame = Instant::now();

    'running: loop {
        let busy = matches!(app.mode, Mode::Scanning | Mode::External { .. });
        let playing = matches!(app.mode, Mode::Playing { .. });

        for event in event_pump.poll_iter() {
            match event {
                Event::Quit { .. } => break 'running,
                Event::ControllerDeviceAdded { which, .. } => {
                    if let Ok(gamepad) = gamepad_subsystem.open(which) {
                        gamepads.push(gamepad);
                    }
                }
                Event::KeyDown { keycode: Some(Keycode::R), keymod, .. }
                    if keymod.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD) =>
                {
                    // Restart in place (re-exec the current binary).
                    use std::os::unix::process::CommandExt;
                    let exe = std::env::current_exe().expect("current_exe");
                    let err = std::process::Command::new(exe).exec();
                    panic!("restart failed: {err}");
                }
                _ if busy => {}
                Event::MouseWheel { y, .. } if !playing => {
                    // One wheel notch (y = ±1) per menu step; the buffer
                    // accumulates fractional high-resolution scrolls.
                    if (y > 0.0) != (scroll_buffer > 0.0) {
                        scroll_buffer = 0.0;
                    }
                    scroll_buffer += y;
                    while scroll_buffer >= 1.0 {
                        app.menu().navigate_up();
                        scroll_buffer -= 1.0;
                    }
                    while scroll_buffer <= -1.0 {
                        app.menu().navigate_down();
                        scroll_buffer += 1.0;
                    }
                }
                event => {
                    if let Some(input) = input::translate(&event) {
                        app.handle_input(input);
                    }
                }
            }
        }

        app.pump_scan();
        app.pump_external();
        if app.notice.as_ref().is_some_and(|(_, since)| since.elapsed() > Duration::from_secs(5)) {
            app.notice = None;
        }

        let dt = last_frame.elapsed().as_secs_f32();
        last_frame = Instant::now();
        let (w, h) = window.size();
        let (wf, hf) = (w as f32, h as f32);

        if let &Mode::Playing { osd_until } = &app.mode {
            if app.player.poll_ended() {
                // The file went away under us (stop/error): save and close.
                app.save_position();
                app.close_player();
            } else if app.player.at_eof() {
                // keep-open paused on the last frame; land in the menu state.
                app.pause_player();
            } else {
                app.autosave_position();
                app.player.render(w as i32, h as i32);
                canvas.set_size(w, h, 1.0);
                let style = &app.config.style;
                // Audio files render no video: gray "now playing" in the
                // corner, the file name centered in white.
                if let Some(name) = app.audio_name() {
                    canvas.clear_rect(0, 0, w, h, color(style.background_color, 1.0));
                    let paint = Paint::color(color(style.dim_color, 1.0))
                        .with_font(&[font])
                        .with_font_size(style.font_size)
                        .with_text_baseline(Baseline::Top);
                    let _ = canvas.fill_text(0.0, 0.0, "now playing", &paint);
                    let paint = paint.with_color(color(style.text_color, 1.0));
                    draw_centered(&mut canvas, name, (hf - style.font_size) / 2.0, wf, &paint);
                }
                // Seek overlay: progress bar + current time, fading out over
                // the last moments before osd_until.
                let osd = osd_until.saturating_duration_since(Instant::now()).as_secs_f32();
                if osd > 0.0 {
                    let alpha = (osd / 0.3).min(1.0);
                    let duration = app.player.duration();
                    let frac = if duration > 0.0 {
                        (app.player.time_pos / duration).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    let time_text =
                        format!("{} / {}", fmt_time(app.player.time_pos), fmt_time(duration));
                    draw_progress(&mut canvas, style, font, frac, &time_text, style.text_color, alpha, wf, hf);
                }
                canvas.flush();
                window.gl_swap_window();
                continue;
            }
        } else if matches!(app.mode, Mode::Paused) && app.player.poll_ended() {
            // Only stop/error events can arrive while paused (a failed load).
            app.close_player();
        }

        let paused = matches!(app.mode, Mode::Paused);
        if paused {
            // The loaded video's still frame is the backdrop; femtovg draws
            // the overlay and menu on top of it at flush.
            app.player.render(w as i32, h as i32);
        }
        canvas.set_size(w, h, 1.0);
        if !paused {
            canvas.clear_rect(0, 0, w, h, color(app.config.style.background_color, 1.0));
        }
        let dim_paint = Paint::color(color(app.config.style.dim_color, 1.0))
            .with_font(&[font])
            .with_font_size(app.config.style.font_size)
            .with_text_baseline(Baseline::Top);

        let loading_text = match &app.mode {
            Mode::Scanning => Some("Processing new media..."),
            Mode::External { text, .. } => Some(text.as_str()),
            _ => None,
        };
        if let Some(text) = loading_text {
            let style = &app.config.style;
            let size = style.font_size;
            let paint = dim_paint.clone().with_color(color(style.text_color, 1.0));
            draw_centered(&mut canvas, text, (hf - size) / 2.0 - size * 0.7, wf, &paint);
            let subtext = &app.scan_status;
            if !subtext.is_empty() {
                let paint = dim_paint.clone().with_font_size(size * 0.5);
                draw_centered(&mut canvas, subtext, (hf - size) / 2.0 + size * 0.7, wf, &paint);
            }
        } else {
            // Browse/paused: starfield + menu + title + watch progress bar +
            // remaining-time line. While paused, the starfield is replaced
            // by the video frame dimmed by a semitransparent black overlay.
            if paused {
                let mut overlay = Path::new();
                overlay.rect(0.0, 0.0, wf, hf);
                canvas.fill_path(&overlay, &Paint::color(color([0.0, 0.0, 0.0], 0.6)));
            } else {
                background.draw(&mut canvas, dt, wf, hf);
            }
            {
                // Split borrows: the menu draws mutably (scroll/squish
                // animation state) while the config is read.
                let App { stack, config, .. } = &mut app;
                stack.last_mut().unwrap().menu_mut().draw(&mut canvas, font, config, dt, wf, hf);
            }

            let _ = canvas.fill_text(0.0, 0.0, app.title(), &dim_paint);

            let style = &app.config.style;
            if let Some(video) = app.video_node() {
                let pct = media::watch_pct(video);
                let time_text = video
                    .meta
                    .duration
                    .map(|duration| {
                        let remaining = duration - video.meta.position.unwrap_or(0.0);
                        let now = chrono::Local::now();
                        let end_time = now + chrono::Duration::seconds(remaining as i64);
                        let mins = (remaining / 60.0).floor() as i64;
                        let (hours, mins) = (mins / 60, mins % 60);
                        let rem_str =
                            if hours > 0 { format!("{hours}h{mins:02}min") } else { format!("{mins}min") };
                        format!("{}  --  {}  ->  {}", now.format("%H:%M"), rem_str, end_time.format("%H:%M"))
                    })
                    .unwrap_or_default();
                draw_progress(&mut canvas, style, font, pct as f32 / 100.0, &time_text, style.dim_color, 1.0, wf, hf);
            }

            // Top-right corner: external-command failures (accented), or the
            // file the background scan is currently probing.
            let corner = match &app.notice {
                Some((message, _)) => Some((message.clone(), style.accent_color)),
                None if !app.scan_status.is_empty() => {
                    Some((format!("processing {}", app.scan_status), style.dim_color))
                }
                None => None,
            };
            if let Some((text, rgb)) = corner {
                let paint = dim_paint
                    .clone()
                    .with_color(color(rgb, 1.0))
                    .with_font_size(style.font_size * 0.4);
                let text_w = text_width(&canvas, &text, &paint);
                let _ = canvas.fill_text(wf - text_w - 10.0, 10.0, &text, &paint);
            }
        }

        canvas.flush();
        window.gl_swap_window();
    }
}

#[cfg(test)]
mod tests {
    use super::fmt_time;

    #[test]
    fn fmt_time_cases() {
        assert_eq!(fmt_time(0.0), "0:00");
        assert_eq!(fmt_time(65.0), "1:05");
        assert_eq!(fmt_time(3600.0), "1:00:00");
        assert_eq!(fmt_time(3725.9), "1:02:05");
        assert_eq!(fmt_time(-5.0), "0:00");
    }
}
