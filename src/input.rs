// Device-independent input: keyboard, gamepad and mouse buttons all map to
// the same semantic inputs here, and the app interprets them per mode. New
// bindings go in this table instead of per-device match arms in the loop.

use sdl3::event::Event;
use sdl3::gamepad::{Axis, Button};
use sdl3::keyboard::Keycode;
use sdl3::mouse::MouseButton;
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone, Copy)]
pub enum Input {
    // The flag marks key autorepeat, which must not wrap around menu ends.
    // Left/Right seek while playing and jump between letter groups in menus.
    Up(bool),
    Down(bool),
    Left(bool),
    Right(bool),
    Confirm,
    Back,
    // Straight to the Home menu, from anywhere.
    Home,
    // The Steam Controller's Menu button; its View+Menu chord sends Esc, Tab,
    // so it only goes Home outside playback.
    Tab,
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

fn is_steam_comm(comm: &str) -> bool {
    comm.trim_end() == "steam"
}

// Steam's desktop layout already turns controller buttons into key events, and
// it also exposes the pads to SDL, so gamepad buttons would count twice. Cached
// for 1 s so a /proc scan doesn't run per event.
fn steam_running() -> bool {
    static CACHE: Mutex<Option<(Instant, bool)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap();
    if let Some((at, running)) = *cache {
        if at.elapsed() < Duration::from_secs(1) {
            return running;
        }
    }
    let running = std::fs::read_dir("/proc").is_ok_and(|dir| {
        dir.flatten().any(|e| {
            std::fs::read_to_string(e.path().join("comm")).is_ok_and(|c| is_steam_comm(&c))
        })
    });
    *cache = Some((Instant::now(), running));
    running
}

// hid-steam's gamepad node exists while the controller is on and Steam isn't
// running (Steam grabs it).
fn controller_present(devices: &str) -> bool {
    devices.lines().any(|l| l.trim_end() == "N: Name=\"Wireless Steam Controller\"")
}

// True on the scan where the controller turns up with Steam not running. While
// Steam runs the controller counts as connected, so quitting Steam (which
// releases the node) isn't taken for a new connection.
fn should_start_steam(connected: &mut bool, present: bool, running: bool) -> bool {
    let start = present && !*connected && !running;
    *connected = present || running;
    start
}

// Starts Steam silently, detached and with CEF debugging, like session/steam-games.
pub fn start_steam_with_controller() {
    std::thread::spawn(|| {
        let mut connected = false;
        loop {
            let devices = std::fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
            if should_start_steam(&mut connected, controller_present(&devices), steam_running()) {
                eprintln!("tiny-media-center: Steam Controller connected, starting Steam");
                let env = ["DISPLAY", "XAUTHORITY"].iter().filter_map(|v| Some(format!("-E{v}={}", std::env::var(v).ok()?)));
                let _ = Command::new("systemd-run")
                    .args(["--user", "--collect", "--unit=steam"])
                    .args(env)
                    .args(["steam", "-silent", "-cef-enable-debugging"])
                    .status();
            }
            std::thread::sleep(Duration::from_secs(3));
        }
    });
}

const STICK_ENGAGE: i32 = 19660; // 60% of full scale
const STICK_RELEASE: i32 = 9830; // 30%
const STICK_DELAY: Duration = Duration::from_millis(400);
const STICK_REPEAT: Duration = Duration::from_millis(120);

// Left stick as a D-pad: dominant axis only, with hysteresis, and autorepeat
// driven by `tick` since a held stick sends no events.
#[derive(Default)]
pub struct Stick {
    x: i32,
    y: i32,
    // 0..4 = up, down, left, right.
    held: Option<usize>,
    next_repeat: Option<Instant>,
}

impl Stick {
    pub fn event(&mut self, event: &Event) {
        if let Event::ControllerAxisMotion { axis, value, .. } = event {
            match axis {
                Axis::LeftX => self.x = *value as i32,
                Axis::LeftY => self.y = *value as i32,
                _ => {}
            }
        }
    }

    fn along(&self, dir: usize) -> i32 {
        [-self.y, self.y, -self.x, self.x][dir]
    }

    pub fn tick(&mut self, now: Instant) -> Option<Input> {
        let dominant = match (self.x.abs() > self.y.abs(), self.x < 0, self.y < 0) {
            (true, true, _) => 2,
            (true, false, _) => 3,
            (false, _, true) => 0,
            (false, _, false) => 1,
        };
        let target = if self.along(dominant) > STICK_ENGAGE {
            Some(dominant)
        } else {
            self.held.filter(|&d| self.along(d) >= STICK_RELEASE)
        };
        let make = |d: usize, repeat: bool| [Input::Up, Input::Down, Input::Left, Input::Right][d](repeat);
        if target != self.held {
            self.held = target;
            self.next_repeat = target.map(|_| now + STICK_DELAY);
            return target.map(|d| make(d, false));
        }
        let (d, at) = self.held.zip(self.next_repeat)?;
        (now >= at).then(|| {
            self.next_repeat = Some(now + STICK_REPEAT);
            make(d, true)
        })
    }

    // Same Steam gating as buttons; state still advances so a release is seen.
    pub fn poll(&mut self, now: Instant) -> Option<Input> {
        self.tick(now).filter(|_| !steam_running())
    }
}

pub fn translate(event: &Event) -> Option<Input> {
    match event {
        Event::KeyDown { keycode: Some(key), repeat, .. } => match key {
            Keycode::Up => Some(Input::Up(*repeat)),
            Keycode::Down => Some(Input::Down(*repeat)),
            Keycode::Left => Some(Input::Left(*repeat)),
            Keycode::Right => Some(Input::Right(*repeat)),
            Keycode::Return => Some(Input::Confirm),
            // Sleep is the power button on some remotes.
            Keycode::Escape | Keycode::AcBack | Keycode::Sleep => Some(Input::Back),
            Keycode::Menu | Keycode::Application => Some(Input::Home),
            Keycode::Tab => Some(Input::Tab),
            key => {
                let name = key.name().to_lowercase();
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Some(Input::Letter(c)),
                    _ => None,
                }
            }
        },
        Event::ControllerButtonDown { .. } if steam_running() => None,
        Event::ControllerButtonDown { button, .. } => match button {
            Button::DPadUp => Some(Input::Up(false)),
            Button::DPadDown => Some(Input::Down(false)),
            Button::DPadLeft => Some(Input::Left(false)),
            Button::DPadRight => Some(Input::Right(false)),
            Button::South => Some(Input::Confirm),
            Button::East => Some(Input::Back),
            _ => None,
        },
        Event::MouseButtonDown { mouse_btn: MouseButton::Left, .. } => Some(Input::Confirm),
        Event::MouseButtonDown { mouse_btn: MouseButton::Right, .. } => Some(Input::Back),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sdl3::sys::{events::*, gamepad::*};

    #[test]
    fn steam_comm_matching() {
        assert!(is_steam_comm("steam\n"));
        for c in ["steamwebhelper\n", "steam-runtime-l\n", "steam-games\n", "xsteam\n"] {
            assert!(!is_steam_comm(c));
        }
    }

    #[test]
    fn steam_starts_once_per_controller_connection() {
        let devices = "N: Name=\"Wireless Steam Controller\"\nN: Name=\"Steam Controller Puck\"\n";
        assert!(controller_present(devices));
        assert!(!controller_present("N: Name=\"Microsoft X-Box 360 pad 0\"\n"));
        let mut c = false;
        assert!(!should_start_steam(&mut c, false, false)); // off
        assert!(should_start_steam(&mut c, true, false)); // appears
        assert!(!should_start_steam(&mut c, true, false)); // debounced while it stays
        assert!(!should_start_steam(&mut c, false, true)); // Steam grabbed it
        assert!(!should_start_steam(&mut c, true, false)); // Steam quit, node back: no restart
        assert!(!should_start_steam(&mut c, false, false)); // switched off
        assert!(should_start_steam(&mut c, true, false)); // switched on again
        let mut c = false;
        assert!(!should_start_steam(&mut c, true, true)); // Steam already running
    }

    fn stick(x: i32, y: i32) -> Stick {
        Stick { x, y, ..Default::default() }
    }

    fn name(i: Option<Input>) -> &'static str {
        match i {
            Some(Input::Up(false)) => "up",
            Some(Input::Up(true)) => "up-repeat",
            Some(Input::Down(false)) => "down",
            Some(Input::Left(false)) => "left",
            Some(Input::Right(false)) => "right",
            Some(Input::Right(true)) => "right-repeat",
            Some(_) => "other",
            None => "none",
        }
    }

    #[test]
    fn stick_engage_hysteresis_and_dominant_axis() {
        let t = Instant::now();
        let mut s = stick(15000, 0);
        assert_eq!(name(s.tick(t)), "none"); // below 60%
        s.x = 25000;
        s.y = -30000;
        assert_eq!(name(s.tick(t)), "up"); // dominant axis wins
        s.y = 0;
        assert_eq!(name(s.tick(t)), "right"); // direction switch
        s.x = 12000;
        assert_eq!(name(s.tick(t)), "none"); // between thresholds: still held
        s.x = 5000;
        assert_eq!(name(s.tick(t)), "none"); // released
        s.x = 25000;
        assert_eq!(name(s.tick(t)), "right"); // fresh engage
    }

    #[test]
    fn stick_repeat_timing() {
        let t = Instant::now();
        let ms = Duration::from_millis;
        let mut s = stick(32767, 0);
        assert_eq!(name(s.tick(t)), "right");
        assert_eq!(name(s.tick(t + ms(399))), "none");
        assert_eq!(name(s.tick(t + ms(400))), "right-repeat");
        assert_eq!(name(s.tick(t + ms(519))), "none");
        assert_eq!(name(s.tick(t + ms(520))), "right-repeat");
        s.x = 0;
        assert_eq!(name(s.tick(t + ms(700))), "none");
        s.y = 32767;
        assert_eq!(name(s.tick(t + ms(710))), "down"); // repeat timer reset
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
