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
use std::time::{Duration, Instant};

#[derive(Clone, PartialEq)]
pub enum Action {
    Open(String),
    Play,
    Tracks(TrackKind),
    SelectTrack(TrackKind, String),
    Browse,
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
    // name is None for the root directory.
    Dir { name: Option<String>, menu: Menu<Action> },
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
    // Selected item of the media root, kept across leaving it for Home.
    root_selection: Option<String>,
    // Other apps' windows, as last reported by the window manager thread.
    windows: Vec<Win>,
    windows_rx: Receiver<Msg>,
}

impl App {
    pub fn new(player: Player, scan_rx: Receiver<ScanMsg>, windows_rx: Receiver<Msg>) -> App {
        App {
            tree: Node::default(),
            walking: Node::default(),
            stack: vec![Frame::Home { menu: Menu::new(Self::home_items(&[]), 0) }],
            mode: Mode::Scanning,
            player,
            notice: None,
            scan_status: String::new(),
            scan_rx: Some(scan_rx),
            launch_errors: channel(),
            launching: None,
            last_pos_save: Instant::now(),
            root_selection: None,
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
                Frame::Dir { name, .. } => name.clone(),
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

    // The open file's display name when it's an audio file: audio has no
    // video frames to render, so playback shows a now-playing screen instead.
    pub fn audio_name(&self) -> Option<&str> {
        self.stack.iter().find_map(|frame| match frame {
            Frame::Video { name, .. } if media::is_audio(name) => {
                Some(media::strip_extension(name))
            }
            _ => None,
        })
    }

    pub fn title(&self) -> &str {
        match self.stack.last().unwrap() {
            Frame::Home { .. } => "tiny media center",
            Frame::Dir { name: None, .. } => "Open",
            Frame::Dir { name: Some(name), .. } => name,
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

    // Other apps' windows / Open / Audio [current output] / Shut down.
    fn home_items(windows: &[Win]) -> Vec<MenuItem<Action>> {
        let default = audio::default_sink();
        let audio = match audio::sinks().into_iter().find(|s| s.name == default) {
            Some(sink) => format!("Audio [{}]", sink.description),
            None => "Audio".into(),
        };
        let mut items = Self::window_items(windows);
        items.push(MenuItem::new("Open".into(), Action::Browse));
        items.push(MenuItem::new(audio, Action::Audio));
        items.push(MenuItem::new("Shut down".into(), Action::Shutdown));
        items
    }

    fn window_items(windows: &[Win]) -> Vec<MenuItem<Action>> {
        windows.iter().map(|w| MenuItem::new(format!("* {}", w.title), Action::Window(w.id))).collect()
    }

    // Replace a menu's items keeping the selection by action: items come and
    // go, and an index could slide onto Shut down.
    fn keep_selection(menu: &mut Menu<Action>, items: Vec<MenuItem<Action>>) {
        let action = menu.items.get(menu.selected).map(|i| &i.action);
        let selected = items.iter().position(|i| Some(&i.action) == action).unwrap_or(0);
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
            Self::keep_selection(menu, Self::home_items(&self.windows));
        }
    }

    fn show_notice(&mut self, message: String) {
        eprintln!("tiny-media-center: {message}");
        self.notice = Some((message, Instant::now()));
    }
    // File name of the selected directory item.
    fn selected_name(menu: &Menu<Action>) -> Option<String> {
        match &menu.items.get(menu.selected)?.action {
            Action::Open(name) => Some(name.clone()),
            _ => None,
        }
    }
    // Item construction + sort: in-progress videos first, then everything
    // else (watched ones dimmed), case-insensitively alphabetical within each
    // group. Jump-to-letter goes by the name, not the "* " progress prefix.
    fn directory_items(node: &Node) -> Vec<MenuItem<Action>> {
        let mut items: Vec<(bool, MenuItem<Action>)> = node
            .children
            .iter()
            .map(|(name, child)| {
                let display = if child.kind == Kind::Directory {
                    name.clone()
                } else {
                    media::strip_extension(name).to_string()
                };
                // watch_pct is 0 for anything that isn't a video, so pct > 0
                // alone identifies (partially) watched videos.
                let pct = media::watch_pct(child);
                let in_progress = pct > 0 && pct < 100;
                let jump = display.to_lowercase();
                let label = if in_progress { format!("* {display}") } else { display };
                let mut item = MenuItem::new(label, Action::Open(name.clone()));
                item.jump = jump;
                item.dim = pct >= 100;
                (!in_progress, item)
            })
            .collect();
        items.sort_by(|(ac, a), (bc, b)| (ac, &a.jump).cmp(&(bc, &b.jump)));
        items.into_iter().map(|(_, item)| item).collect()
    }

    // Play / Audio [current] / Subtitles [current].
    fn video_items(video: &Node) -> Vec<MenuItem<Action>> {
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
        vec![
            MenuItem::new("Play".into(), Action::Play),
            MenuItem::new(label("Audio", TrackKind::Audio), Action::Tracks(TrackKind::Audio)),
            MenuItem::new(label("Subtitles", TrackKind::Sub), Action::Tracks(TrackKind::Sub)),
        ]
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
                Input::Left => self.seek_player(-5.0),
                Input::Right => self.seek_player(5.0),
                Input::Up(_) => self.player.add_sub_delay(-0.5),
                Input::Down(_) => self.player.add_sub_delay(0.5),
                Input::Letter(_) => {}
            }
        } else if matches!(self.mode, Mode::Browse | Mode::Paused) {
            match input {
                Input::Up(repeat) => self.menu().navigate_up(!repeat),
                Input::Down(repeat) => self.menu().navigate_down(!repeat),
                Input::Confirm => self.navigate_in(),
                Input::Back => self.navigate_out(),
                Input::Home => self.go_home(),
                Input::Letter(c) => self.menu().jump_to_letter(c),
                Input::Left | Input::Right => {}
            }
        }
    }

    // Pop everything but Home, closing the open video (position saved) first.
    fn go_home(&mut self) {
        if self.video_path().is_some() {
            self.save_position();
            self.close_player();
        }
        self.stack.truncate(1);
        self.refresh_home();
        self.menu().reset_scroll();
    }

    fn navigate_in(&mut self) {
        let menu = self.menu();
        let Some(action) = menu.items.get(menu.selected).map(|i| i.action.clone()) else { return };
        match action {
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
            Action::Browse => {
                let items = Self::directory_items(&self.tree);
                let selected = self
                    .root_selection
                    .as_ref()
                    .and_then(|n| Self::item_index(&items, n))
                    .unwrap_or_else(|| items.iter().position(|i| !i.dim).unwrap_or(0));
                self.stack.push(Frame::Dir { name: None, menu: Menu::new(items, selected) });
            }
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

    fn open(&mut self, name: String) {
        let mut child_path = self.fs_path();
        child_path.push(name.clone());
        let Some(child) = media::get_node(&self.tree, &child_path) else { return };
        match child.kind {
            Kind::Directory => {
                // Start on the first item not yet watched (watched ones are dimmed).
                let items = Self::directory_items(child);
                let selected = items.iter().position(|i| !i.dim).unwrap_or(0);
                self.stack.push(Frame::Dir { name: Some(name), menu: Menu::new(items, selected) });
            }
            Kind::Video => {
                // Load right into the player, paused: the video menu is
                // drawn over the still frame from here on.
                let menu = Menu::new(Self::video_items(child), 0);
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
        let items = Self::video_items(video);
        let selected = if kind == TrackKind::Audio { 1 } else { 2 };
        self.stack.pop();
        self.menu().set_items(items, selected);
    }

    fn navigate_out(&mut self) {
        match self.stack.last() {
            // Backing out of the paused video menu closes the player.
            Some(Frame::Video { .. }) => self.close_player(),
            _ if self.stack.len() > 1 => {
                if let Some(Frame::Dir { name: None, menu }) = self.stack.pop() {
                    self.root_selection = Self::selected_name(&menu);
                }
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
            node.meta.position = Some(position);
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
    // watch state (star/dim/progress) is current, keeping the video selected.
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
        if home {
            self.handle_input(Input::Home);
        }
        // Not over a video, whose menu and stack stay as they are.
        if let (Some(id), Mode::Browse) = (front, &self.mode) {
            if let Some(selected) = self.windows.iter().position(|w| w.id == id) {
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
        // Swap only the window items: rebuilding the whole of Home runs pactl.
        if let Some(Frame::Home { menu }) = self.stack.first_mut() {
            let mut items = Self::window_items(windows);
            items.extend(menu.items.iter().filter(|i| !matches!(i.action, Action::Window(_))).cloned());
            Self::keep_selection(menu, items);
        }
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
        path.extend(name.clone());
        let Some(dir) = media::get_node(tree, &path).filter(|n| n.kind == Kind::Directory) else {
            keep = i;
            break;
        };
        let items = App::directory_items(dir);
        let selected = App::selected_name(menu)
            .and_then(|n| App::item_index(&items, &n))
            .unwrap_or_else(|| menu.selected.min(items.len().saturating_sub(1)));
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
    fn directory_items_grouping_and_sort() {
        let mut dir = Node::default();
        // Watched (100%), in-progress (50%), unwatched, and a directory.
        dir.children.insert("Delta.mp4".into(), video(Some(100.0), Some(99.0)));
        dir.children.insert("beta.mp4".into(), video(Some(100.0), Some(50.0)));
        dir.children.insert("Alpha.mp4".into(), video(None, None));
        dir.children.insert("shows".into(), Node::default());

        let items = App::directory_items(&dir);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // In-progress first (starred), then the rest case-insensitively
        // sorted (Alpha before shows), watched dimmed but not moved.
        assert_eq!(labels, ["* beta", "Alpha", "Delta", "shows"]);
        assert!(items[2].dim);
        assert!(!items[0].dim);
        // Jump keys ignore the "* " prefix and case.
        assert_eq!(items[0].jump, "beta");
        assert_eq!(items[2].jump, "delta");
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
        tree.children.insert("z.mkv".into(), file("z.mkv"));
        let dir = |name: Option<&str>, node: &Node, selected: &str| {
            let items = App::directory_items(node);
            let sel = App::item_index(&items, selected).unwrap();
            Frame::Dir { name: name.map(Into::into), menu: Menu::new(items, sel) }
        };
        let mut stack = vec![
            Frame::Home { menu: Menu::new(vec![], 0) },
            dir(None, &tree, "show"),
            dir(Some("show"), &tree.children["show"], "b.mkv"),
        ];
        let selected = |stack: &[Frame], i: usize| match &stack[i] {
            Frame::Dir { menu, .. } => App::selected_name(menu),
            _ => None,
        };

        // A file before the selection vanishes: selection follows the name.
        tree.children.get_mut("show").unwrap().children.remove("a.mkv");
        sync_dirs(&mut stack, &tree);
        assert_eq!(selected(&stack, 2).as_deref(), Some("b.mkv"));
        assert_eq!(stack[2].menu_mut().selected, 0);
        // The selected file vanishes: same index, clamped.
        tree.children.get_mut("show").unwrap().children.remove("b.mkv");
        sync_dirs(&mut stack, &tree);
        assert_eq!(selected(&stack, 2).as_deref(), Some("c.mkv"));
        // New files appear in the root, the selection stays on "show".
        tree.children.insert("a.mkv".into(), file("a.mkv"));
        sync_dirs(&mut stack, &tree);
        assert_eq!(selected(&stack, 1).as_deref(), Some("show"));
        // The directory we're in disappears: back to the root, Home untouched.
        tree.children.remove("show");
        sync_dirs(&mut stack, &tree);
        assert_eq!(stack.len(), 2);
        assert!(matches!(stack[0], Frame::Home { .. }));
        assert_eq!(selected(&stack, 1).as_deref(), Some("z.mkv")); // "show" gone: same index
        // The root emptied out entirely.
        tree.children.clear();
        sync_dirs(&mut stack, &tree);
        assert_eq!(stack.len(), 2);
        assert!(stack[1].menu_mut().items.is_empty());
    }

    #[test]
    fn keep_selection_follows_window_not_index() {
        let win = |id, title: &str| Win { id, title: title.into() };
        let mut menu = Menu::new(App::window_items(&[win(1, "a"), win(2, "b"), win(3, "c")]), 1);
        App::keep_selection(&mut menu, App::window_items(&[win(0, "z"), win(2, "b2"), win(3, "c")]));
        assert_eq!(menu.selected, 1);
        App::keep_selection(&mut menu, App::window_items(&[win(3, "c")]));
        assert_eq!(menu.selected, 0); // selected window gone: first item
    }

    #[test]
    fn video_items_show_current_tracks() {
        let mut node = video(Some(100.0), None);
        node.meta.tracks.insert("track_audio_1".into(), "en/2ch".into());
        node.meta.aid = Some("1".into());
        node.meta.sid = Some("".into());
        let labels: Vec<String> = App::video_items(&node).iter().map(|i| i.label.clone()).collect();
        assert_eq!(labels, ["Play", "Audio [en/2ch]", "Subtitles [none]"]);
    }

    #[test]
    fn video_items_show_default_tracks_before_any_pick() {
        let mut node = video(Some(100.0), None);
        node.meta.tracks.insert("track_audio_1".into(), "cs/2ch".into());
        node.meta.tracks.insert("track_audio_2".into(), "eng/6ch".into());
        node.meta.tracks.insert("track_sub_3".into(), "eng/SDH".into());
        node.meta.tracks.insert("track_sub_4".into(), "eng".into());
        let labels: Vec<String> = App::video_items(&node).iter().map(|i| i.label.clone()).collect();
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
