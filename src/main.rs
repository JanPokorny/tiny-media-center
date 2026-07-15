// Port of main.lua: menu construction, navigation state, playback and scan
// orchestration, and the SDL main loop (love.load/update/draw/keypressed...).

mod background;
mod config;
mod loading;
mod media;
mod menu;
mod player;

use background::Background;
use config::color;
use femtovg::{renderer::OpenGl, Baseline, Canvas, FontId, Paint, Path};
use media::{Kind, Meta, Node, ScanMsg};
use menu::{Menu, MenuItem};
use player::Player;
use sdl3::event::Event;
use sdl3::gamepad::Button;
use sdl3::keyboard::{Keycode, Mod};
use sdl3::mouse::MouseButton;
use std::sync::mpsc::{channel, Receiver};
use std::time::Instant;

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
    Scan { rx: Receiver<ScanMsg>, entries: Vec<(Vec<String>, Kind, Meta)> },
    External(Receiver<()>),
}

enum Mode {
    Browse,
    Loading { text: String, subtext: String, task: Task },
    Playing,
}

struct App {
    config: config::Config,
    tree: Node,
    path: Vec<String>,
    menus: Vec<Menu<Action>>,
    mode: Mode,
    player: Player,
    background: Background,
    screen: (f32, f32),
    scroll_buffer: f32,
}

impl App {
    fn line_h(&self) -> f32 {
        self.config.style.font_size * 1.125
    }

    fn current_menu(&mut self) -> &mut Menu<Action> {
        self.menus.last_mut().unwrap()
    }

    // Port of getDirectoryMenuItems' item construction + sort: in-progress
    // videos first, then unwatched, then watched (dimmed), alphabetical
    // within each group.
    fn directory_items(node: &Node) -> Vec<MenuItem<Action>> {
        let mut raw: Vec<(i32, String, bool, String)> = node
            .children
            .values()
            .map(|child| {
                let display = if child.kind == Kind::Directory {
                    child.name.clone()
                } else {
                    media::strip_extension(&child.name).to_string()
                };
                let pct = media::watch_pct(child);
                let category = if (1..=89).contains(&pct) { 1 } else if pct == 0 { 2 } else { 3 };
                let label = if child.kind == Kind::Video && pct > 0 && pct < 100 {
                    format!("* {display}")
                } else {
                    display
                };
                let dim = child.kind == Kind::Video && pct >= 100;
                (category, label, dim, child.name.clone())
            })
            .collect();
        raw.sort_by(|a, b| (a.0, &a.1).cmp(&(b.0, &b.1)));
        raw.into_iter()
            .map(|(_, label, dim, name)| {
                let mut item = MenuItem::new(label, Action::Open(name));
                item.dim = dim;
                item
            })
            .collect()
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
        let mut raw: Vec<(i64, String, String)> = vec![];
        if kind == TrackKind::Sub {
            raw.push((-1, "none".into(), String::new()));
        }
        let prefix = format!("track_{}_", kind.as_str());
        for (key, label) in &video.meta.tracks {
            if let Some(id) = key.strip_prefix(&prefix) {
                raw.push((id.parse().unwrap_or(0), label.clone(), id.to_string()));
            }
        }
        raw.sort_by_key(|(id, ..)| *id);
        raw.into_iter()
            .map(|(_, label, id)| MenuItem::new(label, Action::SelectTrack(kind, id)))
            .collect()
    }

    fn run_external(&mut self, node_path: &[String], kind: Kind) {
        let cmd = media::external_command(&self.config.media_path, node_path, kind);
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = std::process::Command::new("sh").args(["-c", &cmd]).status();
            let _ = tx.send(());
        });
        self.mode = Mode::Loading {
            text: if kind == Kind::WiiGame { "Playing...".into() } else { "Running...".into() },
            subtext: String::new(),
            task: Task::External(rx),
        };
    }

    fn play(&mut self) {
        let Some(node) = media::get_node(&self.tree, &self.path) else { return };
        let meta = &node.meta;
        let start = match (meta.position, meta.duration) {
            (Some(pos), Some(dur)) if pos < dur - 3.0 => Some(pos),
            _ => None,
        };
        let file = media::media_file(&self.config.media_path, &node.path);
        self.player.play(&file, start, meta.aid.as_deref(), meta.sid.as_deref());
        self.mode = Mode::Playing;
    }

    fn navigate_in(&mut self) {
        let Some(action) = self.current_menu().selected_action().cloned() else { return };
        match action {
            Action::Open(name) => {
                let mut child_path = self.path.clone();
                child_path.push(name);
                let Some(child) = media::get_node(&self.tree, &child_path) else { return };
                match child.kind {
                    Kind::Directory => {
                        let menu = Menu::new(Self::directory_items(child), 0);
                        self.path = child_path;
                        self.menus.push(menu);
                    }
                    Kind::Video => {
                        let menu = Menu::new(Self::video_items(child), 0);
                        self.path = child_path;
                        self.menus.push(menu);
                    }
                    Kind::WiiGame | Kind::Script => self.run_external(&child_path, child.kind),
                }
            }
            Action::Play => self.play(),
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
                    TrackKind::Audio => video.meta.aid = Some(id),
                    TrackKind::Sub => video.meta.sid = Some(id),
                }
                media::save_metadata(&self.config.media_path, video);
                let items = Self::video_items(video);
                let selected = if kind == TrackKind::Audio { 1 } else { 2 };
                self.path.pop();
                self.menus.pop();
                self.current_menu().set_items(items, selected);
            }
        }
    }

    fn navigate_out(&mut self) {
        if self.menus.len() > 1 {
            self.menus.pop();
            self.path.pop();
            self.current_menu().reset_scroll();
        }
    }

    fn playback_ended(&mut self) {
        let position = self.player.time_pos;
        let media_path = self.config.media_path.clone();
        if let Some(node) = media::get_node_mut(&mut self.tree, &self.path) {
            node.meta.position = Some(position);
            media::save_metadata(&media_path, node);
            let items = Self::video_items(node);
            self.current_menu().set_items(items, 0);
        }
        self.mode = Mode::Browse;
    }

    fn start_scan(&mut self) {
        let (tx, rx) = channel();
        media::spawn_scan(self.config.media_path.clone(), tx);
        self.mode = Mode::Loading {
            text: "Processing new media...".into(),
            subtext: String::new(),
            task: Task::Scan { rx, entries: vec![] },
        };
    }

    // Drain background-task channels; returns to Browse when done.
    fn update_loading(&mut self) {
        let Mode::Loading { subtext, task, .. } = &mut self.mode else { return };
        let mut finished = false;
        match task {
            Task::Scan { rx, entries } => {
                while let Ok(msg) = rx.try_recv() {
                    match msg {
                        ScanMsg::Status(s) => *subtext = s,
                        ScanMsg::Entry { parts, kind, meta } => entries.push((parts, kind, meta)),
                        ScanMsg::Done => finished = true,
                    }
                }
                if finished {
                    self.tree = Node::root();
                    for (parts, kind, meta) in std::mem::take(entries) {
                        media::insert(&mut self.tree, parts, kind, meta);
                    }
                    let dir = media::get_node(&self.tree, &self.path).unwrap_or(&self.tree);
                    self.menus = vec![Menu::new(Self::directory_items(dir), 0)];
                }
            }
            Task::External(rx) => finished = rx.try_recv().is_ok(),
        }
        if finished {
            self.mode = Mode::Browse;
        }
    }
}

// Port of the love.draw browse branch: menu + title + watch progress bar +
// remaining-time line.
fn draw_browse(app: &mut App, canvas: &mut Canvas<OpenGl>, font: FontId, dt: f32) {
    let (w, h) = app.screen;

    app.background.draw(canvas, h);
    if let Some(menu) = app.menus.last_mut() {
        menu.draw(canvas, font, &app.config, dt, w, h);
    }
    let style = &app.config.style;
    let dim_paint = Paint::color(color(style.dim_color, 1.0))
        .with_font(&[font])
        .with_font_size(style.font_size)
        .with_text_baseline(Baseline::Top);

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
        bar.rect(0.0, h - bar_height, w, bar_height);
        canvas.fill_path(&bar, &Paint::color(color(style.text_color, 0.2)));
        if pct > 0 {
            let mut fill = Path::new();
            fill.rect(0.0, h - bar_height, w * pct as f32 / 100.0, bar_height);
            canvas.fill_path(&fill, &Paint::color(color(style.accent_color, 1.0)));
        }

        if let Some(duration) = video.meta.duration {
            let remaining = duration - video.meta.position.unwrap_or(0.0);
            let now = chrono::Local::now();
            let end_time = now + chrono::Duration::seconds(remaining as i64);
            let mins = (remaining / 60.0).floor() as i64;
            let (hours, mins) = (mins / 60, mins % 60);
            let rem_str = if hours > 0 { format!("{hours}h{mins:02}min") } else { format!("{mins}min") };
            let time_text = format!(
                "{}  --  {}  ->  {}",
                now.format("%H:%M"),
                rem_str,
                end_time.format("%H:%M")
            );
            let text_w = canvas.measure_text(0.0, 0.0, &time_text, &dim_paint).map_or(0.0, |m| m.width());
            let _ = canvas.fill_text(
                (w - text_w) / 2.0,
                h - bar_height - style.font_size * 1.2,
                &time_text,
                &dim_paint,
            );
        }
    }
}

// Write the embedded mpv attachment files to the config dir (port of the
// love.filesystem save-directory copy in love.load).
fn write_mpv_attachments() -> String {
    let dir = config::config_dir().join("mpv");
    let _ = std::fs::create_dir_all(&dir);
    let files: [(&str, &[u8]); 3] = [
        ("mpv.conf", include_bytes!("../attachments/mpv/mpv.conf")),
        ("visualiser.lua", include_bytes!("../attachments/mpv/visualiser.lua")),
        ("subfont.ttf", include_bytes!("../attachments/mpv/subfont.ttf")),
    ];
    for (name, data) in files {
        let _ = std::fs::write(dir.join(name), data);
    }
    dir.to_string_lossy().into_owned()
}

fn restart() -> ! {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe().expect("current_exe");
    let err = std::process::Command::new(exe).exec();
    panic!("restart failed: {err}");
}

fn main() {
    let config = config::load();
    let mpv_config_dir = write_mpv_attachments();

    let sdl = sdl3::init().unwrap();
    let video = sdl.video().unwrap();
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

    let player = Player::new(&video, &mpv_config_dir).expect("mpv");

    let (w, h) = window.size();
    let mut app = App {
        config,
        tree: Node::root(),
        path: vec![],
        menus: vec![Menu::new(vec![], 0)],
        mode: Mode::Browse,
        player,
        background: Background::new(w as f32, h as f32),
        screen: (w as f32, h as f32),
        scroll_buffer: 0.0,
    };
    app.start_scan();

    let mut gamepads = vec![];
    let mut event_pump = sdl.event_pump().unwrap();
    let mut last_frame = Instant::now();

    'running: loop {
        let loading = matches!(app.mode, Mode::Loading { .. });
        let playing = matches!(app.mode, Mode::Playing);

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
                    restart()
                }
                _ if loading => {}
                Event::KeyDown { keycode: Some(key), .. } if playing => match key {
                    Keycode::Escape | Keycode::AcBack => app.player.stop(),
                    Keycode::Left => app.player.seek(-5.0),
                    Keycode::Right => app.player.seek(5.0),
                    Keycode::Return => app.player.toggle_pause(),
                    Keycode::Up => app.player.add_sub_delay(-0.5),
                    Keycode::Down => app.player.add_sub_delay(0.5),
                    _ => {}
                },
                Event::ControllerButtonDown { button, .. } if playing => match button {
                    Button::East => app.player.stop(),
                    Button::South => app.player.toggle_pause(),
                    Button::DPadLeft => app.player.seek(-5.0),
                    Button::DPadRight => app.player.seek(5.0),
                    _ => {}
                },
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
                    if (y > 0.0) != (app.scroll_buffer > 0.0) {
                        app.scroll_buffer = 0.0;
                    }
                    app.scroll_buffer += y;
                    while app.scroll_buffer > 30.0 {
                        app.current_menu().navigate_up();
                        app.scroll_buffer -= 30.0;
                    }
                    while app.scroll_buffer < -30.0 {
                        app.current_menu().navigate_down();
                        app.scroll_buffer += 30.0;
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
        app.screen = (w as f32, h as f32);

        match &app.mode {
            Mode::Playing => {
                if app.player.poll_ended() {
                    app.playback_ended();
                } else {
                    app.player.render(w as i32, h as i32);
                    window.gl_swap_window();
                    continue;
                }
            }
            Mode::Loading { .. } => {
                app.update_loading();
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            Mode::Browse => {}
        }

        canvas.set_size(w, h, 1.0);
        canvas.clear_rect(0, 0, w, h, color(app.config.style.background_color, 1.0));

        if let Mode::Loading { text, subtext, .. } = &app.mode {
            loading::draw(&mut canvas, font, &app.config, text, subtext, w as f32, h as f32);
        } else {
            app.background.update(dt, w as f32, h as f32);
            let (line_h, screen_h) = (app.line_h(), app.screen.1);
            if let Some(menu) = app.menus.last_mut() {
                menu.update(dt, line_h, screen_h);
            }
            draw_browse(&mut app, &mut canvas, font, dt);
        }

        canvas.flush();
        window.gl_swap_window();
    }
}
