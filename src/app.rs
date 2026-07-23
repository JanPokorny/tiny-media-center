// Navigation state and playback/scan orchestration. The UI location is a
// single stack of frames (directory menus, the open video's menu, track
// submenus), so there is no parallel path/menu bookkeeping to keep in sync.

use crate::config::Config;
use crate::input::Input;
use crate::media::{self, Kind, Node, ScanMsg, TrackKind};
use crate::menu::{Menu, MenuItem};
use crate::player::Player;
use std::process::{Command, Stdio};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub enum Action {
    Open(String),
    Play,
    Tracks(TrackKind),
    SelectTrack(TrackKind, String),
}

pub enum Frame {
    // name is None for the root directory.
    Dir { name: Option<String>, menu: Menu<Action> },
    Video { name: String, menu: Menu<Action> },
    Tracks { kind: TrackKind, menu: Menu<Action> },
}

impl Frame {
    pub fn menu_mut(&mut self) -> &mut Menu<Action> {
        match self {
            Frame::Dir { menu, .. } | Frame::Video { menu, .. } | Frame::Tracks { menu, .. } => menu,
        }
    }
}

pub enum Mode {
    // Initial scan: loading screen until the directory walk finishes
    // (probing continues in the background afterwards).
    Scanning,
    Browse,
    // An external program (Wii game / shell script) is running; rx delivers
    // an error message if it finished unsuccessfully.
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
    pub config: Config,
    tree: Node,
    pub stack: Vec<Frame>,
    pub mode: Mode,
    pub player: Player,
    // Transient on-screen message (external command failures).
    pub notice: Option<(String, Instant)>,
    // File currently being probed by the background scan, for display.
    pub scan_status: String,
    scan_rx: Option<Receiver<ScanMsg>>,
    last_pos_save: Instant,
}

impl App {
    pub fn new(config: Config, player: Player, scan_rx: Receiver<ScanMsg>) -> App {
        App {
            config,
            tree: Node::default(),
            stack: vec![Frame::Dir { name: None, menu: Menu::new(vec![], 0) }],
            mode: Mode::Scanning,
            player,
            notice: None,
            scan_status: String::new(),
            scan_rx: Some(scan_rx),
            last_pos_save: Instant::now(),
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
                Frame::Tracks { .. } => None,
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
            Frame::Video { name, .. } if name.ends_with(".mp3") => {
                Some(media::strip_extension(name))
            }
            _ => None,
        })
    }

    pub fn title(&self) -> &str {
        match self.stack.last().unwrap() {
            Frame::Dir { name: None, .. } => "tiny media center",
            Frame::Dir { name: Some(name), .. } => name,
            Frame::Video { name, .. } => media::strip_extension(name),
            Frame::Tracks { kind, .. } => kind.as_str(),
        }
    }

    // Index of a directory item by file name (for restoring the selection).
    fn item_index(items: &[MenuItem<Action>], name: &str) -> Option<usize> {
        items.iter().position(|i| matches!(&i.action, Action::Open(m) if m == name))
    }

    // Item construction + sort: in-progress videos first, then unwatched,
    // then watched (dimmed), case-insensitively alphabetical within each
    // group. Jump-to-letter goes by the name, not the "* " progress prefix.
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
                // watch_pct is 0 for anything that isn't a video, so pct > 0
                // alone identifies (partially) watched videos.
                let pct = media::watch_pct(child);
                let category = if pct == 0 { 2 } else if pct < 100 { 1 } else { 3 };
                let jump = display.to_lowercase();
                let label = if category == 1 { format!("* {display}") } else { display };
                let mut item = MenuItem::new(label, Action::Open(name.clone()));
                item.jump = jump;
                item.dim = category == 3;
                (category, item)
            })
            .collect();
        items.sort_by(|(ac, a), (bc, b)| (ac, &a.jump).cmp(&(bc, &b.jump)));
        items.into_iter().map(|(_, item)| item).collect()
    }

    // Play / Audio [current] / Subtitles [current].
    fn video_items(video: &Node) -> Vec<MenuItem<Action>> {
        let meta = &video.meta;
        let label = |base: &str, kind| {
            let current = match meta.selection(kind) {
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
                Input::Left => self.seek_player(-5.0),
                Input::Right => self.seek_player(5.0),
                Input::Up => self.player.add_sub_delay(-0.5),
                Input::Down => self.player.add_sub_delay(0.5),
                Input::Letter(_) => {}
            }
        } else if matches!(self.mode, Mode::Browse | Mode::Paused) {
            match input {
                Input::Up => self.menu().navigate_up(),
                Input::Down => self.menu().navigate_down(),
                Input::Confirm => self.navigate_in(),
                Input::Back => self.navigate_out(),
                Input::Letter(c) => self.menu().jump_to_letter(c),
                Input::Left | Input::Right => {}
            }
        }
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
                let menu = Menu::new(Self::track_items(video, kind), 0);
                self.stack.push(Frame::Tracks { kind, menu });
            }
            Action::SelectTrack(kind, id) => self.select_track(kind, id),
        }
    }

    fn open(&mut self, name: String) {
        let mut child_path = self.fs_path();
        child_path.push(name.clone());
        let Some(child) = media::get_node(&self.tree, &child_path) else { return };
        match child.kind {
            Kind::Directory => {
                let menu = Menu::new(Self::directory_items(child), 0);
                self.stack.push(Frame::Dir { name: Some(name), menu });
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
                let file = format!("{}/{}", self.config.media_path, child_path.join("/"));
                self.player.play(&file, start, meta.aid.as_deref(), meta.sid.as_deref());
                self.stack.push(Frame::Video { name, menu });
                self.mode = Mode::Paused;
            }
            Kind::WiiGame | Kind::Script => self.run_external(child.kind, &child_path),
        }
    }

    // Launch Dolphin / a shell script on a worker thread, argv-style: no
    // shell parses the path, so file names can't be misread as syntax.
    // Dolphin goes through the systemd user manager when available -- the
    // Lua version needed that to avoid a GL-init segfault, and the detour is
    // kept since the game also survives this app dying mid-session.
    fn run_external(&mut self, kind: Kind, path: &[String]) {
        let full = format!("{}/{}", self.config.media_path, path.join("/"));
        let name = path.last().cloned().unwrap_or_default();
        let is_game = kind == Kind::WiiGame;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let status = if is_game {
                let have_systemd = Command::new("systemd-run")
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .is_ok_and(|s| s.success());
                let mut cmd = if have_systemd {
                    let mut cmd = Command::new("systemd-run");
                    cmd.args(["--user", "--wait", "--collect", "--same-dir", "dolphin-emu"]);
                    cmd
                } else {
                    Command::new("dolphin-emu")
                };
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
            let _ = tx.send(error);
        });
        self.mode = Mode::External {
            text: if is_game { "Playing..." } else { "Running..." }.into(),
            rx,
        };
    }

    fn select_track(&mut self, kind: TrackKind, id: String) {
        let Some(video_path) = self.video_path() else { return };
        let Some(video) = media::get_node_mut(&mut self.tree, &video_path) else { return };
        video.meta.set_selection(kind, id.clone());
        media::save_metadata(&self.config.media_path, &video_path, &video.meta);
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
                self.stack.pop();
                self.menu().reset_scroll();
            }
            _ => {}
        }
    }

    pub fn save_position(&mut self) {
        let Some(path) = self.video_path() else { return };
        let position = self.player.time_pos;
        if let Some(node) = media::get_node_mut(&mut self.tree, &path) {
            node.meta.position = Some(position);
            media::save_metadata(&self.config.media_path, &path, &node.meta);
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
        let name = match self.stack.pop_if(|f| matches!(f, Frame::Video { .. })) {
            Some(Frame::Video { name, .. }) => Some(name),
            _ => None,
        };
        let path = self.fs_path();
        if let Some(dir) = media::get_node(&self.tree, &path) {
            let items = Self::directory_items(dir);
            let selected = name.and_then(|n| Self::item_index(&items, &n)).unwrap_or(0);
            self.menu().set_items(items, selected);
        }
        self.mode = Mode::Browse;
    }

    // Drain the scan channel: entries stream in during the walk, probe
    // results after. Browsing starts as soon as the walk is done.
    pub fn pump_scan(&mut self) {
        let Some(rx) = self.scan_rx.take() else { return };
        let mut refresh = false;
        loop {
            match rx.try_recv() {
                Ok(ScanMsg::Status(status)) => self.scan_status = status,
                Ok(ScanMsg::Entry { parts, kind, meta }) => {
                    media::insert(&mut self.tree, parts, kind, meta);
                }
                Ok(ScanMsg::WalkDone) => {
                    let items = Self::directory_items(&self.tree);
                    self.menu().set_items(items, 0);
                    self.mode = Mode::Browse;
                }
                Ok(ScanMsg::Probed { parts, result }) => {
                    if let Some(node) = media::get_node_mut(&mut self.tree, &parts) {
                        node.meta.apply_probe(&result);
                    }
                    refresh = true;
                }
                Ok(ScanMsg::Done) | Err(TryRecvError::Disconnected) => {
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
            self.refresh_dir_menu();
        }
    }

    // Rebuild the visible directory menu after background metadata updates,
    // keeping the selection (by name) and the scroll position.
    fn refresh_dir_menu(&mut self) {
        if !matches!(self.mode, Mode::Browse) || !matches!(self.stack.last(), Some(Frame::Dir { .. })) {
            return;
        }
        let path = self.fs_path();
        let Some(node) = media::get_node(&self.tree, &path) else { return };
        let items = Self::directory_items(node);
        let menu = self.menu();
        let selected_name = menu.items.get(menu.selected).and_then(|i| match &i.action {
            Action::Open(name) => Some(name.clone()),
            _ => None,
        });
        let selected = selected_name
            .and_then(|n| Self::item_index(&items, &n))
            .unwrap_or_else(|| menu.selected.min(items.len().saturating_sub(1)));
        menu.update_items(items, selected);
    }

    // Check on a running external command; surface its failure if any.
    pub fn pump_external(&mut self) {
        let Mode::External { rx, .. } = &self.mode else { return };
        let error = match rx.try_recv() {
            Ok(error) => error,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Some("external command thread died".into()),
        };
        if let Some(message) = error {
            eprintln!("tiny-media-center: {message}");
            self.notice = Some((message, Instant::now()));
        }
        self.mode = Mode::Browse;
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
    fn directory_items_grouping_and_sort() {
        let mut dir = Node::default();
        // Watched (100%), in-progress (50%), unwatched, and a directory.
        dir.children.insert("Zebra.mp4".into(), video(Some(100.0), Some(99.0)));
        dir.children.insert("beta.mp4".into(), video(Some(100.0), Some(50.0)));
        dir.children.insert("Alpha.mp4".into(), video(None, None));
        dir.children.insert("shows".into(), Node::default());

        let items = App::directory_items(&dir);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // In-progress first (starred), then unwatched (case-insensitively
        // sorted: Alpha before shows), watched (dimmed) last.
        assert_eq!(labels, ["* beta", "Alpha", "shows", "Zebra"]);
        assert!(items[3].dim);
        assert!(!items[0].dim);
        // Jump keys ignore the "* " prefix and case.
        assert_eq!(items[0].jump, "beta");
        assert_eq!(items[3].jump, "zebra");
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
