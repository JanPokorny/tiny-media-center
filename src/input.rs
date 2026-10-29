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
    // The flag marks key autorepeat, which must not wrap around menu ends.
    Up(bool),
    Down(bool),
    Left,
    Right,
    Confirm,
    Back,
    Letter(char),
}

// sdl3's own poll_iter panics converting gamepad buttons/axes its enums don't
// know (newer SDL emits e.g. MISC6); drop those events before converting.
pub fn poll(_pump: &mut sdl3::EventPump) -> impl Iterator<Item = Event> {
    use sdl3::sys::{events::*, gamepad::*};
    std::iter::from_fn(|| loop {
        let mut raw = std::mem::MaybeUninit::uninit();
        if !unsafe { SDL_PollEvent(raw.as_mut_ptr()) } {
            return None;
        }
        let raw = unsafe { raw.assume_init() };
        let known = match SDL_EventType(unsafe { raw.r#type }) {
            SDL_EVENT_GAMEPAD_BUTTON_DOWN | SDL_EVENT_GAMEPAD_BUTTON_UP => {
                Button::from_ll(SDL_GamepadButton(unsafe { raw.gbutton.button } as i32)).is_some()
            }
            SDL_EVENT_GAMEPAD_AXIS_MOTION => sdl3::gamepad::Axis::from_ll(SDL_GamepadAxis(
                unsafe { raw.gaxis.axis } as i32,
            ))
            .is_some(),
            _ => true,
        };
        if known {
            return Some(Event::from_ll(raw));
        }
    })
}

pub fn translate(event: &Event) -> Option<Input> {
    match event {
        Event::KeyDown { keycode: Some(key), repeat, .. } => match key {
            Keycode::Up => Some(Input::Up(*repeat)),
            Keycode::Down => Some(Input::Down(*repeat)),
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
            Button::DPadUp => Some(Input::Up(false)),
            Button::DPadDown => Some(Input::Down(false)),
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
    use sdl3::sys::{events::*, gamepad::*};

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

    // SDL events only; no display needed.
    #[test]
    fn poll_drops_unknown_gamepad_inputs() {
        let sdl = sdl3::init().unwrap();
        let mut pump = sdl.event_pump().unwrap();
        let push = |ty: SDL_EventType, button: u8, axis: u8| unsafe {
            let mut e = SDL_Event { r#type: ty.0 };
            match ty {
                SDL_EVENT_GAMEPAD_AXIS_MOTION => e.gaxis.axis = axis,
                _ => e.gbutton.button = button,
            }
            assert!(SDL_PushEvent(&mut e));
        };
        push(SDL_EVENT_GAMEPAD_BUTTON_DOWN, 200, 0);
        push(SDL_EVENT_GAMEPAD_BUTTON_UP, 200, 0);
        push(SDL_EVENT_GAMEPAD_AXIS_MOTION, 0, 200);
        push(SDL_EVENT_GAMEPAD_BUTTON_DOWN, SDL_GAMEPAD_BUTTON_SOUTH.0 as u8, 0);
        let events: Vec<_> = poll(&mut pump).collect();
        assert!(matches!(events[..], [Event::ControllerButtonDown { button: Button::South, .. }]));
    }
}
