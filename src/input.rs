// Device-independent input: keyboard, gamepad and mouse buttons all map to
// the same semantic inputs here, and the app interprets them per mode. New
// bindings go in this table instead of per-device match arms in the loop.

use sdl3::event::Event;
use sdl3::gamepad::Button;
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;

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
