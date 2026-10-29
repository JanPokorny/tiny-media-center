// SDL/mpv setup, the main loop, and all drawing (menus, loading screen, seek
// overlay, watch progress). Navigation and playback state live in app.rs.

mod app;
mod audio;
mod background;
mod bluetooth;
mod config;
mod input;
mod media;
mod menu;
mod overlay;
mod player;
mod subtitles;
mod windows;

use app::{App, Mode};
use background::Background;
use config::{color, ACCENT_COLOR, BACKGROUND_COLOR, DIM_COLOR, FONT_SIZE, MEDIA_PATH, TEXT_COLOR};
use femtovg::{renderer::OpenGl, Baseline, Canvas, FontId, Paint, Path, Renderer};
use menu::text_width;
use player::Player;
use sdl3::event::{Event, WindowEvent};
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
    canvas.fill_path(&bar, &Paint::color(color(TEXT_COLOR, 0.2 * alpha)));
    if frac > 0.0 {
        let mut fill = Path::new();
        fill.rect(0.0, hf - bar_height, wf * frac.min(1.0), bar_height);
        canvas.fill_path(&fill, &Paint::color(color(ACCENT_COLOR, alpha)));
    }
    if !text.is_empty() {
        let paint = Paint::color(color(text_color, alpha))
            .with_font(&[font])
            .with_font_size(FONT_SIZE)
            .with_text_baseline(Baseline::Top);
        draw_centered(canvas, text, hf - bar_height - FONT_SIZE * 1.2, wf, &paint);
    }
}

#[link(name = "X11")]
extern "C" {
    fn XSetIOErrorHandler(handler: extern "C" fn(*mut std::ffi::c_void) -> i32) -> *mut std::ffi::c_void;
}

extern "C" {
    fn _exit(status: i32) -> !;
}

// Xlib's default handler calls exit() from whichever thread lost the X
// connection. mpv's clipboard thread has its own connection, so two threads
// race into exit() and deadlock (libGLX's destructor waits for a display
// lock held by the other dying thread). _exit skips the destructors.
extern "C" fn x_io_error(_display: *mut std::ffi::c_void) -> i32 {
    eprintln!("X connection lost");
    unsafe { _exit(1) }
}

fn main() {
    unsafe { XSetIOErrorHandler(x_io_error) };
    // Before SDL looks for a window manager and maps its window.
    let windows = windows::start();
    let subtitles = config::load();
    input::watch_controller_chord();

    // mpv finds the subtitle/OSD font as subfont.ttf in its config dir (~~).
    let mpv_dir = config::config_dir().join("mpv");
    let _ = std::fs::create_dir_all(&mpv_dir);
    let _ = std::fs::write(mpv_dir.join("subfont.ttf"), FONT);

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
    let gl_context = window.gl_create_context().unwrap();
    let _ = video.gl_set_swap_interval(1);
    sdl.mouse().show_cursor(false);

    let renderer = unsafe {
        OpenGl::new_from_function(|s| player::get_proc_address(&video, s).cast_const())
            .expect("femtovg renderer")
    };
    let mut canvas = Canvas::new(renderer).expect("femtovg canvas");
    let font = canvas.add_font_mem(FONT).expect("font");
    let mut overlay = overlay::Overlay::new(&video, &window, &gl_context, FONT);
    bluetooth::serve();

    let player = Player::new(&video, &mpv_dir.to_string_lossy()).expect("mpv");

    let (tx, rx) = channel();
    media::spawn_scan(MEDIA_PATH.into(), subtitles, tx);
    let mut app = App::new(player, rx, windows);

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
                    // exec skips destructors; drop explicitly so child
                    // processes (Bluetooth scan) are stopped.
                    drop(app);
                    let err = std::process::Command::new(exe).exec();
                    panic!("restart failed: {err}");
                }
                // Coming back from another app: the audio output may have changed.
                Event::Window { win_event: WindowEvent::FocusGained, .. } => app.refresh_home(),
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
        app.pump_pair();
        app.pump_windows();
        if bluetooth::CHANGED.swap(false, std::sync::atomic::Ordering::Relaxed) {
            app.refresh_home();
        }
        overlay.frame(&window, &gl_context);
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
                // Audio files render no video: gray "now playing" in the
                // corner, the file name centered in white.
                if let Some(name) = app.audio_name() {
                    canvas.clear_rect(0, 0, w, h, color(BACKGROUND_COLOR, 1.0));
                    let paint = Paint::color(color(DIM_COLOR, 1.0))
                        .with_font(&[font])
                        .with_font_size(FONT_SIZE)
                        .with_text_baseline(Baseline::Top);
                    let _ = canvas.fill_text(0.0, 0.0, "now playing", &paint);
                    let paint = paint.with_color(color(TEXT_COLOR, 1.0));
                    draw_centered(&mut canvas, name, (hf - FONT_SIZE) / 2.0, wf, &paint);
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
                    draw_progress(&mut canvas, font, frac, &time_text, TEXT_COLOR, alpha, wf, hf);
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
            canvas.clear_rect(0, 0, w, h, color(BACKGROUND_COLOR, 1.0));
        }
        let dim_paint = Paint::color(color(DIM_COLOR, 1.0))
            .with_font(&[font])
            .with_font_size(FONT_SIZE)
            .with_text_baseline(Baseline::Top);

        let loading_text = match &app.mode {
            Mode::Scanning => Some("Processing new media..."),
            Mode::External { text, .. } => Some(text.as_str()),
            _ => None,
        };
        if let Some(text) = loading_text {
            let paint = dim_paint.clone().with_color(color(TEXT_COLOR, 1.0));
            draw_centered(&mut canvas, text, (hf - FONT_SIZE) / 2.0 - FONT_SIZE * 0.7, wf, &paint);
            let subtext = &app.scan_status;
            if !subtext.is_empty() {
                let paint = dim_paint.clone().with_font_size(FONT_SIZE * 0.5);
                draw_centered(&mut canvas, subtext, (hf - FONT_SIZE) / 2.0 + FONT_SIZE * 0.7, wf, &paint);
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
            app.stack.last_mut().unwrap().menu_mut().draw(&mut canvas, font, dt, wf, hf);

            let _ = canvas.fill_text(0.0, 0.0, app.title(), &dim_paint);

            if let Some((name, since, _)) = &app.launching {
                let mut shade = Path::new();
                shade.rect(0.0, 0.0, wf, hf);
                canvas.fill_path(&shade, &Paint::color(color(BACKGROUND_COLOR, 0.6)));
                let pulse = 0.65 + 0.35 * (since.elapsed().as_secs_f32() * 4.0).sin();
                let paint = dim_paint.clone().with_color(color(TEXT_COLOR, pulse));
                draw_centered(&mut canvas, &format!("Starting {name}..."), (hf - FONT_SIZE) / 2.0, wf, &paint);
            }

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
                draw_progress(&mut canvas, font, pct as f32 / 100.0, &time_text, DIM_COLOR, 1.0, wf, hf);
            }

            // Top-right corner: external-command failures (accented), or the
            // file the background scan is currently probing.
            let corner = match &app.notice {
                Some((message, _)) => Some((message.clone(), ACCENT_COLOR)),
                None if !app.scan_status.is_empty() => {
                    Some((format!("processing {}", app.scan_status), DIM_COLOR))
                }
                None => None,
            };
            if let Some((text, rgb)) = corner {
                let paint = dim_paint
                    .clone()
                    .with_color(color(rgb, 1.0))
                    .with_font_size(FONT_SIZE * 0.4);
                let text_w = text_width(&canvas, &text, &paint);
                let _ = canvas.fill_text(wf - text_w - 10.0, 10.0, &text, &paint);
            }
        }

        canvas.flush();
        window.gl_swap_window();
    }
    // Its drop leaves no GL context current, which the canvas's drop needs.
    drop(overlay);
    let _ = window.gl_make_current(&gl_context);
}
