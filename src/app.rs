// Navigation state and playback/scan orchestration. The UI location is a
// single stack of frames (directory menus, the open video's menu, track
// submenus), so there is no parallel path/menu bookkeeping to keep in sync.

use crate::audio::{self, BtDevice, Discovery};
use crate::config::MEDIA_PATH;
use crate::input::Input;
use crate::media::{self, Kind, Node, ScanMsg, TrackKind};
use crate::menu::{Menu, MenuItem};
use crate::player::Player;
use crate::windows::{self, Msg, Win, Window};
use std::process::Command;
use std::sync::mpsc::{channel, Receiver, Sender, TryRecvError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, PartialEq, Default)]
pub enum Action {
    // Only on headings, which can't be selected.
    #[default]
    Heading,
    Open(String),
    Play,
    // Hide the open video from continue watching and close it.
    Dismiss,
    Tracks(TrackKind),
    SelectTrack(TrackKind, String),
    // Open the in-progress video at this path, as Open -> ... -> video would.
    Resume(Vec<String>),
    Audio,
    SetSink(String),
    // The device, and whether it needs pairing first.
    ConnectBt(BtDevice, bool),
    PairNew,
    // One of other apps' windows (listed on Home), and what to do to it.
    Window(Window),
    Focus(Window),
    Close(Window),
    Shutdown,
}

pub enum Frame {
    // The app's top-level menu, always the bottom of the stack.
    Home { menu: Menu<Action> },
    Dir { name: String, menu: Menu<Action> },
    Video { name: String, menu: Menu<Action> },
    Tracks { kind: TrackKind, menu: Menu<Action> },
    Audio { menu: Menu<Action> },
    // Bluetooth discovery runs while this is on the stack.
    Pair { menu: Menu<Action>, scan: Discovery },
    // Titled after the window, which can change while this is open.
    Window { id: Window, menu: Menu<Action> },
}

impl Frame {
    pub fn menu_mut(&mut self) -> &mut Menu<Action> {
        match self {
            Frame::Home { menu }
            | Frame::Dir { menu, .. }
            | Frame::Video { menu, .. }
            | Frame::Tracks { menu, .. }
            | Frame::Audio { menu }
            | Frame::Pair { menu, .. }
            | Frame::Window { menu, .. } => menu,
        }
    }
}

pub enum Mode {
    // Initial scan: loading screen until the directory walk finishes
    // (probing continues in the background afterwards).
    Scanning,
    Browse,
    // A slow task (Bluetooth connect) is running; rx delivers an error
    // message if it finished unsuccessfully.
    External { text: String, rx: Receiver<Option<String>> },
    // A video is loaded (paused) and the playback menu is shown over its
    // still frame. Opening a video and pausing playback are the same state.
    Paused,
    // Fullscreen playback; osd_until keeps the seek overlay (progress bar +
    // time) up during and shortly after seeking.
    Playing { osd_until: Instant },
}

// How often watch progress is persisted during playback, so it survives the
// power cut that usually ends an HTPC session.
const POSITION_SAVE_INTERVAL: Duration = Duration::from_secs(10);

pub struct App {
    tree: Node,
    // The tree being streamed in by a walk, swapped in when it finishes.
    walking: Node,
    pub stack: Vec<Frame>,
    pub mode: Mode,
    pub player: Player,
    // Transient on-screen message (external command failures).
    pub notice: Option<(String, Instant)>,
    // File currently being probed by the background scan, for display.
    pub scan_status: String,
    scan_rx: Option<Receiver<ScanMsg>>,
    // Failures of launched programs, which run without blocking the UI.
    launch_errors: (Sender<Option<String>>, Receiver<Option<String>>),
    // "Starting <name>" indication: since when, and the window ids that
    // already existed (any other id means the app's window appeared).
    pub launching: Option<(String, Instant, Vec<u32>)>,
    last_pos_save: Instant,
    // Other apps' windows, as last reported by the window manager thread.
    windows: Vec<Win>,
    windows_rx: Receiver<Msg>,
}

impl App {
    pub fn new(player: Player, scan_rx: Receiver<ScanMsg>, windows_rx: Receiver<Msg>) -> App {
        App {
            tree: Node::default(),
            walking: Node::default(),
            stack: vec![Frame::Home { menu: Menu::new(Self::home_items(&[], &Node::default(), Self::audio_item()), 0) }],
            mode: Mode::Scanning,
            player,
            notice: None,
            scan_status: String::new(),
            scan_rx: Some(scan_rx),
            launch_errors: channel(),
            launching: None,
            last_pos_save: Instant::now(),
            windows: vec![],
            windows_rx,
        }
    }

    pub fn menu(&mut self) -> &mut Menu<Action> {
        self.stack.last_mut().unwrap().menu_mut()
    }

    // Filesystem path of the current location (track submenus add nothing).
    fn fs_path(&self) -> Vec<String> {
        self.stack
            .iter()
            .filter_map(|frame| match frame {
                Frame::Dir { name, .. } => Some(name.clone()),
                Frame::Video { name, .. } => Some(name.clone()),
                Frame::Home { .. }
                | Frame::Tracks { .. }
                | Frame::Audio { .. }
                | Frame::Pair { .. }
                | Frame::Window { .. } => None,
            })
            .collect()
    }

    // Path of the open video, when one is on the stack. A Video frame is
    // always the last named frame (only Tracks can sit above it), so its
    // path is the whole filesystem path.
    fn video_path(&self) -> Option<Vec<String>> {
        self.stack
            .iter()
            .any(|frame| matches!(frame, Frame::Video { .. }))
            .then(|| self.fs_path())
    }

    // The open video's node in the media tree, when one is on the stack.
    pub fn video_node(&self) -> Option<&Node> {
        media::get_node(&self.tree, &self.video_path()?)
    }

    // (album, track) of the open file when it's an audio file: audio has no
    // video frames to render, so playback shows a now-playing screen instead.
    // The album is the parent folder's name, empty in the library root.
    pub fn now_playing(&self) -> Option<(&str, &str)> {
        let track = self.stack.iter().find_map(|frame| match frame {
            Frame::Video { name, .. } if media::is_audio(name) => Some(media::strip_extension(name)),
            _ => None,
        })?;
        let album = self.stack.iter().rev().find_map(|frame| match frame {
            Frame::Dir { name, .. } => Some(name.as_str()),
            _ => None,
        });
        Some((album.unwrap_or(""), track))
    }

    pub fn title(&self) -> &str {
        match self.stack.last().unwrap() {
            Frame::Home { .. } => "tiny media center",
            Frame::Dir { name, .. } => name,
            Frame::Video { name, .. } => media::strip_extension(name),
            Frame::Tracks { kind, .. } => kind.as_str(),
            Frame::Audio { .. } => "Audio",
            Frame::Pair { .. } => "Pair new (searching...)",
            Frame::Window { id, .. } => self.windows.iter().find(|w| w.id == *id).map_or("", |w| &w.title),
        }
    }

    // Index of a directory item by file name (for restoring the selection).
    fn item_index(items: &[MenuItem<Action>], name: &str) -> Option<usize> {
        items.iter().position(|i| matches!(&i.action, Action::Open(m) if m == name))
    }

    // "Audio [current output]"; runs pactl.
    fn audio_item() -> MenuItem<Action> {
        let default = audio::default_sink();
        let label = match audio::sinks().into_iter().find(|s| s.name == default) {
            Some(sink) => format!("Audio [{}]", sink.description),
            None => "Audio".into(),
        };
        MenuItem::new(label, Action::Audio)
    }

    // Running apps' windows / continue watching / the library root / Audio and
    // Shut down, each under a heading (the first two only with items).
    fn home_items(windows: &[Win], tree: &Node, audio: MenuItem<Action>) -> Vec<MenuItem<Action>> {
        let windows = windows.iter().map(|w| MenuItem::new(w.title.clone(), Action::Window(w.id))).collect();
        let mut items = vec![];
        for (title, section) in [("running", windows), ("continue watching", Self::continue_items(tree))] {
            if !section.is_empty() {
                items.push(MenuItem::heading(title));
                items.extend(section);
            }
        }
        items.push(MenuItem::heading("media"));
        items.extend(Self::directory_items(tree));
        items.push(MenuItem::heading("system"));
        items.push(audio);
        items.push(MenuItem::new("Shut down".into(), Action::Shutdown));
        items
    }

    // Partially watched, not dismissed video; music isn't tracked.
    fn in_progress(name: &str, node: &Node) -> bool {
        !media::is_audio(name) && !node.meta.dismissed && (1..100).contains(&media::watch_pct(node))
    }

    // The latest in-progress videos, newest first (sidecars without a
    // timestamp last). Music isn't tracked.
    fn continue_items(tree: &Node) -> Vec<MenuItem<Action>> {
        fn walk(node: &Node, path: &mut Vec<String>, out: &mut Vec<(Option<u64>, Vec<String>)>) {
            for (name, child) in &node.children {
                path.push(name.clone());
                if child.kind == Kind::Directory {
                    walk(child, path, out);
                } else if App::in_progress(name, child) {
                    out.push((child.meta.watched_at, path.clone()));
                }
                path.pop();
            }
        }
        let mut found = vec![];
        walk(tree, &mut vec![], &mut found);
        found.sort_by(|a, b| b.0.cmp(&a.0));
        found
            .into_iter()
            .take(5)
            .map(|(_, path)| {
                let stem = media::strip_extension(path.last().unwrap());
                // Episodes sit in a show folder below the category (Shows/<show>/...).
                let label = match path.len() {
                    3.. => format!("{} {stem}", path[path.len() - 2]),
                    _ => stem.to_string(),
                };
                MenuItem::new(label, Action::Resume(path))
            })
            .collect()
    }

    // Rebuild Home from the current windows and library, reusing the Audio
    // item (rebuilding it runs pactl).
    fn update_home(&mut self) {
        let Some(Frame::Home { menu }) = self.stack.first_mut() else { return };
        let audio = menu.items.iter().find(|i| i.action == Action::Audio).cloned().unwrap_or_else(Self::audio_item);
        Self::keep_selection(menu, Self::home_items(&self.windows, &self.tree, audio));
    }

    // Index in items of the menu's selected action. Folders list in-progress
    // videos twice, so the match is the one as far from the end as the old.
    fn reselect(menu: &Menu<Action>, items: &[MenuItem<Action>]) -> Option<usize> {
        let action = &menu.items.get(menu.selected)?.action;
        let after = menu.items[menu.selected + 1..].iter().filter(|i| &i.action == action).count();
        let found: Vec<_> = (0..items.len()).filter(|&i| &items[i].action == action).collect();
        found.iter().rev().nth(after).or(found.first()).copied()
    }

    // Replace a menu's items keeping the selection by action: items come and
    // go, and an index could slide onto Shut down.
    fn keep_selection(menu: &mut Menu<Action>, items: Vec<MenuItem<Action>>) {
        let selected = Self::reselect(menu, &items).unwrap_or(0);
        menu.update_items(items, selected);
    }

    // Current outputs (the default selected), then paired but disconnected
    // Bluetooth audio devices (dimmed), then Pair new.
    fn audio_menu() -> Menu<Action> {
        let default = audio::default_sink();
        let sinks = audio::sinks();
        let selected = sinks.iter().position(|s| s.name == default).unwrap_or(0);
        let mut items: Vec<_> =
            sinks.into_iter().map(|s| MenuItem::new(s.description, Action::SetSink(s.name))).collect();
        for device in audio::bt_audio_devices(true).into_iter().filter(|d| !d.connected) {
            let mut item = MenuItem::new(device.name.clone(), Action::ConnectBt(device, false));
            item.dim = true;
            items.push(item);
        }
        items.push(MenuItem::new("Pair new".into(), Action::PairNew));
        Menu::new(items, selected)
    }

    pub fn refresh_home(&mut self) {
        if let [Frame::Home { menu }] = self.stack.as_mut_slice() {
            Self::keep_selection(menu, Self::home_items(&self.windows, &self.tree, Self::audio_item()));
        }
    }

    fn show_notice(&mut self, message: String) {
        eprintln!("tiny-media-center: {message}");
        self.notice = Some((message, Instant::now()));
    }
    // Case-insensitively alphabetical; watched videos are dimmed, not moved.
    fn directory_items(node: &Node) -> Vec<MenuItem<Action>> {
        let mut items: Vec<_> = node
            .children
            .iter()
            .map(|(name, child)| {
                let display = if child.kind == Kind::Directory {
                    name.clone()
                } else {
                    media::strip_extension(name).to_string()
                };
                let mut item = MenuItem::new(display, Action::Open(name.clone()));
                // Music isn't tracked.
                item.dim = !media::is_audio(name) && media::watch_pct(child) >= 100;
                item
            })
            .collect();
        items.sort_by(|a, b| a.jump.cmp(&b.jump));
        items
    }

    // A folder's listing: with in-progress videos in it, they come first under
    // "continue watching" (newest first) and everything under "all".
    fn folder_items(node: &Node) -> Vec<MenuItem<Action>> {
        let items = Self::directory_items(node);
        let mut progress: Vec<_> = node.children.iter().filter(|(name, child)| Self::in_progress(name, child)).collect();
        if progress.is_empty() {
            return items;
        }
        progress.sort_by_key(|(_, child)| std::cmp::Reverse(child.meta.watched_at));
        let mut out = vec![MenuItem::heading("continue watching")];
        for (name, _) in progress {
            out.extend(items.iter().find(|i| i.action == Action::Open(name.clone())).cloned());
        }
        out.push(MenuItem::heading("all"));
        out.extend(items);
        out
    }

    // Play / Audio [current] / Subtitles [current] / Dismiss from suggestions (in-progress only).
    fn video_items(name: &str, video: &Node) -> Vec<MenuItem<Action>> {
        let meta = &video.meta;
        let label = |base: &str, kind| {
            let current = match meta.effective_selection(kind).as_deref() {
                Some("") => Some("none"), // subtitles off
                Some(id) => meta.track_label(kind, id),
                None => None,
            };
            match current {
                Some(current) => format!("{base} [{current}]"),
                None => base.to_string(),
            }
        };
        let mut items = vec![
            MenuItem::new("Play".into(), Action::Play),
            MenuItem::new(label("Audio", TrackKind::Audio), Action::Tracks(TrackKind::Audio)),
            MenuItem::new(label("Subtitles", TrackKind::Sub), Action::Tracks(TrackKind::Sub)),
        ];
        if Self::in_progress(name, video) {
            items.push(MenuItem::new("Dismiss from suggestions".into(), Action::Dismiss));
        }
        items
    }

    // Available tracks sorted by id, with a "none" entry for subtitles.
    fn track_items(video: &Node, kind: TrackKind) -> Vec<MenuItem<Action>> {
        let mut items = vec![];
        if kind == TrackKind::Sub {
            items.push(MenuItem::new("none".into(), Action::SelectTrack(kind, String::new())));
        }
        items.extend(
            video
                .meta
                .track_list(kind)
                .into_iter()
                .map(|(id, label)| MenuItem::new(label, Action::SelectTrack(kind, id))),
        );
        items
    }

    pub fn handle_input(&mut self, input: Input) {
        if matches!(self.mode, Mode::Playing { .. }) {
            match input {
                // Both back and confirm leave playback for the paused menu.
                Input::Confirm | Input::Back => self.pause_player(),
                Input::Home => self.go_home(),
                Input::Left(_) => self.seek_player(-5.0),
                Input::Right(_) => self.seek_player(5.0),
                Input::Up(_) => self.player.add_sub_delay(-0.5),
                Input::Down(_) => self.player.add_sub_delay(0.5),
                Input::Letter(_) | Input::Tab => {}
            }
        } else if matches!(self.mode, Mode::Browse | Mode::Paused) {
            match input {
                Input::Up(repeat) => self.menu().navigate(false, !repeat),
                Input::Down(repeat) => self.menu().navigate(true, !repeat),
                Input::Confirm => self.navigate_in(),
                Input::Back => self.navigate_out(),
                // The Esc, Tab chord: the Esc that just paused playback (which
                // saved, resetting last_pos_save) must not be followed by Home.
                Input::Tab if self.last_pos_save.elapsed() < Duration::from_millis(500) => {}
                Input::Home | Input::Tab => self.go_home(),
                Input::Letter(c) => self.menu().jump_to_letter(c),
                Input::Left(repeat) => self.menu().jump_group(false, !repeat),
                Input::Right(repeat) => self.menu().jump_group(true, !repeat),
            }
        }
    }

    // Pop everything but Home, closing the open video first. Pausing saved
    // the position already; only playback has unsaved progress.
    fn go_home(&mut self) {
        if self.video_path().is_some() {
            if matches!(self.mode, Mode::Playing { .. }) {
                self.save_position();
            }
            self.close_player();
        }
        self.stack.truncate(1);
        self.menu().reset_scroll();
    }

    fn navigate_in(&mut self) {
        let menu = self.menu();
        let Some(action) = menu.items.get(menu.selected).map(|i| i.action.clone()) else { return };
        match action {
            Action::Heading => {}
            Action::Open(name) => self.open(name),
            Action::Play => {
                // The file is already loaded (paused); at EOF (keep-open's
                // pause on the last frame) Play means watch again.
                if self.player.at_eof() {
                    self.player.restart();
                }
                self.player.set_pause(false);
                self.last_pos_save = Instant::now();
                self.mode = Mode::Playing { osd_until: Instant::now() };
            }
            Action::Dismiss => {
                let Some(path) = self.video_path() else { return };
                if let Some(node) = media::get_node_mut(&mut self.tree, &path) {
                    node.meta.dismissed = true;
                    media::save_metadata(MEDIA_PATH, &path, &node.meta);
                }
                self.close_player();
            }
            Action::Tracks(kind) => {
                let Some(video) = self.video_node() else { return };
                let items = Self::track_items(video, kind);
                let current = video.meta.effective_selection(kind);
                let selected = items
                    .iter()
                    .position(|i| matches!(&i.action, Action::SelectTrack(_, id) if Some(id) == current.as_ref()))
                    .unwrap_or(0);
                self.stack.push(Frame::Tracks { kind, menu: Menu::new(items, selected) });
            }
            Action::SelectTrack(kind, id) => self.select_track(kind, id),
            Action::Resume(path) => self.resume(path),
            Action::Audio => self.stack.push(Frame::Audio { menu: Self::audio_menu() }),
            Action::SetSink(name) => {
                if let Some(error) = audio::set_default_sink(&name) {
                    self.show_notice(error);
                }
                self.stack.truncate(1);
                self.refresh_home();
            }
            Action::ConnectBt(device, pair) => {
                self.stack.truncate(1);
                let text = format!("Connecting to {}...", device.name);
                self.run_task(text, move || audio::connect(&device, pair));
            }
            Action::Window(id) => self.stack.push(Self::window_frame(id)),
            Action::Focus(id) => windows::activate(id),
            Action::Close(id) => {
                windows::close(id);
                self.stack.pop();
                self.menu().reset_scroll();
            }
            Action::PairNew => {
                match audio::discover() {
                    Ok(scan) => self.stack.push(Frame::Pair { menu: Menu::new(vec![], 0), scan }),
                    Err(e) => self.show_notice(format!("bluetoothctl: {e}")),
                }
            }
            // systemctl only queues the poweroff job, so this doesn't block.
            Action::Shutdown => {
                let message = match Command::new("systemctl").arg("poweroff").output() {
                    Ok(out) if out.status.success() => "Shutting down...".into(),
                    Ok(out) => format!("shut down: {}", String::from_utf8_lossy(&out.stderr).trim()),
                    Err(e) => format!("shut down: {e}"),
                };
                self.show_notice(message);
            }
        }
    }

    // The stack opening the folders and the video from Home would leave, so
    // Back walks up the library.
    fn resume(&mut self, path: Vec<String>) {
        let Some((file, dirs)) = path.split_last() else { return };
        let mut frames = vec![];
        let mut node = &self.tree;
        for (i, name) in dirs.iter().enumerate() {
            let Some(child) = node.children.get(name) else { return };
            node = child;
            let items = Self::folder_items(node);
            let selected = Self::item_index(&items, &path[i + 1]).unwrap_or(0);
            frames.push(Frame::Dir { name: name.clone(), menu: Menu::new(items, selected) });
        }
        self.stack.extend(frames);
        self.open(file.clone());
    }

    fn open(&mut self, name: String) {
        let mut child_path = self.fs_path();
        child_path.push(name.clone());
        let Some(child) = media::get_node(&self.tree, &child_path) else { return };
        match child.kind {
            Kind::Directory => {
                // Start on the first item not yet watched (watched ones are dimmed).
                let items = Self::folder_items(child);
                let selected = items.iter().position(|i| !i.dim && !i.heading).unwrap_or(0);
                self.stack.push(Frame::Dir { name, menu: Menu::new(items, selected) });
            }
            Kind::Video => {
                // Load right into the player, paused: the video menu is
                // drawn over the still frame from here on.
                let mut items = Self::video_items(&name, child);
                if media::is_audio(&name) {
                    items.truncate(1); // Play only
                }
                let menu = Menu::new(items, 0);
                let meta = &child.meta;
                let start = match (meta.position, meta.duration) {
                    (Some(pos), Some(dur)) if pos < dur - 3.0 => Some(pos),
                    _ => None,
                };
                let file = format!("{}/{}", MEDIA_PATH, child_path.join("/"));
                let aid = meta.effective_selection(TrackKind::Audio);
                let sid = meta.effective_selection(TrackKind::Sub);
                self.player.play(&file, start, aid.as_deref(), sid.as_deref());
                self.stack.push(Frame::Video { name, menu });
                self.mode = Mode::Paused;
            }
            Kind::WiiGame | Kind::Script => self.run_external(child.kind, &child_path),
        }
    }

    // Launch Dolphin / a shell script on a worker thread without waiting for
    // it (failures surface as a notice later), argv-style: no
    // shell parses the path, so file names can't be misread as syntax.
    // Dolphin goes through the systemd user manager: avoids a GL-init
    // segfault, and the game survives this app dying mid-session.
    fn run_external(&mut self, kind: Kind, path: &[String]) {
        let full = format!("{}/{}", MEDIA_PATH, path.join("/"));
        let name = path.last().cloned().unwrap_or_default();
        let is_game = kind == Kind::WiiGame;
        let errors = self.launch_errors.0.clone();
        let shown = name.rsplit_once('.').map_or(name.as_str(), |(stem, _)| stem).to_string();
        self.launching = Some((shown, Instant::now(), self.windows.iter().map(|w| w.id).collect()));
        std::thread::spawn(move || {
            let status = if is_game {
                let mut cmd = Command::new("systemd-run");
                cmd.args(["--user", "--wait", "--collect", "--same-dir", "dolphin-emu"]);
                cmd.args(["--batch", "-C", "Dolphin.Display.Fullscreen=True"])
                    .args(["-C", "Dolphin.Interface.ConfirmStop=False"])
                    .arg(format!("--exec={full}"))
                    .status()
            } else {
                Command::new("bash").arg(&full).status()
            };
            let error = match status {
                Ok(status) if status.success() => None,
                Ok(status) => Some(format!("{name}: {status}")),
                Err(e) => Some(format!("{name}: {e}")),
            };
            let _ = errors.send(error);
        });
    }

    // Run task on a worker thread behind a loading screen showing text; it
    // returns an error message on failure.
    fn run_task(&mut self, text: String, task: impl FnOnce() -> Option<String> + Send + 'static) {
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(task());
        });
        self.mode = Mode::External { text, rx };
    }

    fn select_track(&mut self, kind: TrackKind, id: String) {
        let Some(video_path) = self.video_path() else { return };
        let Some(video) = media::get_node_mut(&mut self.tree, &video_path) else { return };
        video.meta.set_selection(kind, id.clone());
        media::save_metadata(MEDIA_PATH, &video_path, &video.meta);
        // Apply to the loaded file too, so it takes effect right away.
        self.player.set_track(kind.prop(), &id);
        let items = Self::video_items(video_path.last().unwrap(), video);
        let selected = if kind == TrackKind::Audio { 1 } else { 2 };
        self.stack.pop();
        self.menu().set_items(items, selected);
    }

    fn navigate_out(&mut self) {
        match self.stack.last() {
            // Backing out of the paused video menu closes the player.
            Some(Frame::Video { .. }) => self.close_player(),
            _ if self.stack.len() > 1 => {
                self.stack.pop();
                self.menu().reset_scroll();
                self.refresh_home();
            }
            _ => {}
        }
    }

    pub fn save_position(&mut self) {
        let Some(path) = self.video_path() else { return };
        let position = self.player.time_pos;
        if let Some(node) = media::get_node_mut(&mut self.tree, &path) {
            node.meta.mark_watched(position, SystemTime::now().duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs()));
            media::save_metadata(MEDIA_PATH, &path, &node.meta);
        }
        self.last_pos_save = Instant::now();
    }

    // Periodic save during playback, so progress survives a power cut.
    pub fn autosave_position(&mut self) {
        if self.last_pos_save.elapsed() >= POSITION_SAVE_INTERVAL {
            self.save_position();
        }
    }

    // Playing -> Paused: freeze on the current frame and bring the menu up
    // (the same state opening the video lands in).
    pub fn pause_player(&mut self) {
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
    // watch state (dim/progress) is current, keeping the video selected.
    pub fn close_player(&mut self) {
        self.player.stop();
        while matches!(self.stack.last(), Some(Frame::Tracks { .. })) {
            self.stack.pop();
        }
        self.stack.pop_if(|f| matches!(f, Frame::Video { .. }));
        self.mode = Mode::Browse;
        self.refresh_dirs();
        self.menu().reset_scroll();
    }

    // Drain the scan channel: entries stream in during a walk, probe
    // results after. Browsing starts as soon as the first walk is done;
    // later walks (the library changed) swap the tree in silently.
    pub fn pump_scan(&mut self) {
        let Some(rx) = self.scan_rx.take() else { return };
        let mut refresh = false;
        loop {
            match rx.try_recv() {
                Ok(ScanMsg::Status(status)) => self.scan_status = status,
                Ok(ScanMsg::Entry { parts, kind, meta }) => {
                    media::insert(&mut self.walking, parts, kind, meta);
                }
                Ok(ScanMsg::WalkDone) => {
                    let mut tree = std::mem::take(&mut self.walking);
                    media::keep_meta(&self.tree, &mut tree);
                    self.tree = tree;
                    if matches!(self.mode, Mode::Scanning) {
                        self.mode = Mode::Browse;
                    }
                    refresh = true;
                }
                Ok(ScanMsg::Probed { parts, result, subs_done }) => {
                    // Mirror what the scan saved, so later saves from here
                    // (position, track picks) don't write stale values back.
                    if let Some(node) = media::get_node_mut(&mut self.tree, &parts) {
                        node.meta.apply_probe(&result);
                        node.meta.subs_done = subs_done;
                    }
                    refresh = true;
                }
                Ok(ScanMsg::Done) => self.scan_status.clear(),
                Err(TryRecvError::Disconnected) => {
                    self.scan_status.clear();
                    break;
                }
                Err(TryRecvError::Empty) => {
                    self.scan_rx = Some(rx);
                    break;
                }
            }
        }
        if refresh {
            self.refresh_dirs();
        }
    }

    // Bring the directory frames up to date after the tree changed. Playback
    // is left alone (mpv has the file open); a paused video that vanished is
    // closed.
    fn refresh_dirs(&mut self) {
        match self.mode {
            Mode::Paused if self.video_node().is_none() => self.close_player(),
            Mode::Browse | Mode::Paused => sync_dirs(&mut self.stack, &self.tree),
            _ => {}
        }
        self.update_home();
    }

    // Take the latest window list from the window manager; leave window frames
    // whose window (or whole list) went away, and keep the selection.
    pub fn pump_windows(&mut self) {
        let (mut list, mut front, mut home) = (None, None, false);
        for msg in self.windows_rx.try_iter() {
            match msg {
                Msg::List(windows) => list = Some(windows),
                Msg::Front(id) => front = Some(id),
                Msg::Home => home = true,
            }
        }
        if let Some(windows) = list {
            self.update_windows(windows);
        }
        // Like other input, not while busy.
        if home && !matches!(self.mode, Mode::Scanning | Mode::External { .. }) {
            self.handle_input(Input::Home);
        }
        // Not over a video, whose menu and stack stay as they are.
        if let (Some(id), Mode::Browse) = (front, &self.mode) {
            if let Some(selected) = self.stack[0].menu_mut().items.iter().position(|i| i.action == Action::Window(id)) {
                self.stack.truncate(1);
                self.menu().selected = selected;
                self.stack.push(Self::window_frame(id));
                self.menu().reset_scroll();
            }
        }
    }

    fn window_frame(id: Window) -> Frame {
        let items = vec![MenuItem::new("Focus".into(), Action::Focus(id)), MenuItem::new("Close".into(), Action::Close(id))];
        Frame::Window { id, menu: Menu::new(items, 0) }
    }

    fn update_windows(&mut self, windows: Vec<Win>) {
        self.windows = windows;
        let windows = &self.windows;
        if self.launching.as_ref().is_some_and(|(_, _, old)| windows.iter().any(|w| !old.contains(&w.id))) {
            self.launching = None;
        }
        let popped = self
            .stack
            .pop_if(|f| matches!(f, Frame::Window { id, .. } if !windows.iter().any(|w| w.id == *id)))
            .is_some();
        self.update_home();
        if popped {
            self.menu().reset_scroll();
        }
    }

    // Check on a running external command; surface its failure if any.
    pub fn pump_external(&mut self) {
        while let Ok(error) = self.launch_errors.1.try_recv() {
            self.launching = None;
            if let Some(message) = error {
                self.show_notice(message);
            }
        }
        if self.launching.as_ref().is_some_and(|(_, since, _)| since.elapsed() > Duration::from_secs(30)) {
            self.launching = None;
        }
        let Mode::External { rx, .. } = &self.mode else { return };
        let error = match rx.try_recv() {
            Ok(error) => error,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Some("external command thread died".into()),
        };
        if let Some(message) = error {
            self.show_notice(message);
        }
        self.mode = Mode::Browse;
        self.refresh_home();
    }

    // Show the latest Bluetooth discovery results, keeping the selection;
    // leave if the scan died.
    pub fn pump_pair(&mut self) {
        let Some(Frame::Pair { menu, scan }) = self.stack.last_mut() else { return };
        if scan.stopped() {
            self.stack.pop();
            self.show_notice("Bluetooth scan stopped (is Bluetooth on?)".into());
            return;
        }
        let Some(found) = scan.rx.try_iter().last() else { return };
        let selected_mac = match &menu.items.get(menu.selected).map(|i| &i.action) {
            Some(Action::ConnectBt(device, _)) => Some(device.mac.clone()),
            _ => None,
        };
        let selected = found.iter().position(|d| Some(&d.mac) == selected_mac.as_ref()).unwrap_or(0);
        let items = found.into_iter().map(|d| MenuItem::new(d.name.clone(), Action::ConnectBt(d, true))).collect();
        menu.update_items(items, selected);
    }
}

// Rebuild every directory frame from the tree, keeping each selection by
// name (else by index), and pop back to the deepest directory that still
// exists. Other frames are untouched.
fn sync_dirs(stack: &mut Vec<Frame>, tree: &Node) {
    let mut path = vec![];
    let mut keep = stack.len();
    for (i, frame) in stack.iter_mut().enumerate() {
        let Frame::Dir { name, menu } = frame else { continue };
        path.push(name.clone());
        let Some(dir) = media::get_node(tree, &path).filter(|n| n.kind == Kind::Directory) else {
            keep = i;
            break;
        };
        let items = App::folder_items(dir);
        let selected = App::reselect(menu, &items).unwrap_or_else(|| menu.selected.min(items.len().saturating_sub(1)));
        menu.update_items(items, selected);
    }
    if keep < stack.len() {
        stack.truncate(keep);
        stack.last_mut().unwrap().menu_mut().reset_scroll();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::Meta;
    use std::collections::BTreeMap;

    fn video(duration: Option<f64>, position: Option<f64>) -> Node {
        Node {
            kind: Kind::Video,
            children: BTreeMap::new(),
            meta: Meta { duration, position, ..Meta::default() },
        }
    }

    #[test]
    fn builds_the_window_frame() {
        let window = App::window_frame(2);
        assert!(matches!(&window, Frame::Window { id: 2, menu } if menu.selected == 0 && menu.items[0].action == Action::Focus(2)));
    }

    #[test]
    fn directory_items_sort_and_dim() {
        let mut dir = Node::default();
        // Watched (100%), in-progress (50%), unwatched, and a directory.
        dir.children.insert("Delta.mp4".into(), video(Some(100.0), Some(99.0)));
        dir.children.insert("beta.mp4".into(), video(Some(100.0), Some(50.0)));
        dir.children.insert("Alpha.mp4".into(), video(None, None));
        dir.children.insert("shows".into(), Node::default());

        let items = App::directory_items(&dir);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // Plain case-insensitive order, no markers; watched dimmed, not moved.
        assert_eq!(labels, ["Alpha", "beta", "Delta", "shows"]);
        assert_eq!(items.iter().map(|i| i.dim).collect::<Vec<_>>(), [false, false, true, false]);
        assert_eq!(items[2].jump, "delta");

        // Music is never dimmed, however far in.
        let mut dir = Node::default();
        dir.children.insert("b.flac".into(), video(Some(100.0), Some(50.0)));
        dir.children.insert("a.flac".into(), video(Some(100.0), Some(100.0)));
        let items = App::directory_items(&dir);
        assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert!(items.iter().all(|i| !i.dim));
    }

    #[test]
    fn continue_items_newest_first_capped_videos_only() {
        let mut tree = Node::default();
        let mut show = Node::default();
        for (i, at) in [Some(5), None, Some(9), Some(1), Some(7), Some(3)].into_iter().enumerate() {
            let mut ep = video(Some(100.0), Some(50.0));
            ep.meta.watched_at = at;
            show.children.insert(format!("S01E0{i}.mkv"), ep);
        }
        show.children.insert("done.mkv".into(), video(Some(100.0), Some(99.0)));
        let mut shows = Node::default();
        shows.children.insert("Show".into(), show);
        tree.children.insert("Shows".into(), shows);
        let mut movies = Node::default();
        movies.children.insert("Film.mkv".into(), video(Some(100.0), Some(10.0)));
        movies.children.insert("song.flac".into(), video(Some(100.0), Some(10.0)));
        tree.children.insert("Movies".into(), movies);

        let items = App::continue_items(&tree);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // Newest first, capped at 5; the timestamp-less ones and music miss out.
        assert_eq!(labels, ["Show S01E02", "Show S01E04", "Show S01E00", "Show S01E05", "Show S01E03"]);
        tree.children.remove("Shows");
        // Dismissed videos are left out, here and in folder listings.
        tree.children.get_mut("Movies").unwrap().children.get_mut("Film.mkv").unwrap().meta.dismissed = true;
        assert!(App::continue_items(&tree).is_empty());
        assert_eq!(App::folder_items(&tree.children["Movies"]).len(), 2);
        tree.children.get_mut("Movies").unwrap().children.get_mut("Film.mkv").unwrap().meta.dismissed = false;
        assert_eq!(App::continue_items(&tree)[0].label, "Film");
        assert!(matches!(&App::continue_items(&tree)[0].action, Action::Resume(p) if p == &["Movies", "Film.mkv"]));
    }

    #[test]
    fn sync_dirs_keeps_selection_and_pops_to_deepest_existing() {
        let file = |_: &str| video(None, None);
        let mut tree = Node::default();
        let mut show = Node::default();
        for f in ["a.mkv", "b.mkv", "c.mkv"] {
            show.children.insert(f.into(), file(f));
        }
        tree.children.insert("show".into(), show);
        let items = App::directory_items(&tree.children["show"]);
        let mut stack = vec![
            Frame::Home { menu: Menu::new(vec![], 0) },
            Frame::Dir { name: "show".into(), menu: Menu::new(items, 1) },
        ];
        let selected = |stack: &[Frame]| match &stack[1] {
            Frame::Dir { menu, .. } => menu.items[menu.selected].label.clone(),
            _ => String::new(),
        };

        // A file before the selection vanishes: selection follows the name.
        tree.children.get_mut("show").unwrap().children.remove("a.mkv");
        sync_dirs(&mut stack, &tree);
        assert_eq!(selected(&stack), "b");
        assert_eq!(stack[1].menu_mut().selected, 0);
        // The selected file vanishes: same index, clamped.
        tree.children.get_mut("show").unwrap().children.remove("b.mkv");
        sync_dirs(&mut stack, &tree);
        assert_eq!(selected(&stack), "c");
        // The directory we're in disappears: back to Home.
        tree.children.remove("show");
        sync_dirs(&mut stack, &tree);
        assert_eq!(stack.len(), 1);
        assert!(matches!(stack[0], Frame::Home { .. }));
    }

    #[test]
    fn home_layout_with_and_without_sections() {
        let mut tree = Node::default();
        tree.children.insert("Movies".into(), Node::default());
        let mut film = video(Some(100.0), Some(50.0));
        film.meta.watched_at = Some(1);
        tree.children.get_mut("Movies").unwrap().children.insert("Film.mkv".into(), film);
        tree.children.insert("Wii.sh".into(), video(None, None));
        let audio = || MenuItem::new("Audio [x]".into(), Action::Audio);
        let layout = |windows: &[Win], tree: &Node| -> Vec<String> {
            let items = App::home_items(windows, tree, audio());
            items.iter().map(|i| if i.heading { format!("[{}]", i.label) } else { i.label.clone() }).collect()
        };
        let win = Win { id: 1, title: "Browser".into() };
        assert_eq!(
            layout(&[win], &tree),
            ["[running]", "Browser", "[continue watching]", "Film", "[media]", "Movies", "Wii", "[system]", "Audio [x]", "Shut down"]
        );
        // No apps: no "running".
        assert_eq!(layout(&[], &tree)[..2], ["[continue watching]", "Film"]);
        // No continue items either: media and system only.
        tree.children.get_mut("Movies").unwrap().children.clear();
        assert_eq!(layout(&[], &tree), ["[media]", "Movies", "Wii", "[system]", "Audio [x]", "Shut down"]);
        // Home opens on a real item, not its first heading.
        let menu = Menu::new(App::home_items(&[], &tree, audio()), 0);
        assert_eq!(menu.items[menu.selected].label, "Movies");
    }

    #[test]
    fn folder_items_list_in_progress_twice_only_when_present() {
        let mut dir = Node::default();
        let mut e1 = video(Some(100.0), Some(50.0));
        e1.meta.watched_at = Some(1);
        let mut e3 = video(Some(100.0), Some(50.0));
        e3.meta.watched_at = Some(9);
        dir.children.insert("E1.mkv".into(), e1);
        dir.children.insert("E2.mkv".into(), video(Some(100.0), Some(100.0)));
        dir.children.insert("E3.mkv".into(), e3);
        let labels = |items: &[MenuItem<Action>]| -> Vec<String> {
            items.iter().map(|i| if i.heading { format!("[{}]", i.label) } else { i.label.clone() }).collect()
        };
        let items = App::folder_items(&dir);
        assert_eq!(labels(&items), ["[continue watching]", "E3", "E1", "[all]", "E1", "E2", "E3"]);
        assert!(items[5].dim);

        // Selection by action: the "all" copy stays selected when the first copy appears or goes.
        let mut menu = Menu::new(items, 6);
        dir.children.get_mut("E3.mkv").unwrap().meta.position = None;
        let items = App::folder_items(&dir);
        assert_eq!(labels(&items), ["[continue watching]", "E1", "[all]", "E1", "E2", "E3"]);
        App::keep_selection(&mut menu, items);
        assert_eq!(menu.items[menu.selected].label, "E3");
        dir.children.get_mut("E3.mkv").unwrap().meta.position = Some(50.0);
        App::keep_selection(&mut menu, App::folder_items(&dir));
        assert_eq!(menu.selected, 6); // and the copy in "all" it was
        menu.selected = 1;
        App::keep_selection(&mut menu, App::folder_items(&dir));
        assert_eq!(menu.selected, 1); // the top copy stays the top copy

        // Nothing in progress: the plain list, no headings.
        dir.children.retain(|n, _| n == "E2.mkv");
        assert_eq!(labels(&App::folder_items(&dir)), ["E2"]);
    }

    #[test]
    fn keep_selection_follows_window_not_index() {
        let items = |ws: &[(u32, &str)]| -> Vec<MenuItem<Action>> {
            ws.iter().map(|(id, t)| MenuItem::new(t.to_string(), Action::Window(*id))).collect()
        };
        let mut menu = Menu::new(items(&[(1, "a"), (2, "b"), (3, "c")]), 1);
        App::keep_selection(&mut menu, items(&[(0, "z"), (2, "b2"), (3, "c")]));
        assert_eq!(menu.selected, 1);
        App::keep_selection(&mut menu, items(&[(3, "c")]));
        assert_eq!(menu.selected, 0); // selected window gone: first item
    }

    #[test]
    fn video_items_show_current_tracks() {
        let mut node = video(Some(100.0), None);
        node.meta.tracks.insert("track_audio_1".into(), "en/2ch".into());
        node.meta.aid = Some("1".into());
        node.meta.sid = Some("".into());
        let labels: Vec<String> = App::video_items("a.mkv", &node).iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, ["Play", "Audio [en/2ch]", "Subtitles [none]"]);
    }

    #[test]
    fn dismiss_item_only_for_undismissed_in_progress_videos() {
        let mut node = video(Some(100.0), Some(50.0));
        let labels = |node: &Node| -> Vec<String> { App::video_items("a.mkv", node).into_iter().map(|i| i.label).collect() };
        assert_eq!(labels(&node).last().unwrap(), "Dismiss from suggestions");
        node.meta.dismissed = true;
        assert_eq!(labels(&node).len(), 3);
        node.meta.dismissed = false;
        node.meta.position = Some(99.0); // watched
        assert_eq!(labels(&node).len(), 3);
    }

    #[test]
    fn video_items_show_default_tracks_before_any_pick() {
        let mut node = video(Some(100.0), None);
        node.meta.tracks.insert("track_audio_1".into(), "cs/2ch".into());
        node.meta.tracks.insert("track_audio_2".into(), "eng/6ch".into());
        node.meta.tracks.insert("track_sub_3".into(), "eng/SDH".into());
        node.meta.tracks.insert("track_sub_4".into(), "eng".into());
        let labels: Vec<String> = App::video_items("a.mkv", &node).iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, ["Play", "Audio [eng/6ch]", "Subtitles [eng]"]);
    }
    #[test]
    fn track_items_sub_has_none_entry_first() {
        let mut node = video(Some(100.0), None);
        node.meta.tracks.insert("track_sub_2".into(), "en".into());
        node.meta.tracks.insert("track_sub_10".into(), "cs".into());
        let items = App::track_items(&node, TrackKind::Sub);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert_eq!(labels, ["none", "en", "cs"]); // none, then numeric id order
        assert!(matches!(&items[0].action, Action::SelectTrack(TrackKind::Sub, id) if id.is_empty()));
    }
}
