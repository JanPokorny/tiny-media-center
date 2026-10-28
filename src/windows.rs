// EWMH client for the window manager (Openbox): lists other apps' windows
// and activates/closes them by id. Everything degrades to "no windows"
// when there is no X server or the WM doesn't speak EWMH.

use std::sync::mpsc::{channel, Receiver};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt, EventMask, GetPropertyReply,
};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::CURRENT_TIME;

pub use x11rb::protocol::xproto::Window;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        UTF8_STRING,
        _NET_CLIENT_LIST,
        _NET_WM_NAME,
        _NET_WM_WINDOW_TYPE,
        _NET_WM_WINDOW_TYPE_DOCK,
        _NET_WM_WINDOW_TYPE_DESKTOP,
        _NET_ACTIVE_WINDOW,
        _NET_CLOSE_WINDOW,
    }
}

const OWN_CLASS: &str = "tiny-media-center";
// EWMH source indication "pager": the WM lets it override focus stealing prevention.
const SOURCE_PAGER: u32 = 2;

#[derive(Clone, PartialEq, Debug)]
pub struct Win {
    pub id: Window,
    pub title: String,
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

struct X {
    conn: RustConnection,
    root: Window,
    atoms: Atoms,
}

impl X {
    fn connect() -> Option<X> {
        let (conn, screen) = RustConnection::connect(None).ok()?;
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
        Some(X { conn, root, atoms })
    }

    fn property(&self, window: Window, property: impl Into<Atom>, ty: impl Into<Atom>) -> Option<GetPropertyReply> {
        self.conn.get_property(false, window, property, ty, 0, 1 << 16).ok()?.reply().ok()
    }

    fn atoms_of(&self, window: Window, property: Atom, ty: impl Into<Atom>) -> Vec<u32> {
        let reply = self.property(window, property, ty);
        reply.and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default()
    }

    fn clients(&self) -> Vec<Window> {
        self.atoms_of(self.root, self.atoms._NET_CLIENT_LIST, AtomEnum::WINDOW)
    }

    fn class(&self, window: Window) -> String {
        self.property(window, AtomEnum::WM_CLASS, AtomEnum::STRING).map_or_else(String::new, |r| class_of(&r.value))
    }

    // Titled, non-dock, non-desktop windows other than our own.
    fn others(&self, clients: &[Window]) -> Vec<Win> {
        let skipped = [self.atoms._NET_WM_WINDOW_TYPE_DOCK, self.atoms._NET_WM_WINDOW_TYPE_DESKTOP];
        clients
            .iter()
            .filter(|&&id| {
                let types = self.atoms_of(id, self.atoms._NET_WM_WINDOW_TYPE, AtomEnum::ATOM);
                !types.iter().any(|t| skipped.contains(t)) && self.class(id) != OWN_CLASS
            })
            .filter_map(|&id| {
                let net = self.property(id, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING)?;
                let legacy = self.property(id, AtomEnum::WM_NAME, AtomEnum::ANY)?;
                let title = decode_title(&net.value, &legacy.value, legacy.type_ == u32::from(AtomEnum::STRING));
                (!title.is_empty()).then_some(Win { id, title })
            })
            .collect()
    }

    fn client_message(&self, window: Window, message: Atom, data: [u32; 5]) {
        let event = ClientMessageEvent::new(32, window, message, data);
        let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
        // check() waits for the server: the connection is dropped right after.
        let _ = self.conn.send_event(false, self.root, mask, event).map(|c| c.check());
    }
}

pub fn activate(id: Window) {
    if let Some(x) = X::connect() {
        x.client_message(id, x.atoms._NET_ACTIVE_WINDOW, [SOURCE_PAGER, CURRENT_TIME, 0, 0, 0]);
    }
}

// The WM asks the app to close (WM_DELETE_WINDOW) rather than killing it.
pub fn close(id: Window) {
    if let Some(x) = X::connect() {
        x.client_message(id, x.atoms._NET_CLOSE_WINDOW, [CURRENT_TIME, SOURCE_PAGER, 0, 0, 0]);
    }
}

pub fn raise_own() {
    if let Some(id) = X::connect().and_then(|x| x.clients().into_iter().find(|&w| x.class(w) == OWN_CLASS)) {
        activate(id);
    }
}

// Sends the full list of other windows whenever it changes (also when a
// title does); the first list arrives right after startup.
pub fn watch() -> Receiver<Vec<Win>> {
    let (tx, rx) = channel();
    std::thread::spawn(move || {
        let Some(x) = X::connect() else { return };
        let events = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
        let _ = x.conn.change_window_attributes(x.root, &events);
        let watched = [x.atoms._NET_CLIENT_LIST, x.atoms._NET_WM_NAME, AtomEnum::WM_NAME.into()];
        let mut last = vec![];
        loop {
            // Select before listing, so no title change slips between the two.
            let clients = x.clients();
            for &id in &clients {
                let _ = x.conn.change_window_attributes(id, &events);
            }
            let _ = x.conn.flush();
            let windows = x.others(&clients);
            if windows != last {
                last = windows.clone();
                if tx.send(windows).is_err() {
                    return;
                }
            }
            loop {
                match x.conn.wait_for_event() {
                    Ok(Event::PropertyNotify(e)) if watched.contains(&e.atom) => {
                        // A burst of changes costs one relist.
                        while let Ok(Some(_)) = x.conn.poll_for_event() {}
                        break;
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        }
    });
    rx
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
}
