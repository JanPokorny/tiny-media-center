// The volume pop-up and desktop notifications, shown in a small window that
// stays above every app. It is an SDL tooltip popup: an override-redirect X
// window (so the window manager leaves it alone and it never takes focus)
// without input; windows.rs raises it after every restack.

use crate::config::{color, Style};
use crate::menu::text_width;
use femtovg::{renderer::OpenGl, Baseline, Canvas, FontId, Paint, Path, Renderer};
use sdl3::video::{GLContext, Window, WindowPos};
use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};
use zbus::connection::{Builder, Connection};

const MAX_NOTES: usize = 3;
const VOLUME_TIME: Duration = Duration::from_millis(1500);
const PATH: &str = "/org/freedesktop/Notifications";
const NAME: &str = "org.freedesktop.Notifications";
// NotificationClosed reasons.
const EXPIRED: u32 = 1;
const CLOSED: u32 = 3;
const UNDEFINED: u32 = 4;

struct Note {
    id: u32,
    app: String,
    summary: String,
    body: String,
    until: Instant,
}

struct State {
    next_id: u32,
    // Oldest first.
    notes: Vec<Note>,
    // Level (1.0 = 100%), muted, and when to hide it.
    volume: Option<(f32, bool, Instant)>,
    // (id, reason) to announce on the bus.
    closed: Vec<(u32, u32)>,
}

// Written by the bus thread and the key handler, read by the main loop.
static STATE: Mutex<State> = Mutex::new(State { next_id: 1, notes: Vec::new(), volume: None, closed: Vec::new() });
static CONN: OnceLock<Connection> = OnceLock::new();

fn state() -> MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

// -1 asks for the default; 0 (never) and long ones are cut short, as nobody
// can dismiss a notification from the couch.
fn timeout(ms: i32) -> Duration {
    Duration::from_millis(if ms < 0 { 5000 } else if ms == 0 { 10_000 } else { ms.min(10_000) } as u64)
}

// Removes the tags of the markup subset notifications may use, and its entities.
fn strip_markup(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find('<') {
        out += &rest[..start];
        let tag = rest[start..].find('>').map(|end| &rest[start + 1..start + end]);
        let name = tag.and_then(|t| t.trim_start_matches('/').split([' ', '/']).next());
        match (tag, name) {
            (Some(tag), Some("b" | "i" | "u" | "a" | "img" | "br")) => rest = &rest[start + tag.len() + 2..],
            _ => {
                out.push('<');
                rest = &rest[start + 1..];
            }
        }
    }
    out += rest;
    out.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// `wpctl get-volume` prints "Volume: 0.45" or "Volume: 0.45 [MUTED]".
fn parse_volume(s: &str) -> Option<(f32, bool)> {
    let level = s.strip_prefix("Volume:")?.split_whitespace().next()?.parse().ok()?;
    Some((level, s.contains("[MUTED]")))
}

impl State {
    // The id the notification gets: replaces_id when it is still showing.
    fn notify(&mut self, replaces: u32, app: String, summary: String, body: String, ms: i32, now: Instant) -> u32 {
        let until = now + timeout(ms);
        if let Some(note) = self.notes.iter_mut().find(|n| n.id == replaces) {
            (note.app, note.summary, note.body, note.until) = (app, summary, body, until);
            return replaces;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.notes.push(Note { id, app, summary, body, until });
        while self.notes.len() > MAX_NOTES {
            self.closed.push((self.notes.remove(0).id, UNDEFINED));
        }
        id
    }

    fn close(&mut self, id: u32) {
        if self.notes.iter().any(|n| n.id == id) {
            self.notes.retain(|n| n.id != id);
            self.closed.push((id, CLOSED));
        }
    }

    fn expire(&mut self, now: Instant) {
        let expired = self.notes.iter().filter(|n| n.until <= now).map(|n| (n.id, EXPIRED));
        self.closed.extend(expired);
        self.notes.retain(|n| n.until > now);
        if self.volume.is_some_and(|v| v.2 <= now) {
            self.volume = None;
        }
    }

    // Top to bottom: the volume, then the notifications, newest first.
    fn cards(&self) -> Vec<Card> {
        let volume = self.volume.map(|(level, muted, _)| Card::Volume { level, muted });
        let notes = self.notes.iter().rev().map(|n| Card::Note {
            app: n.app.clone(),
            summary: n.summary.clone(),
            body: n.body.clone(),
        });
        volume.into_iter().chain(notes).collect()
    }
}

#[derive(Debug, PartialEq)]
enum Card {
    Volume { level: f32, muted: bool },
    Note { app: String, summary: String, body: String },
}

// Heights in units of u (the corner text size).
const PAD: f32 = 0.5;
const SUMMARY: f32 = 1.3;
const APP: f32 = 0.9;
const BODY: f32 = 1.1;
const BAR: f32 = 0.5;

impl Card {
    fn height(&self) -> f32 {
        let content = match self {
            Card::Volume { .. } => SUMMARY + BAR + 0.2,
            Card::Note { app, body, .. } => {
                SUMMARY + if app.is_empty() { 0.0 } else { APP } + if body.is_empty() { 0.0 } else { BODY }
            }
        };
        content + 2.0 * PAD
    }
}

pub fn show_volume((level, muted): (f32, bool)) {
    state().volume = Some((level, muted, Instant::now() + VOLUME_TIME));
}

// Runs a wpctl command, then pops up the resulting volume.
pub fn volume_command(command: &str) {
    let _ = std::process::Command::new("sh").arg("-c").arg(command).status();
    let get = std::process::Command::new("wpctl").args(["get-volume", "@DEFAULT_AUDIO_SINK@"]).output();
    if let Some(volume) = get.ok().and_then(|o| parse_volume(&String::from_utf8_lossy(&o.stdout))) {
        show_volume(volume);
    }
}

struct Server;

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl Server {
    #[allow(clippy::too_many_arguments)]
    fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        _actions: Vec<String>,
        _hints: HashMap<String, zbus::OwnedVariant>,
        expire_timeout: i32,
    ) -> u32 {
        let (summary, body) = (strip_markup(&summary), strip_markup(&body));
        state().notify(replaces_id, app_name, summary, body, expire_timeout, Instant::now())
    }

    fn close_notification(&self, id: u32) {
        state().close(id);
    }

    fn get_capabilities(&self) -> Vec<&str> {
        vec!["body"]
    }

    fn get_server_information(&self) -> (&str, &str, &str, &str) {
        ("tiny-media-center", "", env!("CARGO_PKG_VERSION"), "1.2")
    }
}

// zbus drives the connection from the thread that calls block_on.
fn serve() {
    std::thread::spawn(|| {
        zbus::block_on(async {
            let conn = Builder::session().name(NAME).serve_at(PATH, Server).build().await;
            match conn {
                Ok(conn) => {
                    let _ = CONN.set(conn);
                    std::future::pending::<()>().await
                }
                // Another notification daemon owns the name.
                Err(e) => eprintln!("notification server: {e}"),
            }
        })
    });
}

pub struct Overlay {
    window: Window,
    gl: GLContext,
    canvas: Canvas<OpenGl>,
    font: FontId,
    shown: bool,
    size: (u32, u32),
}

impl Overlay {
    // Leaves `main`'s GL context current. The popup is a child of `main` for SDL only.
    pub fn new(video: &sdl3::VideoSubsystem, main: &Window, main_gl: &GLContext, font: &[u8]) -> Overlay {
        serve();
        let mut builder = unsafe { video.popup_window(main, 100, 100) };
        let window = builder.tooltip().always_on_top().opengl().hidden().build().expect("overlay window");
        let gl = window.gl_create_context().expect("overlay GL context");
        // Vsync is per window: the overlay mustn't make the frame loop wait twice.
        let _ = video.gl_set_swap_interval(0);
        let renderer = unsafe {
            OpenGl::new_from_function(|s| crate::player::get_proc_address(video, s).cast_const()).expect("renderer")
        };
        let mut canvas = Canvas::new(renderer).expect("canvas");
        let font = canvas.add_font_mem(font).expect("font");
        let _ = main.gl_make_current(main_gl);
        let props = unsafe { sdl3::sys::video::SDL_GetWindowProperties(window.raw()) };
        let id = unsafe {
            sdl3::sys::properties::SDL_GetNumberProperty(props, sdl3::sys::video::SDL_PROP_WINDOW_X11_WINDOW_NUMBER, 0)
        };
        crate::windows::set_overlay(id as u32);
        Overlay { window, gl, canvas, font, shown: false, size: (0, 0) }
    }

    // Draws and shows what is due, or hides the window. Call once per frame.
    pub fn frame(&mut self, main: &Window, main_gl: &GLContext, style: &Style) {
        let (cards, closed) = {
            let mut state = state();
            state.expire(Instant::now());
            (state.cards(), std::mem::take(&mut state.closed))
        };
        if let Some(conn) = CONN.get() {
            for signal in closed {
                let _ = zbus::block_on(conn.emit_signal(None::<&str>, PATH, NAME, "NotificationClosed", &signal));
            }
        }
        if cards.is_empty() {
            if self.shown {
                self.window.hide();
                self.shown = false;
            }
            return;
        }
        let u = style.font_size * 0.4;
        let (w, h) = ((26.0 * u) as u32, (cards.iter().map(Card::height).sum::<f32>() * u).ceil() as u32);
        if self.size != (w, h) {
            let (screen_w, _) = main.size();
            let _ = self.window.set_size(w, h);
            self.window.set_position(WindowPos::Positioned(screen_w as i32 - w as i32 - u as i32), WindowPos::Positioned(u as i32));
            self.size = (w, h);
        }
        if !self.shown {
            self.window.show();
            crate::windows::raise_overlay();
            self.shown = true;
        }
        let _ = self.window.gl_make_current(&self.gl);
        self.canvas.set_size(w, h, 1.0);
        draw(&mut self.canvas, self.font, style, &cards, u, w as f32, h as f32);
        self.canvas.flush();
        self.window.gl_swap_window();
        let _ = main.gl_make_current(main_gl);
    }
}

// Cuts text to fit in max_w pixels.
fn fit<R: Renderer>(canvas: &Canvas<R>, text: &str, paint: &Paint, max_w: f32) -> String {
    let full = text.chars().count();
    let cut = |n: usize| -> String {
        let ellipsis = if n < full { Some('…') } else { None };
        text.chars().take(n).chain(ellipsis).collect()
    };
    (0..=full.min(120)).rev().map(cut).find(|s| text_width(canvas, s, paint) <= max_w).unwrap_or_default()
}

fn draw<R: Renderer>(canvas: &mut Canvas<R>, font: FontId, style: &Style, cards: &[Card], u: f32, w: f32, h: f32) {
    let rect = |canvas: &mut Canvas<R>, (x, y, w, h): (f32, f32, f32, f32), rgb: [f32; 3], alpha: f32| {
        let mut path = Path::new();
        path.rect(x, y, w, h);
        canvas.fill_path(&path, &Paint::color(color(rgb, alpha)));
    };
    let text = |size: f32, rgb: [f32; 3]| {
        Paint::color(color(rgb, 1.0)).with_font(&[font]).with_font_size(size * u).with_text_baseline(Baseline::Top)
    };
    rect(canvas, (0.0, 0.0, w, h), style.dim_color, 1.0);
    rect(canvas, (2.0, 2.0, w - 4.0, h - 4.0), style.background_color, 1.0);
    let (x, max_w) = (PAD * 1.6 * u, w - PAD * 3.2 * u);
    let mut y = 0.0;
    for (i, card) in cards.iter().enumerate() {
        if i > 0 {
            rect(canvas, (x, y, max_w, 1.0), style.dim_color, 0.5);
        }
        let mut row = y + PAD * u;
        match card {
            Card::Volume { level, muted } => {
                let (label, rgb) =
                    if *muted { ("Muted".into(), style.dim_color) } else { (format!("Volume: {}%", (level * 100.0).round()), style.text_color) };
                let _ = canvas.fill_text(x, row, &label, &text(SUMMARY * 0.8, rgb));
                row += (SUMMARY + 0.2) * u;
                rect(canvas, (x, row, max_w, BAR * u), style.text_color, 0.2);
                let fill = if *muted { style.dim_color } else { style.accent_color };
                rect(canvas, (x, row, max_w * level.clamp(0.0, 1.0), BAR * u), fill, 1.0);
            }
            Card::Note { app, summary, body } => {
                for (line, size, rgb) in [(app, APP, style.dim_color), (summary, SUMMARY, style.text_color), (body, BODY, style.dim_color)] {
                    if !line.is_empty() {
                        let paint = text(size * 0.8, rgb);
                        let _ = canvas.fill_text(x, row, fit(canvas, line, &paint, max_w), &paint);
                        row += size * u;
                    }
                }
            }
        }
        y += card.height() * u;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(state: &mut State, replaces: u32, summary: &str, ms: i32, now: Instant) -> u32 {
        state.notify(replaces, "app".into(), summary.into(), String::new(), ms, now)
    }

    const fn empty() -> State {
        State { next_id: 1, notes: Vec::new(), volume: None, closed: Vec::new() }
    }

    #[test]
    fn parses_wpctl_output() {
        assert_eq!(parse_volume("Volume: 0.45\n"), Some((0.45, false)));
        assert_eq!(parse_volume("Volume: 1.00 [MUTED]\n"), Some((1.0, true)));
        assert_eq!(parse_volume("Could not connect"), None);
        assert_eq!(parse_volume("Volume: x"), None);
    }

    #[test]
    fn timeout_policy() {
        assert_eq!(timeout(-1), Duration::from_secs(5));
        assert_eq!(timeout(0), Duration::from_secs(10));
        assert_eq!(timeout(2000), Duration::from_secs(2));
        assert_eq!(timeout(60_000), Duration::from_secs(10));
    }

    #[test]
    fn strips_markup() {
        assert_eq!(strip_markup("<b>Hi</b> <a href=\"x\">there</a><br/>!"), "Hi there!");
        assert_eq!(strip_markup("a &lt; b &amp;&amp; c &gt; d"), "a < b && c > d");
        assert_eq!(strip_markup("1 < 2 and <unknown>"), "1 < 2 and <unknown>");
        assert_eq!(strip_markup("5 <"), "5 <");
    }

    #[test]
    fn stacks_replaces_and_expires() {
        let (mut s, now) = (empty(), Instant::now());
        let a = note(&mut s, 0, "a", 1000, now);
        let b = note(&mut s, 0, "b", -1, now);
        assert_eq!((a, b), (1, 2));
        assert_eq!(note(&mut s, a, "a2", 1000, now), a); // replaced in place
        assert_eq!(note(&mut s, 99, "c", -1, now), 3); // unknown id: new
        let summaries = |s: &State| s.cards().into_iter().map(|c| format!("{c:?}")).collect::<Vec<_>>();
        assert!(summaries(&s)[0].contains("\"c\"") && summaries(&s)[2].contains("\"a2\""));
        note(&mut s, 0, "d", -1, now); // a fourth pushes the oldest out
        assert_eq!(s.notes.iter().map(|n| n.id).collect::<Vec<_>>(), [2, 3, 4]);
        assert_eq!(s.closed, [(1, UNDEFINED)]);
        s.closed.clear();
        s.close(3);
        s.close(3); // already gone: no second signal
        assert_eq!(s.closed, [(3, CLOSED)]);
        s.closed.clear();
        s.volume = Some((0.5, false, now + Duration::from_secs(1)));
        s.expire(now + Duration::from_secs(6));
        assert!(s.notes.is_empty() && s.volume.is_none());
        assert_eq!(s.closed, [(2, EXPIRED), (4, EXPIRED)]);
    }

    #[test]
    fn volume_card_goes_first() {
        let (mut s, now) = (empty(), Instant::now());
        note(&mut s, 0, "a", -1, now);
        s.volume = Some((0.3, true, now));
        assert!(matches!(s.cards()[0], Card::Volume { muted: true, .. }));
        assert_eq!(s.cards().len(), 2);
    }
}
