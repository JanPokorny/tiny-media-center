// The media tree: the Node tree, .tmc metadata sidecars, watch percentage,
// and the background scan thread (walk + subliminal + mpv preflight probe).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    Directory,
    Video,
    WiiGame,
    Script,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Sub,
}

impl TrackKind {
    // The track type name mpv uses in track-list (and our .tmc track keys).
    pub fn as_str(self) -> &'static str {
        match self {
            TrackKind::Audio => "audio",
            TrackKind::Sub => "sub",
        }
    }

    // The mpv property that selects a track of this kind.
    pub fn prop(self) -> &'static str {
        match self {
            TrackKind::Audio => "aid",
            TrackKind::Sub => "sid",
        }
    }
}

// The flattened .tmc key a track's label is stored under.
pub fn track_key(kind: &str, id: &str) -> String {
    format!("track_{kind}_{id}")
}

fn is_false(b: &bool) -> bool {
    !b
}

// .tmc sidecar contents. Kept byte-compatible with the files written by the
// Lua version: duration/position are TOML numbers, aid/sid are strings, and
// track labels live in flattened `track_<type>_<id>` string keys.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Meta {
    // serde deserializes f64 from TOML integers too, so `duration = 120` parses.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    // Negative cache: the preflight probe failed, don't retry (and re-run
    // subliminal) on every launch. Delete the .tmc to force a retry.
    #[serde(default, skip_serializing_if = "is_false")]
    pub probe_failed: bool,
    #[serde(flatten)]
    pub tracks: BTreeMap<String, String>,
}

impl Meta {
    // The selected track id for a kind (aid/sid); "" means subtitles off.
    pub fn selection(&self, kind: TrackKind) -> Option<&str> {
        match kind {
            TrackKind::Audio => self.aid.as_deref(),
            TrackKind::Sub => self.sid.as_deref(),
        }
    }

    pub fn set_selection(&mut self, kind: TrackKind, id: String) {
        match kind {
            TrackKind::Audio => self.aid = Some(id),
            TrackKind::Sub => self.sid = Some(id),
        }
    }

    pub fn track_label(&self, kind: TrackKind, id: &str) -> Option<&str> {
        self.tracks.get(&track_key(kind.as_str(), id)).map(String::as_str)
    }

    // Record a preflight probe's outcome (see ScanMsg::Probed).
    pub fn apply_probe(&mut self, result: &Option<(f64, BTreeMap<String, String>)>) {
        match result {
            Some((duration, tracks)) => {
                self.duration = Some(*duration);
                self.tracks = tracks.clone();
                self.probe_failed = false;
            }
            None => self.probe_failed = true,
        }
    }

    // All tracks of one kind as (id, label), sorted by numeric id.
    pub fn track_list(&self, kind: TrackKind) -> Vec<(String, String)> {
        let prefix = format!("track_{}_", kind.as_str());
        let mut list: Vec<(i64, String, String)> = self
            .tracks
            .iter()
            .filter_map(|(key, label)| {
                let id = key.strip_prefix(&prefix)?;
                Some((id.parse().unwrap_or(0), id.to_string(), label.clone()))
            })
            .collect();
        list.sort_by_key(|(id, _, _)| *id);
        list.into_iter().map(|(_, id, label)| (id, label)).collect()
    }
}

// A node is named by its key in the parent's children map; the path to it is
// whatever segments the caller walked to reach it.
#[derive(Default)]
pub struct Node {
    pub kind: Kind,
    pub children: BTreeMap<String, Node>,
    pub meta: Meta,
}

pub fn strip_extension(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(i) if i > 0 && i + 1 < filename.len() => &filename[..i],
        _ => filename,
    }
}

// Sidecar path: the media file's full name + ".tmc", so movie.mp4 and
// movie.mkv next to each other don't share a sidecar.
fn tmc_path(media_path: &str, node_path: &[String]) -> String {
    format!("{}/{}.tmc", media_path, node_path.join("/"))
}

// Earlier versions replaced the extension instead of appending; still read
// as a fallback so existing libraries keep their positions.
fn legacy_tmc_path(media_path: &str, node_path: &[String]) -> String {
    let rel = node_path.join("/");
    format!("{}/{}.tmc", media_path, strip_extension(&rel))
}

pub fn load_metadata(media_path: &str, node_path: &[String]) -> Meta {
    for path in [tmc_path(media_path, node_path), legacy_tmc_path(media_path, node_path)] {
        let Ok(s) = std::fs::read_to_string(&path) else { continue };
        return toml::from_str(&s).unwrap_or_else(|e| {
            eprintln!("tiny-media-center: ignoring unparsable metadata {path}: {e}");
            Meta::default()
        });
    }
    Meta::default()
}

// Write-to-temp + rename, so a power cut mid-write can't leave a corrupt
// sidecar (which would silently reset watch progress and re-trigger probing).
pub fn save_metadata(media_path: &str, node_path: &[String], meta: &Meta) {
    let path = tmc_path(media_path, node_path);
    let tmp = format!("{path}.tmp");
    let result = toml::to_string(meta)
        .map_err(std::io::Error::other)
        .and_then(|s| std::fs::write(&tmp, s).and_then(|()| std::fs::rename(&tmp, &path)));
    if let Err(e) = result {
        eprintln!("tiny-media-center: failed to save metadata {path}: {e}");
    }
}

pub fn get_node<'a>(root: &'a Node, path: &[String]) -> Option<&'a Node> {
    path.iter().try_fold(root, |node, seg| node.children.get(seg))
}

pub fn get_node_mut<'a>(root: &'a mut Node, path: &[String]) -> Option<&'a mut Node> {
    path.iter().try_fold(root, |node, seg| node.children.get_mut(seg))
}

pub fn watch_pct(node: &Node) -> i32 {
    let duration = node.meta.duration.unwrap_or(0.0);
    if node.kind != Kind::Video || duration <= 0.0 {
        return 0;
    }
    let pct = (node.meta.position.unwrap_or(0.0) / duration * 100.0).round() as i32;
    if pct >= 90 {
        100
    } else {
        pct
    }
}

pub enum ScanMsg {
    // The file currently being probed, for progress display.
    Status(String),
    Entry { parts: Vec<String>, kind: Kind, meta: Meta },
    // The walk finished: the whole tree is browsable (probes continue).
    WalkDone,
    // Probe results for one video, arriving after WalkDone.
    Probed { parts: Vec<String>, result: Option<(f64, BTreeMap<String, String>)> },
    Done,
}

pub fn insert(root: &mut Node, parts: Vec<String>, kind: Kind, meta: Meta) {
    let mut cur = root;
    for part in &parts[..parts.len() - 1] {
        cur = cur.children.entry(part.clone()).or_default();
    }
    let leaf = Node { kind, children: BTreeMap::new(), meta };
    cur.children.insert(parts.last().unwrap().clone(), leaf);
}

// The background scan. Pass 1 walks the media dir and streams entries with
// whatever metadata the sidecars already hold -- the UI becomes browsable as
// soon as that finishes. Pass 2 then handles videos never probed before:
// subliminal downloads subtitles *first*, so the mpv preflight probe that
// follows sees them in its track-list.
pub fn spawn_scan(media_path: String, tx: Sender<ScanMsg>) {
    std::thread::spawn(move || {
        let mut pending: Vec<Vec<String>> = vec![];
        for entry in walkdir::WalkDir::new(&media_path)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.') || e.depth() == 0)
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let kind = match entry.path().extension().and_then(|e| e.to_str()) {
                Some("mp4" | "mkv" | "avi" | "mp3") => Kind::Video,
                Some("rvz" | "wbfs") => Kind::WiiGame,
                Some("sh") => Kind::Script,
                _ => continue,
            };
            let Ok(rel) = entry.path().strip_prefix(&media_path) else { continue };
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();

            let mut meta = Meta::default();
            if kind == Kind::Video {
                meta = load_metadata(&media_path, &parts);
                if meta.duration.is_none() && !meta.probe_failed {
                    pending.push(parts.clone());
                }
            }
            let _ = tx.send(ScanMsg::Entry { parts, kind, meta });
        }
        let _ = tx.send(ScanMsg::WalkDone);

        let mut probe_mpv = None;
        for parts in pending {
            let rel = parts.join("/");
            let full = format!("{media_path}/{rel}");
            let _ = tx.send(ScanMsg::Status(rel));
            let _ = std::process::Command::new("subliminal")
                .args(["download", "-l", "en", "-HI", "-FO", &full])
                .status();
            let result = crate::player::probe(&mut probe_mpv, &full);
            // Re-read the sidecar before writing: the user may have started
            // watching this video (saving a position) while this pass ran.
            let mut meta = load_metadata(&media_path, &parts);
            meta.apply_probe(&result);
            save_metadata(&media_path, &parts, &meta);
            let _ = tx.send(ScanMsg::Probed { parts, result });
        }
        let _ = tx.send(ScanMsg::Done);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_extension_cases() {
        assert_eq!(strip_extension("movie.mp4"), "movie");
        assert_eq!(strip_extension("a.b.c"), "a.b");
        assert_eq!(strip_extension("noext"), "noext");
        assert_eq!(strip_extension(".hidden"), ".hidden");
        assert_eq!(strip_extension("trailing."), "trailing.");
    }

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn tmc_paths_do_not_collide_across_extensions() {
        let a = tmc_path("/m", &path(&["dir", "movie.mp4"]));
        let b = tmc_path("/m", &path(&["dir", "movie.mkv"]));
        assert_eq!(a, "/m/dir/movie.mp4.tmc");
        assert_ne!(a, b);
        // The legacy scheme (extension replaced) is what old files used.
        assert_eq!(legacy_tmc_path("/m", &path(&["dir", "movie.mp4"])), "/m/dir/movie.tmc");
        // A dotted directory name must not confuse the legacy path.
        assert_eq!(legacy_tmc_path("/m", &path(&["Season.1", "ep.mp4"])), "/m/Season.1/ep.tmc");
    }

    fn video(duration: Option<f64>, position: Option<f64>) -> Node {
        Node {
            kind: Kind::Video,
            children: BTreeMap::new(),
            meta: Meta { duration, position, ..Meta::default() },
        }
    }

    #[test]
    fn watch_pct_cases() {
        assert_eq!(watch_pct(&video(None, Some(10.0))), 0);
        assert_eq!(watch_pct(&video(Some(0.0), Some(10.0))), 0); // no div-by-zero
        assert_eq!(watch_pct(&video(Some(200.0), Some(100.0))), 50);
        assert_eq!(watch_pct(&video(Some(100.0), Some(95.0))), 100); // >=90 rounds up
        assert_eq!(watch_pct(&video(Some(100.0), None)), 0);
        assert_eq!(watch_pct(&Node::default()), 0); // directories have no progress
    }

    #[test]
    fn meta_toml_round_trip() {
        // A Lua-era file: flattened track keys, integer duration.
        let meta: Meta = toml::from_str(
            "duration = 120\nposition = 33.5\naid = \"2\"\ntrack_audio_2 = \"en/5.1ch\"\ntrack_sub_1 = \"en\"\n",
        )
        .unwrap();
        assert_eq!(meta.duration, Some(120.0));
        assert_eq!(meta.aid.as_deref(), Some("2"));
        assert!(!meta.probe_failed);
        assert_eq!(meta.track_label(TrackKind::Audio, "2"), Some("en/5.1ch"));
        let out = toml::to_string(&meta).unwrap();
        assert!(out.contains("track_sub_1"));
        assert!(!out.contains("probe_failed")); // false is not serialized
        assert!(!out.contains("sid")); // None is not serialized

        let failed = Meta { probe_failed: true, ..Meta::default() };
        let reparsed: Meta = toml::from_str(&toml::to_string(&failed).unwrap()).unwrap();
        assert!(reparsed.probe_failed);
    }

    #[test]
    fn track_list_sorts_numerically() {
        let mut meta = Meta::default();
        for id in [10, 2, 1] {
            meta.tracks.insert(track_key("audio", &id.to_string()), format!("a{id}"));
        }
        meta.tracks.insert(track_key("sub", "3"), "s3".into());
        let ids: Vec<String> = meta.track_list(TrackKind::Audio).into_iter().map(|(id, _)| id).collect();
        assert_eq!(ids, ["1", "2", "10"]);
        assert_eq!(meta.track_list(TrackKind::Sub).len(), 1);
    }

    #[test]
    fn insert_and_get_node() {
        let mut root = Node::default();
        insert(&mut root, path(&["shows", "a.mp4"]), Kind::Video, Meta::default());
        insert(&mut root, path(&["shows", "b.mp4"]), Kind::Video, Meta::default());
        assert_eq!(get_node(&root, &path(&["shows"])).unwrap().children.len(), 2);
        assert!(get_node(&root, &path(&["shows", "a.mp4"])).is_some());
        assert!(get_node(&root, &path(&["missing"])).is_none());
    }
}
