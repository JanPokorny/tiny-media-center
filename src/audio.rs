// Audio output selection: sinks through pactl, Bluetooth audio devices
// through bluetoothctl. Everything here shells out and blocks; the slow parts
// (connecting, pairing, discovery) run on worker threads.

use crate::bluetooth::Pairing;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::{Duration, Instant};

pub struct Sink {
    pub name: String,
    pub description: String,
}

#[derive(Clone, PartialEq)]
pub struct BtDevice {
    pub mac: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

fn stdout(program: &str, args: &[&str]) -> String {
    // pactl translates its "Name:"/"Description:" labels.
    Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .stderr(Stdio::null())
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).into_owned())
        .unwrap_or_default()
}

// `pactl list sinks` blocks: "Sink #n", then indented "Name:" and
// "Description:" lines (properties use `key = "value"`, so no clash).
fn parse_sinks(text: &str) -> Vec<Sink> {
    let mut sinks: Vec<Sink> = vec![];
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix("Name: ") {
            sinks.push(Sink { name: name.into(), description: name.into() });
        } else if let (Some(desc), Some(sink)) = (line.strip_prefix("Description: "), sinks.last_mut()) {
            sink.description = desc.into();
        }
    }
    sinks.retain(|s| s.name != "auto_null");
    sinks
}

pub fn sinks() -> Vec<Sink> {
    parse_sinks(&stdout("pactl", &["list", "sinks"]))
}

pub fn default_sink() -> String {
    stdout("pactl", &["get-default-sink"]).trim().to_string()
}

pub fn set_default_sink(name: &str) -> Option<String> {
    match Command::new("pactl").args(["set-default-sink", name]).output() {
        Ok(out) if out.status.success() => None,
        Ok(out) => Some(format!("audio: {}", String::from_utf8_lossy(&out.stderr).trim())),
        Err(e) => Some(format!("audio: {e}")),
    }
}

// Known (or only paired) Bluetooth devices whose class says audio (headsets,
// speakers, ...). One bluetoothctl call per device: fine for the handful a TV
// knows about. Every bluetoothctl call goes through timeout(1), since without
// bluetoothd it waits forever (its own --timeout always waits it out).
pub fn bt_audio_devices(paired_only: bool) -> Vec<BtDevice> {
    let mut args = vec!["5", "bluetoothctl", "devices"];
    if paired_only {
        args.push("Paired");
    }
    stdout("timeout", &args)
        .lines()
        .filter_map(|line| line.strip_prefix("Device ")?.split_once(' '))
        .filter_map(|(mac, name)| {
            let info = stdout("timeout", &["5", "bluetoothctl", "info", mac]);
            let field = |key: &str| {
                info.lines().find_map(|l| l.trim().strip_prefix(key).map(str::to_string))
            };
            field("Icon: ")?.starts_with("audio-").then(|| BtDevice {
                mac: mac.into(),
                name: name.into(),
                paired: field("Paired: ").as_deref() == Some("yes"),
                connected: field("Connected: ").as_deref() == Some("yes"),
            })
        })
        .collect()
}

// `timeout <secs> bluetoothctl ...`; bluetoothctl prints its failure reason
// as the last stdout line.
fn btctl(args: &[&str]) -> Result<(), String> {
    let out = Command::new("timeout").args(args).output().map_err(|e| e.to_string())?;
    if out.status.success() {
        return Ok(());
    }
    let text = String::from_utf8_lossy(&out.stdout);
    Err(text.lines().last().unwrap_or("bluetoothctl failed").to_string())
}

// Trust (so connections the device opens itself need no agent prompt, which
// nobody can answer with a remote), optionally pair, connect, then wait for
// its sink to show up and make it the default output.
pub fn connect(device: &BtDevice, pair: bool) -> Option<String> {
    let mac = device.mac.as_str();
    // The Pair new screen has closed by now; its pairing requests still come.
    let _pairing = pair.then(Pairing::start);
    let trust: &[&str] = &["5", "bluetoothctl", "trust", mac];
    let pair_step: &[&str] = &["30", "bluetoothctl", "pair", mac];
    let connect: &[&str] = &["20", "bluetoothctl", "connect", mac];
    let steps = if pair { vec![trust, pair_step, connect] } else { vec![trust, connect] };
    if let Err(e) = steps.iter().try_for_each(|args| btctl(args)) {
        return Some(format!("{}: {e}", device.name));
    }
    let prefix = format!("bluez_output.{}", mac.replace(':', "_"));
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        if let Some(sink) = sinks().into_iter().find(|s| s.name.starts_with(&prefix)) {
            return set_default_sink(&sink.name);
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Some(format!("{}: no audio output appeared", device.name))
}

// A running scan for unpaired audio devices: rx gets the current list every
// second. Dropping it stops the scan.
pub struct Discovery {
    pub rx: Receiver<Vec<BtDevice>>,
    // Discovery lasts as long as the bluetoothctl client that started it.
    scan: Child,
    _pairing: Pairing,
}

impl Discovery {
    // The scan ended by itself: adapter off/blocked, or its 10 minutes ran out.
    pub fn stopped(&mut self) -> bool {
        !matches!(self.scan.try_wait(), Ok(None))
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.scan.kill();
        let _ = self.scan.wait();
    }
}

pub fn discover() -> std::io::Result<Discovery> {
    let scan = Command::new("bluetoothctl")
        .args(["--timeout", "600", "scan", "on"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let (tx, rx) = channel();
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let found = bt_audio_devices(false).into_iter().filter(|d| !d.paired).collect();
        if tx.send(found).is_err() {
            break;
        }
    });
    Ok(Discovery { rx, scan, _pairing: Pairing::start() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_sinks_names_and_descriptions() {
        let text = "Sink #57\n\tState: SUSPENDED\n\tName: alsa_output.hdmi\n\tDescription: HDMI\n\
                    \tProperties:\n\t\tdevice.description = \"HDMI\"\n\
                    Sink #60\n\tName: auto_null\n\tDescription: Dummy Output\n\
                    Sink #61\n\tName: bluez_output.3C_B0.1\n\tDescription: CMF Buds\n";
        let sinks = parse_sinks(text);
        let pairs: Vec<(&str, &str)> =
            sinks.iter().map(|s| (s.name.as_str(), s.description.as_str())).collect();
        assert_eq!(pairs, [("alsa_output.hdmi", "HDMI"), ("bluez_output.3C_B0.1", "CMF Buds")]);
    }
}
