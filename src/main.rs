// Port of main.lua: menu construction, navigation state, playback and scan
// orchestration, and the SDL main loop (love.load/update/draw/keypressed...).

mod background;
mod config;
mod media;
mod menu;
mod player;

use background::Background;
use config::color;
use femtovg::{renderer::OpenGl, Baseline, Canvas, Paint, Path};
use media::{Kind, Node, ScanMsg};
use menu::{Menu, MenuItem};
use player::Player;
use sdl3::event::Event;
use sdl3::gamepad::Button;
use sdl3::keyboard::{Keycode, Mod};
use sdl3::mouse::MouseButton;
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum TrackKind {
    Audio,
    Sub,
}

impl TrackKind {
    fn as_str(self) -> &'static str {
        match self {
            TrackKind::Audio => "audio",
            TrackKind::Sub => "sub",
        }
    }
}

#[derive(Clone)]
enum Action {
    Open(String),
    Play,
    Tracks(TrackKind),
    SelectTrack(TrackKind, String),
}

enum Task {
    Scan { rx: Receiver<ScanMsg>, tree: Node },
    External(Receiver<()>),
}

enum Mode {
    Browse,
    Loading { text: String, subtext: String, task: Task },
    // A video is loaded (paused) and the playback menu is shown over its
    // still frame. Opening a video and pausing playback are the same state.
    Paused,
    // Fullscreen playback; osd_until keeps the seek overlay (progress bar +
    // time) up during and shortly after seeking.
    Playing { osd_until: Instant },
}

struct App {
    config: config::Config,
    tree: Node,
    path: Vec<String>,
    menus: Vec<Menu<Action>>,
    mode: Mode,
    player: Player,
}

impl App {
    fn current_menu(&mut self) -> &mut Menu<Action> {
        self.menus.last_mut().unwrap()
    }

    // Port of getDirectoryMenuItems' item construction + sort: in-progress
    // videos first, then unwatched, then watched (dimmed), alphabetical
    // within each group.
    fn directory_items(node: &Node) -> Vec<MenuItem<Action>> {
        let mut items: Vec<(i32, MenuItem<Action>)> = node
            .children
            .iter()
            .map(|(name, child)| {
                let display = if child.kind == Kind::Directory {
                    name.clone()
                } else {
                    media::strip_extension(name).to_string()
                };
                let pct = media::watch_pct(child);
                let category = if (1..=89).contains(&pct) { 1 } else if pct == 0 { 2 } else { 3 };
                let label = if child.kind == Kind::Video && pct > 0 && pct < 100 {
                    format!("* {display}")
                } else {
                    display
                };
                let mut item = MenuItem::new(label, Action::Open(name.clone()));
                item.dim = child.kind == Kind::Video && pct >= 100;
                (category, item)
            })
            .collect();
        items.sort_by(|(ac, a), (bc, b)| (ac, &a.label).cmp(&(bc, &b.label)));
        items.into_iter().map(|(_, item)| item).collect()
    }

    // Port of getVideoMenuItems: Play / Audio [current] / Subtitles [current].
    fn video_items(video: &Node) -> Vec<MenuItem<Action>> {
        let meta = &video.meta;
        let track_label = |kind: &str, id: &Option<String>| {
            id.as_ref().and_then(|id| meta.tracks.get(&format!("track_{kind}_{id}")))
        };
        let audio_label = match track_label("audio", &meta.aid) {
            Some(label) => format!("Audio [{label}]"),
            None => "Audio".to_string(),
        };
        let sub_label = if meta.sid.as_deref() == Some("") {
            "Subtitles [none]".to_string()
        } else {
            match track_label("sub", &meta.sid) {
                Some(label) => format!("Subtitles [{label}]"),
                None => "Subtitles".to_string(),
            }
        };
        vec![
            MenuItem::new("Play".into(), Action::Play),
            MenuItem::new(audio_label, Action::Tracks(TrackKind::Audio)),
            MenuItem::new(sub_label, Action::Tracks(TrackKind::Sub)),
        ]
    }

    // Port of getAudioSubMenuItems: available tracks sorted by id, with a
    // "none" entry for subtitles.
    fn track_items(video: &Node, kind: TrackKind) -> Vec<MenuItem<Action>> {
        let prefix = format!("track_{}_", kind.as_str());
        let mut tracks: Vec<(i64, MenuItem<Action>)> = video
            .meta
            .tracks
            .iter()
            .filter_map(|(key, label)| {
                let id = key.strip_prefix(&prefix)?;
                let item = MenuItem::new(label.clone(), Action::SelectTrack(kind, id.to_string()));
                Some((id.parse().unwrap_or(0), item))
            })
            .collect();
        if kind == TrackKind::Sub {
            tracks.push((-1, MenuItem::new("none".into(), Action::SelectTrack(kind, String::new()))));
        }
        tracks.sort_by_key(|(id, _)| *id);
        tracks.into_iter().map(|(_, item)| item).collect()
    }

    fn navigate_in(&mut self) {
        let menu = self.menus.last().unwrap();
        let Some(action) = menu.items.get(menu.selected).map(|i| i.action.clone()) else { return };
        match action {
            Action::Open(name) => {
                let mut child_path = self.path.clone();
                child_path.push(name);
                let Some(child) = media::get_node(&self.tree, &child_path) else { return };
                match child.kind {
                    Kind::Directory => {
                        let items = Self::directory_items(child);
                        self.path = child_path;
                        self.menus.push(Menu::new(items, 0));
                    }
                    Kind::Video => {
                        // Load right into the player, paused: the video menu
                        // is drawn over the still frame from here on.
                        let items = Self::video_items(child);
                        let meta = &child.meta;
                        let start = match (meta.position, meta.duration) {
                            (Some(pos), Some(dur)) if pos < dur - 3.0 => Some(pos),
                            _ => None,
                        };
                        let file = format!("{}/{}", self.config.media_path, child_path.join("/"));
                        self.player.play(&file, start, meta.aid.as_deref(), meta.sid.as_deref());
                        self.path = child_path;
                        self.menus.push(Menu::new(items, 0));
                        self.mode = Mode::Paused;
                    }
                    Kind::WiiGame | Kind::Script => {
                        // The Lua version launched Dolphin through the systemd
                        // user manager because spawning it directly from a
                        // LÖVE worker thread segfaulted its GL init; the
                        // detour is kept since it also survives this app dying
                        // mid-game.
                        let full = format!("{}/{}", self.config.media_path, child_path.join("/"));
                        let cmd = if child.kind == Kind::WiiGame {
                            format!(
                                r#"g="{full}"; if command -v systemd-run >/dev/null; then systemd-run --user --wait --collect --same-dir dolphin-emu --batch -C Dolphin.Display.Fullscreen=True -C Dolphin.Interface.ConfirmStop=False --exec="$g"; else dolphin-emu --batch -C Dolphin.Display.Fullscreen=True -C Dolphin.Interface.ConfirmStop=False --exec="$g"; fi"#
                            )
                        } else {
                            format!(r#"bash "{full}""#)
                        };
                        let (tx, rx) = channel();
                        std::thread::spawn(move || {
                            let _ = std::process::Command::new("sh").args(["-c", &cmd]).status();
                            let _ = tx.send(());
                        });
                        self.mode = Mode::Loading {
                            text: if child.kind == Kind::WiiGame { "Playing..." } else { "Running..." }.into(),
                            subtext: String::new(),
                            task: Task::External(rx),
                        };
                    }
                }
            }
            Action::Play => {
                // The file is already loaded (paused); at EOF (keep-open's
                // pause on the last frame) Play means watch again.
                if self.player.at_eof() {
                    self.player.restart();
                }
                self.player.set_pause(false);
                self.mode = Mode::Playing { osd_until: Instant::now() };
            }
            Action::Tracks(kind) => {
                let Some(node) = media::get_node(&self.tree, &self.path) else { return };
                let menu = Menu::new(Self::track_items(node, kind), 0);
                self.path.push(format!(":{}", kind.as_str()));
                self.menus.push(menu);
            }
            Action::SelectTrack(kind, id) => {
                let video_path = &self.path[..self.path.len() - 1];
                let Some(video) = media::get_node_mut(&mut self.tree, video_path) else { return };
                match kind {
                    TrackKind::Audio => video.meta.aid = Some(id.clone()),
                    TrackKind::Sub => video.meta.sid = Some(id.clone()),
                }
                media::save_metadata(&self.config.media_path, video_path, &video.meta);
                // Apply to the loaded file too, so it takes effect right away.
                let prop = if kind == TrackKind::Audio { "aid" } else { "sid" };
                self.player.set_track(prop, &id);
                let items = Self::video_items(video);
                let selected = if kind == TrackKind::Audio { 1 } else { 2 };
                self.path.pop();
                self.menus.pop();
                self.current_menu().set_items(items, selected);
            }
        }
    }

    fn navigate_out(&mut self) {
        // Backing out of the paused video menu closes the player (track
        // submenus, whose path segments start with ':', pop normally).
        if matches!(self.mode, Mode::Paused)
            && !self.path.last().is_some_and(|s| s.starts_with(':'))
        {
            self.close_player();
        } else if self.menus.len() > 1 {
            self.menus.pop();
            self.path.pop();
            self.current_menu().reset_scroll();
        }
    }

    fn save_position(&mut self) {
        let position = self.player.time_pos;
        if let Some(node) = media::get_node_mut(&mut self.tree, &self.path) {
            node.meta.position = Some(position);
            media::save_metadata(&self.config.media_path, &self.path, &node.meta);
        }
    }

    // Playing -> Paused: freeze on the current frame and bring the menu up
    // (the same state opening the video lands in).
    fn pause_player(&mut self) {
        self.player.set_pause(true);
        self.save_position();
        self.mode = Mode::Paused;
    }

    fn seek_player(&mut self, secs: f64) {
        self.player.seek(secs);
        if let Mode::Playing { osd_until } = &mut self.mode {
            *osd_until = Instant::now() + Duration::from_secs(2);
        }
    }

    // Unload the file and return to the directory menu, refreshed so the
    // watch state (star/dim/progress) is current, keeping the video selected.
    fn close_player(&mut self) {
        self.player.stop();
        while self.path.last().is_some_and(|s| s.starts_with(':')) {
            self.menus.pop();
            self.path.pop();
        }
        self.menus.pop();
        let name = self.path.pop();
        if let Some(dir) = media::get_node(&self.tree, &self.path) {
            let items = Self::directory_items(dir);
            let selected = name
                .and_then(|n| {
                    items.iter().position(|i| matches!(&i.action, Action::Open(m) if *m == n))
                })
                .unwrap_or(0);
            self.current_menu().set_items(items, selected);
        }
        self.mode = Mode::Browse;
    }
}

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

fn main() {
    let config = config::load();

    // Write the embedded mpv attachment files to the config dir (port of the
    // love.filesystem save-directory copy in love.load).
    let mpv_dir = config::config_dir().join("mpv");
    let _ = std::fs::create_dir_all(&mpv_dir);
    for (name, data) in [
        ("mpv.conf", &include_bytes!("../attachments/mpv/mpv.conf")[..]),
        ("visualiser.lua", &include_bytes!("../attachments/mpv/visualiser.lua")[..]),
        ("subfont.ttf", &include_bytes!("../attachments/mpv/subfont.ttf")[..]),
    ] {
        let _ = std::fs::write(mpv_dir.join(name), data);
    }

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
    let font = canvas
        .add_font_mem(include_bytes!("../attachments/mpv/subfont.ttf"))
        .expect("font");

    let player = Player::new(&video, &mpv_dir.to_string_lossy()).expect("mpv");

    let (tx, rx) = channel();
    media::spawn_scan(config.media_path.clone(), tx);
    let mut app = App {
        config,
        tree: Node::default(),
        path: vec![],
        menus: vec![Menu::new(vec![], 0)],
        mode: Mode::Loading {
            text: "Processing new media...".into(),
            subtext: String::new(),
            task: Task::Scan { rx, tree: Node::default() },
        },
        player,
    };

    let (w, h) = window.size();
    let mut background = Background::new(w as f32, h as f32);
    let mut scroll_buffer = 0.0_f32;
    let mut gamepads = vec![];
    let mut event_pump = sdl.event_pump().unwrap();
    let mut last_frame = Instant::now();

    'running: loop {
        let loading = matches!(app.mode, Mode::Loading { .. });
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
                    // Port of love.event.quit("restart").
                    use std::os::unix::process::CommandExt;
                    let exe = std::env::current_exe().expect("current_exe");
                    let err = std::process::Command::new(exe).exec();
                    panic!("restart failed: {err}");
                }
                _ if loading => {}
                // Both back and confirm leave playback for the paused menu.
                Event::KeyDown { keycode: Some(key), .. } if playing => match key {
                    Keycode::Escape | Keycode::AcBack | Keycode::Return => app.pause_player(),
                    Keycode::Left => app.seek_player(-5.0),
                    Keycode::Right => app.seek_player(5.0),
                    Keycode::Up => app.player.add_sub_delay(-0.5),
                    Keycode::Down => app.player.add_sub_delay(0.5),
                    _ => {}
                },
                Event::ControllerButtonDown { button, .. } if playing => match button {
                    Button::East | Button::South => app.pause_player(),
                    Button::DPadLeft => app.seek_player(-5.0),
                    Button::DPadRight => app.seek_player(5.0),
                    _ => {}
                },
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left | MouseButton::Right, ..
                } if playing => app.pause_player(),
                _ if playing => {}
                Event::KeyDown { keycode: Some(key), .. } => match key {
                    Keycode::Up => app.current_menu().navigate_up(),
                    Keycode::Down => app.current_menu().navigate_down(),
                    Keycode::Return => app.navigate_in(),
                    Keycode::Escape | Keycode::AcBack | Keycode::Sleep => app.navigate_out(),
                    key => {
                        let name = key.name().to_lowercase();
                        let mut chars = name.chars();
                        if let (Some(c), None) = (chars.next(), chars.next()) {
                            app.current_menu().jump_to_letter(c);
                        }
                    }
                },
                Event::MouseButtonDown { mouse_btn: MouseButton::Left, .. } => app.navigate_in(),
                Event::MouseButtonDown { mouse_btn: MouseButton::Right, .. } => app.navigate_out(),
                Event::MouseWheel { y, .. } => {
                    if (y > 0.0) != (scroll_buffer > 0.0) {
                        scroll_buffer = 0.0;
                    }
                    scroll_buffer += y;
                    while scroll_buffer > 30.0 {
                        app.current_menu().navigate_up();
                        scroll_buffer -= 30.0;
                    }
                    while scroll_buffer < -30.0 {
                        app.current_menu().navigate_down();
                        scroll_buffer += 30.0;
                    }
                }
                Event::ControllerButtonDown { button, .. } => match button {
                    Button::DPadUp => app.current_menu().navigate_up(),
                    Button::DPadDown => app.current_menu().navigate_down(),
                    Button::South => app.navigate_in(),
                    Button::East => app.navigate_out(),
                    _ => {}
                },
                _ => {}
            }
        }

        let dt = last_frame.elapsed().as_secs_f32();
        last_frame = Instant::now();
        let (w, h) = window.size();
        let (wf, hf) = (w as f32, h as f32);

        let playing_osd = match &app.mode {
            Mode::Playing { osd_until } => Some(*osd_until),
            _ => None,
        };
        if let Some(osd_until) = playing_osd {
            if app.player.poll_ended() {
                // The file went away under us (stop/error): save and close.
                app.save_position();
                app.close_player();
            } else if app.player.at_eof() {
                // keep-open paused on the last frame; land in the menu state.
                app.pause_player();
            } else {
                app.player.render(w as i32, h as i32);
                // Seek overlay: progress bar + current time, fading out over
                // the last moments before osd_until.
                let osd = osd_until.saturating_duration_since(Instant::now()).as_secs_f32();
                if osd > 0.0 {
                    let alpha = (osd / 0.3).min(1.0);
                    let style = &app.config.style;
                    canvas.set_size(w, h, 1.0);
                    let duration = app.player.duration();
                    let frac = if duration > 0.0 {
                        (app.player.time_pos / duration).clamp(0.0, 1.0) as f32
                    } else {
                        0.0
                    };
                    let bar_height = 4.0;
                    let mut bar = Path::new();
                    bar.rect(0.0, hf - bar_height, wf, bar_height);
                    canvas.fill_path(&bar, &Paint::color(color(style.text_color, 0.2 * alpha)));
                    if frac > 0.0 {
                        let mut fill = Path::new();
                        fill.rect(0.0, hf - bar_height, wf * frac, bar_height);
                        canvas.fill_path(&fill, &Paint::color(color(style.accent_color, alpha)));
                    }
                    let time_text =
                        format!("{} / {}", fmt_time(app.player.time_pos), fmt_time(duration));
                    let paint = Paint::color(color(style.text_color, alpha))
                        .with_font(&[font])
                        .with_font_size(style.font_size)
                        .with_text_baseline(Baseline::Top);
                    let text_w =
                        canvas.measure_text(0.0, 0.0, &time_text, &paint).map_or(0.0, |m| m.width());
                    let _ = canvas.fill_text(
                        (wf - text_w) / 2.0,
                        hf - bar_height - style.font_size * 1.2,
                        &time_text,
                        &paint,
                    );
                    canvas.flush();
                }
                window.gl_swap_window();
                continue;
            }
        } else if matches!(app.mode, Mode::Paused) && app.player.poll_ended() {
            // Only stop/error events can arrive while paused (a failed load).
            app.close_player();
        }

        // Drain background-task channels; return to Browse when done.
        if let Mode::Loading { subtext, task, .. } = &mut app.mode {
            let mut finished = false;
            match task {
                Task::Scan { rx, tree } => {
                    while let Ok(msg) = rx.try_recv() {
                        match msg {
                            ScanMsg::Status(s) => *subtext = s,
                            ScanMsg::Entry { parts, kind, meta } => media::insert(tree, parts, kind, meta),
                            ScanMsg::Done => finished = true,
                        }
                    }
                    if finished {
                        app.tree = std::mem::take(tree);
                        let dir = media::get_node(&app.tree, &app.path).unwrap_or(&app.tree);
                        app.menus = vec![Menu::new(App::directory_items(dir), 0)];
                    }
                }
                Task::External(rx) => finished = rx.try_recv().is_ok(),
            }
            if finished {
                app.mode = Mode::Browse;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
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
        let style = &app.config.style;
        let dim_paint = Paint::color(color(style.dim_color, 1.0))
            .with_font(&[font])
            .with_font_size(style.font_size)
            .with_text_baseline(Baseline::Top);

        if let Mode::Loading { text, subtext, .. } = &app.mode {
            // Port of components/loading_screen.lua.
            let size = style.font_size;
            let paint = dim_paint.clone().with_color(color(style.text_color, 1.0));
            let text_w = canvas.measure_text(0.0, 0.0, text, &paint).map_or(0.0, |m| m.width());
            let _ = canvas.fill_text((wf - text_w) / 2.0, (hf - size) / 2.0 - size * 0.7, text, &paint);
            if !subtext.is_empty() {
                let paint = dim_paint.clone().with_font_size(size * 0.5);
                let sub_w = canvas.measure_text(0.0, 0.0, subtext, &paint).map_or(0.0, |m| m.width());
                let _ = canvas.fill_text((wf - sub_w) / 2.0, (hf - size) / 2.0 + size * 0.7, subtext, &paint);
            }
        } else {
            // Port of the love.draw browse branch: starfield + menu + title +
            // watch progress bar + remaining-time line. While paused, the
            // starfield is replaced by the video frame dimmed by a
            // semitransparent black overlay.
            if paused {
                let mut overlay = Path::new();
                overlay.rect(0.0, 0.0, wf, hf);
                canvas.fill_path(&overlay, &Paint::color(color([0.0, 0.0, 0.0], 0.6)));
            } else {
                background.draw(&mut canvas, dt, wf, hf);
            }
            if let Some(menu) = app.menus.last_mut() {
                menu.draw(&mut canvas, font, &app.config, dt, wf, hf);
            }

            let title = app.path.last().map(|s| s.as_str()).unwrap_or("tiny media center");
            let title = title.strip_prefix(':').unwrap_or(title);
            let node = media::get_node(&app.tree, &app.path);
            let display_title = if node.is_some_and(|n| n.kind != Kind::Directory) {
                media::strip_extension(title)
            } else {
                title
            };
            let _ = canvas.fill_text(0.0, 0.0, display_title, &dim_paint);

            if let Some(video) = media::video_context(&app.tree, &app.path) {
                let pct = media::watch_pct(video);
                let bar_height = 4.0;

                let mut bar = Path::new();
                bar.rect(0.0, hf - bar_height, wf, bar_height);
                canvas.fill_path(&bar, &Paint::color(color(style.text_color, 0.2)));
                if pct > 0 {
                    let mut fill = Path::new();
                    fill.rect(0.0, hf - bar_height, wf * pct as f32 / 100.0, bar_height);
                    canvas.fill_path(&fill, &Paint::color(color(style.accent_color, 1.0)));
                }

                if let Some(duration) = video.meta.duration {
                    let remaining = duration - video.meta.position.unwrap_or(0.0);
                    let now = chrono::Local::now();
                    let end_time = now + chrono::Duration::seconds(remaining as i64);
                    let mins = (remaining / 60.0).floor() as i64;
                    let (hours, mins) = (mins / 60, mins % 60);
                    let rem_str =
                        if hours > 0 { format!("{hours}h{mins:02}min") } else { format!("{mins}min") };
                    let time_text = format!(
                        "{}  --  {}  ->  {}",
                        now.format("%H:%M"),
                        rem_str,
                        end_time.format("%H:%M")
                    );
                    let text_w =
                        canvas.measure_text(0.0, 0.0, &time_text, &dim_paint).map_or(0.0, |m| m.width());
                    let _ = canvas.fill_text(
                        (wf - text_w) / 2.0,
                        hf - bar_height - style.font_size * 1.2,
                        &time_text,
                        &dim_paint,
                    );
                }
            }
        }

        canvas.flush();
        window.gl_swap_window();
    }
}
