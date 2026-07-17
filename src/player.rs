// Embedded mpv. Replaces the Lua version's shell-outs:
//  - playback: mpv renders into our GL framebuffer via the render API
//    (was: spawn fullscreen mpv + runtime.lua printing position on stdout)
//  - preflight: a reusable headless handle reads duration/track-list
//    properties (was: spawn mpv + preflight.lua per file)

use libmpv2::events::Event;
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use libmpv2::{mpv_node::MpvNode, Mpv};
use sdl3::VideoSubsystem;
use std::collections::BTreeMap;
use std::ffi::c_void;

// libmpv2's command() joins args into a command *string*, so every arg must
// be quoted for mpv's parser or paths with spaces fall apart.
fn quote(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn command(mpv: &Mpv, name: &str, args: &[&str]) -> Result<(), libmpv2::Error> {
    let quoted: Vec<String> = args.iter().map(|a| quote(a)).collect();
    let quoted: Vec<&str> = quoted.iter().map(String::as_str).collect();
    mpv.command(name, &quoted)
}

pub fn get_proc_address(video: &VideoSubsystem, name: &str) -> *mut c_void {
    match video.gl_get_proc_address(name) {
        Some(f) => f as *mut c_void,
        None => std::ptr::null_mut(),
    }
}

pub struct Player {
    render: RenderContext, // declared before mpv: must be freed first
    pub mpv: Mpv,
    pub time_pos: f64,
}

impl Player {
    pub fn new(video: &VideoSubsystem, mpv_config_dir: &str) -> Result<Player, libmpv2::Error> {
        let mut mpv = Mpv::with_initializer(|init| {
            // Render through our RenderContext instead of letting mpv
            // autoselect vo=gpu, which opens its own window.
            init.set_option("vo", "libmpv")?;
            init.set_option("config", "yes")?;
            init.set_option("config-dir", mpv_config_dir)?;
            init.set_option("idle", "yes")?;
            init.set_option("input-default-bindings", "no")?;
            init.set_option("osd-on-seek", "msg-bar")?;
            init.set_option("sub-font-provider", "none")?;
            init.set_option("sub-font-size", 60_i64)?;
            init.set_option("osd-font-provider", "none")?;
            init.set_option("osd-font-size", 60_i64)?;
            Ok(())
        })?;
        let render = RenderContext::new(
            unsafe { mpv.ctx.as_mut() },
            [
                RenderParam::ApiType(RenderParamApiType::OpenGl),
                RenderParam::InitParams(OpenGLInitParams {
                    get_proc_address,
                    ctx: video.clone(),
                }),
            ],
        )?;
        // Audio visualiser (lavfi-complex) for music files; needs an mpv
        // build with Lua scripting -- ignore failure if unavailable.
        let _ = command(&mpv, "load-script", &[&format!("{mpv_config_dir}/visualiser.lua")]);
        Ok(Player { render, mpv, time_pos: 0.0 })
    }

    pub fn play(&mut self, file: &str, start: Option<f64>, aid: Option<&str>, sid: Option<&str>) {
        // Options set as properties (applied to the next loaded file) rather
        // than a loadfile options arg, whose position changed in mpv 0.38.
        let start_val = start.map_or("none".into(), |s| s.to_string());
        let _ = self.mpv.set_property("start", start_val.as_str());
        let _ = self.mpv.set_property("aid", aid.unwrap_or("auto"));
        let sid = match sid {
            Some("") => "no",
            Some(sid) => sid,
            None => "auto",
        };
        let _ = self.mpv.set_property("sid", sid);
        self.time_pos = start.unwrap_or(0.0);
        let _ = command(&self.mpv, "loadfile", &[file, "replace"]);
    }

    pub fn stop(&self) {
        let _ = command(&self.mpv, "stop", &[]);
    }

    pub fn toggle_pause(&self) {
        let _ = command(&self.mpv, "cycle", &["pause"]);
        // Port of runtime.lua's show-end-time OSD.
        let paused = self.mpv.get_property::<bool>("pause").unwrap_or(false);
        let remaining = self.mpv.get_property::<f64>("time-remaining").unwrap_or(0.0);
        let end = chrono::Local::now() + chrono::Duration::seconds(remaining as i64);
        let text = format!(
            "{} (ends at: {})",
            if paused { "paused" } else { "unpaused" },
            end.format("%H:%M")
        );
        let _ = command(&self.mpv, "show-text", &[&text, "5000"]);
    }

    pub fn seek(&self, secs: f64) {
        let _ = command(&self.mpv, "seek", &[&secs.to_string()]);
    }

    pub fn add_sub_delay(&self, secs: f64) {
        let _ = command(&self.mpv, "add", &["sub-delay", &secs.to_string()]);
    }

    // Track playback position; true once playback has ended.
    pub fn poll_ended(&mut self) -> bool {
        if let Ok(pos) = self.mpv.get_property::<f64>("time-pos") {
            self.time_pos = pos;
        }
        let mut ended = false;
        while let Some(event) = self.mpv.event_context_mut().wait_event(0.0) {
            if let Ok(Event::EndFile(_) | Event::Shutdown) = event {
                ended = true;
            }
        }
        ended
    }

    pub fn render(&self, w: i32, h: i32) {
        let _ = self.render.render::<VideoSubsystem>(0, w, h, true);
    }
}

// Preflight probe (port of preflight.lua): load the file paused with no
// audio/video output, read duration and the track list, formatted exactly as
// before: "lang[/title][/Nch][/codec]" keyed by track_<type>_<id>.
pub fn probe(mpv: &mut Option<Mpv>, file: &str) -> Option<(f64, BTreeMap<String, String>)> {
    if mpv.is_none() {
        *mpv = Mpv::with_initializer(|init| {
            init.set_option("vo", "null")?;
            init.set_option("ao", "null")?;
            init.set_option("pause", "yes")?;
            init.set_option("idle", "yes")?;
            Ok(())
        })
        .ok();
    }
    let mpv = mpv.as_mut()?;
    command(mpv, "loadfile", &[file]).ok()?;
    loop {
        match mpv.event_context_mut().wait_event(30.0)?.ok()? {
            Event::FileLoaded => break,
            Event::EndFile(_) | Event::Shutdown => return None,
            _ => {}
        }
    }

    let duration = mpv.get_property::<f64>("duration").ok()?;
    let mut tracks = BTreeMap::new();
    if let Ok(list) = mpv.get_property::<MpvNode>("track-list") {
        for track in list.array().into_iter().flatten() {
            let map: BTreeMap<String, MpvNode> =
                track.map().map(|m| m.collect()).unwrap_or_default();
            let str_of = |key: &str| map.get(key).and_then(|n| n.str().map(str::to_owned));
            let kind = str_of("type").unwrap_or_default();
            if kind != "audio" && kind != "sub" {
                continue;
            }
            let id = map.get("id").and_then(|n| n.i64()).unwrap_or(0);
            let mut label = str_of("lang").unwrap_or_else(|| "?".into());
            if let Some(title) = str_of("title") {
                label = format!("{label}/{title}");
            }
            if let Some(channels) = map.get("audio-channels").and_then(|n| n.i64()) {
                label = format!("{label}/{channels}ch");
            }
            match str_of("codec") {
                Some(codec) if codec != "subrip" => label = format!("{label}/{codec}"),
                _ => {}
            }
            tracks.insert(format!("track_{kind}_{id}"), label);
        }
    }
    let _ = command(mpv, "stop", &[]);
    Some((duration, tracks))
}

