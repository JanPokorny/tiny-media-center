// Device-independent input: keyboard, gamepad and mouse buttons all map to
// the same semantic inputs here, and the app interprets them per mode. New
// bindings go in this table instead of per-device match arms in the loop.

use sdl3::event::Event;
use sdl3::gamepad::Button;
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Copy)]
pub enum Input {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Back,
    Letter(char),
}

pub fn translate(event: &Event) -> Option<Input> {
    match event {
        Event::KeyDown { keycode: Some(key), .. } => match key {
            Keycode::Up => Some(Input::Up),
            Keycode::Down => Some(Input::Down),
            Keycode::Left => Some(Input::Left),
            Keycode::Right => Some(Input::Right),
            Keycode::Return => Some(Input::Confirm),
            // Sleep is the power button on some remotes.
            Keycode::Escape | Keycode::AcBack | Keycode::Sleep => Some(Input::Back),
            key => {
                let name = key.name().to_lowercase();
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Some(Input::Letter(c)),
                    _ => None,
                }
            }
        },
        Event::ControllerButtonDown { button, .. } => match button {
            Button::DPadUp => Some(Input::Up),
            Button::DPadDown => Some(Input::Down),
            Button::DPadLeft => Some(Input::Left),
            Button::DPadRight => Some(Input::Right),
            Button::South => Some(Input::Confirm),
            Button::East => Some(Input::Back),
            _ => None,
        },
        Event::MouseButtonDown { mouse_btn: MouseButton::Left, .. } => Some(Input::Confirm),
        Event::MouseButtonDown { mouse_btn: MouseButton::Right, .. } => Some(Input::Back),
        _ => None,
    }
}

// With Steam closed, a Steam Controller's firmware sends View as Esc and Menu
// as Tab from its own keyboard device. Holding both raises the media center
// from any app. Only the controller's evdev nodes are read (not grabbed), so
// real keyboards are unaffected and the app in front still sees the keys.
// Rescans every few seconds to pick up a replugged receiver.
pub fn watch_controller_chord() {
    let watched = Arc::new(Mutex::new(HashSet::new()));
    std::thread::spawn(move || loop {
        let paths = std::fs::read_dir("/dev/input/by-id").into_iter().flatten().flatten().map(|e| e.path());
        for path in paths {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            let is_controller =
                name.starts_with("usb-Valve_Software_Steam_Controller") && name.ends_with("-event-kbd");
            if is_controller && watched.lock().unwrap().insert(path.clone()) {
                let watched = watched.clone();
                std::thread::spawn(move || {
                    let _ = read_chords(&path);
                    watched.lock().unwrap().remove(&path);
                });
            }
        }
        std::thread::sleep(Duration::from_secs(5));
    });
}

fn read_chords(path: &Path) -> std::io::Result<()> {
    let mut file = File::open(path)?;
    let mut held = (false, false);
    // struct input_event on 64-bit: 16-byte timeval, u16 type, u16 code, i32 value.
    let mut ev = [0u8; 24];
    loop {
        file.read_exact(&mut ev)?;
        let kind = u16::from_ne_bytes([ev[16], ev[17]]);
        let code = u16::from_ne_bytes([ev[18], ev[19]]);
        let value = i32::from_ne_bytes([ev[20], ev[21], ev[22], ev[23]]);
        if kind == EV_KEY && chord_step(&mut held, code, value) {
            crate::windows::raise_own();
        }
    }
}

const EV_KEY: u16 = 1;
const KEY_ESC: u16 = 1;
const KEY_TAB: u16 = 15;

// Track Esc/Tab held state; true on the press that completes the chord
// (not on autorepeat, value 2).
fn chord_step(held: &mut (bool, bool), code: u16, value: i32) -> bool {
    match code {
        KEY_ESC => held.0 = value != 0,
        KEY_TAB => held.1 = value != 0,
        _ => return false,
    }
    value == 1 && held.0 && held.1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chord_fires_once_when_both_held() {
        let mut held = (false, false);
        assert!(!chord_step(&mut held, KEY_ESC, 1));
        assert!(chord_step(&mut held, KEY_TAB, 1));
        assert!(!chord_step(&mut held, KEY_TAB, 2)); // autorepeat
        assert!(!chord_step(&mut held, KEY_TAB, 0));
        assert!(!chord_step(&mut held, KEY_ESC, 0));
        assert!(!chord_step(&mut held, KEY_TAB, 1)); // Tab alone
        assert!(!chord_step(&mut held, 30, 1)); // other key
        assert!(chord_step(&mut held, KEY_ESC, 1)); // reverse order
    }
}
