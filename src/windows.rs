// tmc is the TV's X window manager. No reparenting or decorations: normal
// windows fill the screen, dialogs are centered, and the top of the stack
// has the input focus. It also reports the other windows to the app for the
// Running menu. Only what Steam, Dolphin, VacuumTube and SteamLink need is
// implemented: no iconic state, desktops or frame extents. WM_STATE and
// WM_TAKE_FOCUS are for Proton: without them its windows never hide or activate.

use crate::overlay;
use std::iter::successors;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;
use x11rb::connection::Connection;
use x11rb::errors::ReplyOrIdError;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConfigureNotifyEvent, ConfigureWindowAux,
    ConnectionExt, CreateWindowAux, EventMask, GetPropertyReply, GrabMode, InputFocus, MapState, ModMask, PropMode,
    StackMode, WindowClass, CONFIGURE_NOTIFY_EVENT,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{CURRENT_TIME, NONE};

pub use x11rb::protocol::xproto::Window;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        UTF8_STRING,
        WM_PROTOCOLS,
        WM_DELETE_WINDOW,
        WM_TAKE_FOCUS,
        WM_STATE,
        _NET_SUPPORTED,
        _NET_SUPPORTING_WM_CHECK,
        _NET_CLIENT_LIST,
        _NET_ACTIVE_WINDOW,
        _NET_WM_NAME,
        _NET_WM_STATE,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_NORMAL,
        _NET_WM_WINDOW_TYPE_DIALOG,
        _NET_WM_WINDOW_TYPE_UTILITY,
        _NET_WM_WINDOW_TYPE_SPLASH,
        _NET_WM_WINDOW_TYPE_DROPDOWN_MENU,
        _NET_WM_WINDOW_TYPE_POPUP_MENU,
    }
}

const OWN_CLASS: &str = "tiny-media-center";
// Global keys: keysym and shell command; Home (no command) raises tmc.
const KEYS: [(u32, &str); 4] = [
    (0x1008ff18, ""), // XF86HomePage
    (0x1008ff13, "wpctl set-mute @DEFAULT_AUDIO_SINK@ 0; wpctl set-volume -l 1.5 @DEFAULT_AUDIO_SINK@ 5%+"),
    (0x1008ff11, "wpctl set-volume @DEFAULT_AUDIO_SINK@ 5%-"),
    (0x1008ff12, "wpctl set-mute @DEFAULT_AUDIO_SINK@ toggle"),
];
#[derive(Clone, PartialEq, Debug)]
pub struct Win {
    pub id: Window,
    pub title: String,
}

// To the app: the other windows, or the one that was in front when tmc was raised.
pub enum Msg {
    List(Vec<Win>),
    Front(Window),
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Normal,
    Dialog,
    // Mapped where it asks to be, otherwise left alone.
    Ignore,
}

// WM_CLASS is "instance\0class\0".
fn class_of(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes.split(|&b| b == 0).nth(1).unwrap_or_default()).into()
}

// Prefer the UTF-8 EWMH name; the legacy one is Latin-1 when its type is STRING.
fn decode_title(net: &[u8], legacy: &[u8], legacy_latin1: bool) -> String {
    if !net.is_empty() {
        String::from_utf8_lossy(net).into()
    } else if legacy_latin1 {
        legacy.iter().map(|&b| b as char).collect()
    } else {
        String::from_utf8_lossy(legacy).into()
    }
}

// The first known _NET_WM_WINDOW_TYPE wins (they are listed by preference);
// without one, transients are dialogs (EWMH). An explicit NORMAL transient
// stays normal: Dolphin's render window may be one.
fn classify(types: &[Atom], transient: bool, kinds: &[(Atom, Kind)]) -> Kind {
    let known = types.iter().find_map(|t| kinds.iter().find(|k| k.0 == *t));
    known.map_or(if transient { Kind::Dialog } else { Kind::Normal }, |k| k.1)
}

fn centered(screen: (u16, u16), size: (u16, u16)) -> (i32, i32) {
    let center = |screen: u16, size: u16| ((i32::from(screen) - i32::from(size)) / 2).max(0);
    (center(screen.0, size.0), center(screen.1, size.1))
}

// Moves w to the top of the stack (bottom first) with its transients, and
// theirs, above it.
fn raise(stack: &mut Vec<Window>, w: Window, parent: impl Fn(Window) -> Option<Window>) {
    // Bounded, as WM_TRANSIENT_FOR can form loops.
    let n = stack.len();
    let depth = |s| successors(Some(s), |&p| parent(p)).take(n).position(|a| a == w);
    let mut moved: Vec<_> = stack.iter().filter_map(|&s| Some((depth(s)?, s))).collect();
    moved.sort_by_key(|m| m.0);
    stack.retain(|&s| depth(s).is_none());
    stack.extend(moved.into_iter().map(|m| m.1));
}

struct X {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
}

impl X {
    fn property(&self, window: Window, property: impl Into<Atom>, ty: impl Into<Atom>) -> Option<GetPropertyReply> {
        self.conn.get_property(false, window, property, ty, 0, 1 << 16).ok()?.reply().ok()
    }

    fn atoms_of(&self, window: Window, property: impl Into<Atom>, ty: impl Into<Atom>) -> Vec<u32> {
        let reply = self.property(window, property, ty);
        reply.and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default()
    }

    fn title(&self, window: Window) -> String {
        let net = self.property(window, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING);
        let legacy = self.property(window, AtomEnum::WM_NAME, AtomEnum::ANY);
        match (net, legacy) {
            (Some(net), Some(legacy)) => {
                decode_title(&net.value, &legacy.value, legacy.type_ == u32::from(AtomEnum::STRING))
            }
            _ => String::new(),
        }
    }
}

// Shared by the WM thread and the app (x11rb connections are thread-safe).
static CONN: OnceLock<X> = OnceLock::new();
// tmc's own window, found by WM_CLASS when it maps.
static OWN: AtomicU32 = AtomicU32::new(NONE);
// The notification overlay, which stays above everything else.
static OVERLAY: AtomicU32 = AtomicU32::new(NONE);

struct Client {
    id: Window,
    kind: Kind,
    parent: Option<Window>,
    title: String,
}

struct Wm {
    x: &'static X,
    screen: (u16, u16),
    // In mapping order.
    clients: Vec<Client>,
    // Bottom first.
    stack: Vec<Window>,
    tx: Sender<Msg>,
    // Grabbed keycodes and their KEYS command.
    keys: Vec<(u8, &'static str)>,
}

impl Wm {
    fn init(&mut self) -> Result<(), ReplyOrIdError> {
        let (x, a) = (self.x, self.x.atoms);
        let geometry = x.conn.get_geometry(x.root)?.reply()?;
        self.screen = (geometry.width, geometry.height);
        // Toolkits only use EWMH (fullscreen, activation) when this exists.
        let check = x.conn.generate_id()?;
        x.conn.create_window(0, check, x.root, 0, 0, 1, 1, 0, WindowClass::INPUT_ONLY, 0, &CreateWindowAux::new())?;
        x.conn.change_property8(PropMode::REPLACE, check, a._NET_WM_NAME, a.UTF8_STRING, OWN_CLASS.as_bytes())?;
        for window in [check, x.root] {
            x.conn.change_property32(PropMode::REPLACE, window, a._NET_SUPPORTING_WM_CHECK, AtomEnum::WINDOW, &[check])?;
        }
        let supported =
            [a._NET_SUPPORTING_WM_CHECK, a._NET_CLIENT_LIST, a._NET_ACTIVE_WINDOW, a._NET_WM_STATE, a._NET_WM_STATE_FULLSCREEN];
        x.conn.change_property32(PropMode::REPLACE, x.root, a._NET_SUPPORTED, AtomEnum::ATOM, &supported)?;

        // Intentional simplification: the grab isn't redone on keymap changes (MappingNotify).
        let (min, max) = (x.conn.setup().min_keycode, x.conn.setup().max_keycode);
        let map = x.conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        for (keycode, syms) in (min..=max).zip(map.keysyms.chunks(map.keysyms_per_keycode.into())) {
            if let Some(&(_, command)) = KEYS.iter().find(|k| syms.contains(&k.0)) {
                x.conn.grab_key(false, x.root, ModMask::ANY, keycode, GrabMode::ASYNC, GrabMode::ASYNC)?;
                self.keys.push((keycode, command));
            }
        }

        // Adopt the windows a previous run left mapped.
        for window in x.conn.query_tree(x.root)?.reply()?.children {
            let attributes = x.conn.get_window_attributes(window)?.reply();
            if attributes.is_ok_and(|a| !a.override_redirect && a.map_state == MapState::VIEWABLE) {
                let _ = self.manage(window);
            }
        }
        Ok(())
    }

    fn run(mut self) {
        while let Ok(event) = self.x.conn.wait_for_event() {
            // Errors are BadWindow races with vanishing windows, or a lost
            // connection, which the next wait reports.
            let _ = self.handle(event);
            let _ = self.x.conn.flush();
        }
    }

    fn handle(&mut self, event: Event) -> Result<(), ReplyOrIdError> {
        let (x, a) = (self.x, self.x.atoms);
        match event {
            Event::MapRequest(e) => self.manage(e.window)?,
            Event::UnmapNotify(e) => self.forget(e.window)?,
            Event::DestroyNotify(e) => self.forget(e.window)?,
            Event::ConfigureRequest(e) => {
                if self.clients.iter().any(|c| c.id == e.window && c.kind == Kind::Normal) {
                    // Denied: tell the client its (unchanged) fullscreen geometry.
                    let (width, height) = self.screen;
                    let notify = ConfigureNotifyEvent {
                        response_type: CONFIGURE_NOTIFY_EVENT,
                        sequence: 0,
                        event: e.window,
                        window: e.window,
                        above_sibling: NONE,
                        x: 0,
                        y: 0,
                        width,
                        height,
                        border_width: 0,
                        override_redirect: false,
                    };
                    x.conn.send_event(false, e.window, EventMask::STRUCTURE_NOTIFY, notify)?;
                } else {
                    let aux = ConfigureWindowAux::from_configure_request(&e).sibling(None).stack_mode(None);
                    x.conn.configure_window(e.window, &aux)?;
                }
            }
            // The screen size changed (a game switched resolution).
            Event::ConfigureNotify(e) if e.window == x.root => {
                self.screen = (e.width, e.height);
                for client in &self.clients {
                    let _ = self.place(client);
                }
            }
            Event::ClientMessage(e) if e.type_ == a._NET_ACTIVE_WINDOW && e.window == OWN.load(Relaxed) => self.raise_own()?,
            Event::ClientMessage(e) if e.type_ == a._NET_ACTIVE_WINDOW => self.activate(e.window)?,
            // Normal windows stay fullscreen whatever is asked.
            Event::ClientMessage(e) if e.type_ == a._NET_WM_STATE => {
                if let Some(client) = self.clients.iter().find(|c| c.id == e.window && c.kind == Kind::Normal) {
                    self.place(client)?;
                }
            }
            Event::PropertyNotify(e) if e.atom == a._NET_WM_NAME || e.atom == u32::from(AtomEnum::WM_NAME) => {
                if let Some(client) = self.clients.iter_mut().find(|c| c.id == e.window) {
                    client.title = x.title(e.window);
                    self.send_list();
                }
            }
            Event::KeyPress(e) => match self.keys.iter().find(|k| k.0 == e.detail) {
                Some((_, "")) => self.raise_own()?,
                Some(&(_, command)) => {
                    // Waited for on its own thread, so the WM never blocks and no zombie is left.
                    std::thread::spawn(move || overlay::volume_command(command));
                }
                None => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn manage(&mut self, window: Window) -> Result<(), ReplyOrIdError> {
        let (x, a) = (self.x, self.x.atoms);
        if self.clients.iter().any(|c| c.id == window) {
            return Ok(());
        }
        let types = x.atoms_of(window, a._NET_WM_WINDOW_TYPE, AtomEnum::ATOM);
        let parent = x.atoms_of(window, AtomEnum::WM_TRANSIENT_FOR, AtomEnum::WINDOW).first().copied();
        let kinds = [
            (a._NET_WM_WINDOW_TYPE_NORMAL, Kind::Normal),
            (a._NET_WM_WINDOW_TYPE_DIALOG, Kind::Dialog),
            (a._NET_WM_WINDOW_TYPE_UTILITY, Kind::Dialog),
            (a._NET_WM_WINDOW_TYPE_SPLASH, Kind::Dialog),
            (a._NET_WM_WINDOW_TYPE_DROPDOWN_MENU, Kind::Ignore),
            (a._NET_WM_WINDOW_TYPE_POPUP_MENU, Kind::Ignore),
        ];
        let kind = classify(&types, parent.is_some(), &kinds);
        if kind == Kind::Ignore {
            // Above the managed windows, which a sync put on top since it was last mapped.
            x.conn.configure_window(window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
            x.conn.map_window(window)?;
            return Ok(());
        }
        let class = x.property(window, AtomEnum::WM_CLASS, AtomEnum::STRING).map(|r| class_of(&r.value));
        if class.as_deref() == Some(OWN_CLASS) {
            OWN.store(window, Relaxed);
        }
        x.conn.change_window_attributes(window, &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE))?;
        let client = Client { id: window, kind, parent, title: x.title(window) };
        self.place(&client)?;
        x.conn.change_property32(PropMode::REPLACE, window, a.WM_STATE, a.WM_STATE, &[1, NONE])?; // NormalState
        x.conn.map_window(window)?;
        self.clients.push(client);
        self.stack.push(window);
        self.send_list();
        self.activate(window)
    }

    fn forget(&mut self, window: Window) -> Result<(), ReplyOrIdError> {
        // Unmanaged windows (popups) come and go without restacking or refocusing.
        if !self.clients.iter().any(|c| c.id == window) {
            return Ok(());
        }
        // Withdrawn; a BadWindow if it was destroyed.
        self.x.conn.delete_property(window, self.x.atoms.WM_STATE)?;
        self.clients.retain(|c| c.id != window);
        self.stack.retain(|&w| w != window);
        self.send_list();
        self.sync()
    }

    fn place(&self, client: &Client) -> Result<(), ReplyOrIdError> {
        let (x, a) = (self.x, self.x.atoms);
        let (width, height) = self.screen;
        let aux = if client.kind == Kind::Normal {
            let fullscreen = [a._NET_WM_STATE_FULLSCREEN];
            x.conn.change_property32(PropMode::REPLACE, client.id, a._NET_WM_STATE, AtomEnum::ATOM, &fullscreen)?;
            ConfigureWindowAux::new().x(0).y(0).width(u32::from(width)).height(u32::from(height))
        } else {
            // Intentional simplification: centered on the screen, not the
            // parent, as parents are fullscreen.
            let geometry = x.conn.get_geometry(client.id)?.reply()?;
            let (left, top) = centered(self.screen, (geometry.width, geometry.height));
            ConfigureWindowAux::new().x(left).y(top)
        };
        x.conn.configure_window(client.id, &aux.border_width(0))?;
        Ok(())
    }

    fn activate(&mut self, window: Window) -> Result<(), ReplyOrIdError> {
        raise(&mut self.stack, window, |w| self.clients.iter().find(|c| c.id == w).and_then(|c| c.parent));
        self.sync()
    }

    // Tells the app which window was in front, unless tmc already was.
    fn raise_own(&mut self) -> Result<(), ReplyOrIdError> {
        let own = OWN.load(Relaxed);
        if let Some(&top) = self.stack.last().filter(|&&t| t != own) {
            let _ = self.tx.send(Msg::Front(top));
        }
        self.activate(own)
    }

    // Applies the stack and focuses its top.
    fn sync(&self) -> Result<(), ReplyOrIdError> {
        let (x, a) = (self.x, self.x.atoms);
        for &window in &self.stack {
            x.conn.configure_window(window, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        }
        raise_overlay();
        let ids: Vec<_> = self.clients.iter().map(|c| c.id).collect();
        x.conn.change_property32(PropMode::REPLACE, x.root, a._NET_CLIENT_LIST, AtomEnum::WINDOW, &ids)?;
        let top = self.stack.last().copied().unwrap_or(NONE);
        x.conn.change_property32(PropMode::REPLACE, x.root, a._NET_ACTIVE_WINDOW, AtomEnum::WINDOW, &[top])?;
        if top != NONE {
            x.conn.set_input_focus(InputFocus::POINTER_ROOT, top, CURRENT_TIME)?;
            if x.atoms_of(top, a.WM_PROTOCOLS, AtomEnum::ATOM).contains(&a.WM_TAKE_FOCUS) {
                let event = ClientMessageEvent::new(32, top, a.WM_PROTOCOLS, [a.WM_TAKE_FOCUS, CURRENT_TIME, 0, 0, 0]);
                x.conn.send_event(false, top, EventMask::NO_EVENT, event)?;
            }
        }
        Ok(())
    }

    // Titled windows other than our own.
    fn send_list(&self) {
        let own = OWN.load(Relaxed);
        let others = self.clients.iter().filter(|c| c.id != own && !c.title.is_empty());
        let _ = self.tx.send(Msg::List(others.map(|c| Win { id: c.id, title: c.title.clone() }).collect()));
    }
}

// Becomes the window manager, or exits if there is another one. The list of
// other windows arrives on the returned channel whenever it may have changed.
pub fn start() -> Receiver<Msg> {
    let connect = || {
        let (conn, screen) = RustConnection::connect(None).ok()?;
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
        Some(X { conn, root, atoms })
    };
    let Some(x) = connect() else {
        eprintln!("cannot connect to the X server");
        std::process::exit(1);
    };
    let x = CONN.get_or_init(|| x);
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY | EventMask::STRUCTURE_NOTIFY;
    // Only one client at a time can redirect the root's children.
    let redirect = x.conn.change_window_attributes(x.root, &ChangeWindowAttributesAux::new().event_mask(mask));
    if !redirect.is_ok_and(|c| c.check().is_ok()) {
        eprintln!("another window manager is running");
        std::process::exit(1);
    }
    let (tx, rx) = channel();
    let mut wm = Wm { x, screen: (0, 0), clients: vec![], stack: vec![], tx, keys: vec![] };
    let _ = wm.init();
    std::thread::spawn(move || wm.run());
    rx
}

// Handled by the WM thread, which owns the stack.
pub fn activate(id: Window) {
    let x = CONN.wait();
    let event = ClientMessageEvent::new(32, id, x.atoms._NET_ACTIVE_WINDOW, [0; 5]);
    let _ = x.conn.send_event(false, x.root, EventMask::SUBSTRUCTURE_REDIRECT, event);
    let _ = x.conn.flush();
}

// Asks the app to close (WM_DELETE_WINDOW) if it can, else disconnects it.
pub fn close(id: Window) {
    let (x, a) = (CONN.wait(), CONN.wait().atoms);
    if x.atoms_of(id, a.WM_PROTOCOLS, AtomEnum::ATOM).contains(&a.WM_DELETE_WINDOW) {
        let event = ClientMessageEvent::new(32, id, a.WM_PROTOCOLS, [a.WM_DELETE_WINDOW, CURRENT_TIME, 0, 0, 0]);
        let _ = x.conn.send_event(false, id, EventMask::NO_EVENT, event);
    } else {
        let _ = x.conn.kill_client(id);
    }
    let _ = x.conn.flush();
}

pub fn set_overlay(id: Window) {
    OVERLAY.store(id, Relaxed);
}

pub fn raise_overlay() {
    let overlay = OVERLAY.load(Relaxed);
    if overlay != NONE {
        let x = CONN.wait();
        let _ = x.conn.configure_window(overlay, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
        let _ = x.conn.flush();
    }
}

pub fn raise_own() {
    activate(OWN.load(Relaxed));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_wm_class_and_titles() {
        assert_eq!(class_of(b"steam\0Steam\0"), "Steam");
        assert_eq!(class_of(b"tiny-media-center\0tiny-media-center\0"), OWN_CLASS);
        assert_eq!(class_of(b""), "");
        assert_eq!(decode_title("Čaj".as_bytes(), b"legacy", true), "Čaj");
        assert_eq!(decode_title(b"", b"legacy", true), "legacy");
        assert_eq!(decode_title(b"", b"caf\xe9", true), "café"); // Latin-1
        assert_eq!(decode_title(b"", "café".as_bytes(), false), "café"); // UTF8_STRING / compound
    }

    #[test]
    fn classifies_by_first_known_type_then_transience() {
        let kinds = [(1, Kind::Normal), (2, Kind::Dialog), (3, Kind::Ignore)];
        assert_eq!(classify(&[], false, &kinds), Kind::Normal);
        assert_eq!(classify(&[], true, &kinds), Kind::Dialog);
        assert_eq!(classify(&[99, 2], false, &kinds), Kind::Dialog); // unknown type skipped
        assert_eq!(classify(&[1], true, &kinds), Kind::Normal); // explicit normal transient
        assert_eq!(classify(&[3, 2], true, &kinds), Kind::Ignore);
        assert_eq!(classify(&[99], true, &kinds), Kind::Dialog);
    }

    #[test]
    fn centers_and_clamps_to_screen() {
        assert_eq!(centered((1920, 1080), (400, 200)), (760, 440));
        assert_eq!(centered((1920, 1080), (2000, 1080)), (0, 0));
    }

    #[test]
    fn raises_with_transients_above() {
        // 4 is a dialog of 1, 5 a dialog of 4; 6 and 7 are transient for each other.
        let parent = |w| match w {
            4 => Some(1),
            5 => Some(4),
            6 => Some(7),
            7 => Some(6),
            _ => None,
        };
        let mut stack = vec![5, 1, 2, 4, 3];
        raise(&mut stack, 1, parent);
        assert_eq!(stack, [2, 3, 1, 4, 5]);
        raise(&mut stack, 2, parent);
        assert_eq!(stack, [3, 1, 4, 5, 2]);
        raise(&mut stack, 4, parent);
        assert_eq!(stack, [3, 1, 2, 4, 5]);
        raise(&mut stack, 9, parent); // unmanaged: no change
        assert_eq!(stack, [3, 1, 2, 4, 5]);
        let mut stack = vec![6, 7, 1];
        raise(&mut stack, 6, parent);
        assert_eq!(stack, [1, 6, 7]); // the loop terminates
    }
}
