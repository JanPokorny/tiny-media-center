// The media tree: the Node tree, .tmc metadata sidecars, watch percentage,
// and the background scan thread (walk + mpv preflight probe).

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

// .tmc sidecar contents. Existing files must stay readable: duration/position are TOML numbers, aid/sid are strings, and
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
    // Negative cache: the preflight probe failed, don't retry on every launch. Delete the .tmc to force a retry.
    #[serde(default, skip_serializing_if = "is_false")]
    pub probe_failed: bool,
    // English subtitles are settled: the file has them, one was downloaded,
    // or no provider has any. Unset after transient failures (offline, out
    // of quota), so the next scan retries.
    #[serde(default, skip_serializing_if = "is_false")]
    pub subs_done: bool,
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

    // The user's pick, else the default: the best English track, else the
    // first audio track / subtitles off. None until the probe has listed the
    // tracks (mpv then picks). Passed to mpv explicitly, so the menu always
    // shows what plays.
    pub fn effective_selection(&self, kind: TrackKind) -> Option<String> {
        if let Some(id) = self.selection(kind) {
            return Some(id.to_string());
        }
        let tracks = self.track_list(kind);
        // Dubtitles transcribe the English dub, so they only fit English audio.
        let english_audio = kind == TrackKind::Sub
            && self
                .effective_selection(TrackKind::Audio)
                .and_then(|id| self.track_label(TrackKind::Audio, &id).map(is_english))
                .unwrap_or(false);
        let english = tracks
            .iter()
            .filter(|(_, label)| is_english(label) && track_rank(label, english_audio).is_some())
            .min_by_key(|(_, label)| track_rank(label, english_audio));
        match (english, tracks.first()) {
            (Some((id, _)), _) => Some(id.clone()),
            (None, None) => None,
            (None, Some((id, _))) if kind == TrackKind::Audio => Some(id.clone()),
            (None, Some(_)) => Some(String::new()),
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
            // A file that probed fine before keeps its data through a
            // transient failure (slow disk, timeout) instead of being
            // written off for good.
            None if self.duration.is_none() => self.probe_failed = true,
            None => {}
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

// Labels start with the track's language code ("en", "eng", "en-US").
fn is_english(label: &str) -> bool {
    let lang = label.split('/').next().unwrap_or("").to_lowercase();
    lang == "eng" || lang == "en" || lang.starts_with("en-")
}

// Lower is better: dubtitles (when the audio is the English dub they
// transcribe), then plain, then SDH (or dubtitles under other audio), then
// forced-only / signs & songs.
// Commentary and audio description are never a default (None). Judged by
// words in the label after the language.
fn track_rank(label: &str, english_audio: bool) -> Option<u8> {
    let rest = label.split_once('/').map_or("", |(_, rest)| rest).to_lowercase();
    let has = |words: &[&str]| rest.split(|c: char| !c.is_alphanumeric()).any(|w| words.contains(&w));
    if has(&["commentary", "description", "descriptive"]) {
        None
    } else if has(&["dubtitle", "dubtitles"]) {
        Some(if english_audio { 0 } else { 2 })
    } else if has(&["forced", "signs", "songs"]) {
        Some(3)
    } else if has(&["sdh", "cc", "hi"]) {
        Some(2)
    } else {
        Some(1)
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

const VIDEO_EXTS: &[&str] =
    &["mkv", "mp4", "m4v", "avi", "mov", "wmv", "webm", "mpg", "mpeg", "ts", "m2ts", "mts", "flv", "ogv", "3gp", "vob"];
const AUDIO_EXTS: &[&str] =
    &["mp3", "flac", "ogg", "oga", "opus", "m4a", "aac", "wav", "wma", "alac", "aiff", "ape", "mka"];

fn has_ext(name: &str, exts: &[&str]) -> bool {
    let ext = std::path::Path::new(name).extension().and_then(|e| e.to_str()).unwrap_or("");
    exts.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

pub fn is_audio(name: &str) -> bool {
    has_ext(name, AUDIO_EXTS)
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

pub fn load_metadata(media_path: &str, node_path: &[String]) -> Meta {
    let path = tmc_path(media_path, node_path);
    let Ok(s) = std::fs::read_to_string(&path) else { return Meta::default() };
    toml::from_str(&s).unwrap_or_else(|e| {
        eprintln!("tiny-media-center: ignoring unparsable metadata {path}: {e}");
        Meta::default()
    })
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
    Probed { parts: Vec<String>, result: Option<(f64, BTreeMap<String, String>)>, subs_done: bool },
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
// soon as that finishes. Pass 2 then handles videos never probed, or without
// settled English subtitles: the mpv preflight probe lists the tracks, and
// if there's no English subtitle among them one is downloaded and the file
// re-probed so the new track shows up.
pub fn spawn_scan(media_path: String, subtitles: crate::config::Subtitles, tx: Sender<ScanMsg>) {
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
            let name = entry.file_name().to_string_lossy();
            let kind = match entry.path().extension().and_then(|e| e.to_str()) {
                Some("rvz" | "wbfs") => Kind::WiiGame,
                Some("sh") => Kind::Script,
                _ if is_audio(&name) || has_ext(&name, VIDEO_EXTS) => Kind::Video,
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
                if !meta.probe_failed && (meta.duration.is_none() || !meta.subs_done) {
                    pending.push(parts.clone());
                }
            }
            let _ = tx.send(ScanMsg::Entry { parts, kind, meta });
        }
        let _ = tx.send(ScanMsg::WalkDone);

        let mut probe_mpv = None;
        let mut fetcher = crate::subtitles::Fetcher::new(subtitles);
        for parts in pending {
            let rel = parts.join("/");
            let full = format!("{media_path}/{rel}");
            let _ = tx.send(ScanMsg::Status(rel.clone()));
            let mut result = crate::player::probe(&mut probe_mpv, &full);
            let mut subs_done = false;
            if let Some((_, tracks)) = &result {
                // Forced-only / signs & songs tracks don't count: they leave
                // most of the dialogue untranslated.
                let has_english = tracks.iter().any(|(key, label)| {
                    key.starts_with("track_sub_") && is_english(label) && track_rank(label, false).is_some_and(|r| r < 3)
                });
                // Audio files are in the video list too, but have nothing to subtitle.
                subs_done = has_english || is_audio(&rel);
                if !subs_done {
                    match fetcher.fetch(std::path::Path::new(&full)) {
                        Ok(downloaded) => {
                            subs_done = true;
                            if downloaded {
                                result = crate::player::probe(&mut probe_mpv, &full);
                            }
                        }
                        Err(e) => eprintln!("tiny-media-center: subtitles for {rel}: {e}"),
                    }
                }
            }
            // Re-read the sidecar before writing: the user may have started
            // watching this video (saving a position) while this pass ran.
            let mut meta = load_metadata(&media_path, &parts);
            meta.apply_probe(&result);
            meta.subs_done = subs_done;
            save_metadata(&media_path, &parts, &meta);
            let _ = tx.send(ScanMsg::Probed { parts, result, subs_done });
        }
        let _ = tx.send(ScanMsg::Done);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn path(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn extension_classification() {
        assert!(is_audio("a.flac") && is_audio("A.MP3") && is_audio("x.y.opus"));
        assert!(!is_audio("a.mkv") && !is_audio("mp3") && !is_audio("a.flac.tmc"));
        assert!(has_ext("a.MKV", VIDEO_EXTS) && has_ext("a.ts", VIDEO_EXTS) && !has_ext("a.txt", VIDEO_EXTS));
    }

    #[test]
    fn tmc_paths_do_not_collide_across_extensions() {
        let a = tmc_path("/m", &path(&["dir", "movie.mp4"]));
        let b = tmc_path("/m", &path(&["dir", "movie.mkv"]));
        assert_eq!(a, "/m/dir/movie.mp4.tmc");
        assert_ne!(a, b);
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
    fn effective_selection_prefers_english_and_user_pick() {
        let mut meta = Meta::default();
        for (key, label) in [
            ("track_audio_1", "jpn/2ch"),
            ("track_audio_2", "eng/Commentary/2ch"),
            ("track_audio_3", "eng/6ch"),
            ("track_sub_1", "eng/Forced"),
            ("track_sub_2", "eng/SDH"),
            ("track_sub_3", "en"),
            ("track_sub_4", "eng/Dubtitles"),
            ("track_sub_5", "jpn"),
        ] {
            meta.tracks.insert(key.into(), label.into());
        }
        assert_eq!(meta.effective_selection(TrackKind::Audio).as_deref(), Some("3"));
        assert_eq!(meta.effective_selection(TrackKind::Sub).as_deref(), Some("4"));
        meta.tracks.remove("track_sub_4");
        assert_eq!(meta.effective_selection(TrackKind::Sub).as_deref(), Some("3")); // plain over SDH/forced
        meta.tracks.remove("track_sub_3");
        assert_eq!(meta.effective_selection(TrackKind::Sub).as_deref(), Some("2")); // SDH over forced
        meta.sid = Some("5".into());
        assert_eq!(meta.effective_selection(TrackKind::Sub).as_deref(), Some("5")); // user pick wins

        let mut foreign = Meta::default();
        foreign.tracks.insert("track_audio_2".into(), "jpn".into());
        foreign.tracks.insert("track_audio_1".into(), "fra".into());
        foreign.tracks.insert("track_sub_1".into(), "jpn".into());
        assert_eq!(foreign.effective_selection(TrackKind::Audio).as_deref(), Some("1")); // first track
        assert_eq!(foreign.effective_selection(TrackKind::Sub).as_deref(), Some("")); // subs off
        assert_eq!(Meta::default().effective_selection(TrackKind::Audio), None); // not probed yet

        let mut anime = Meta::default();
        for (key, label) in [
            ("track_audio_1", "jpn/6ch"),
            ("track_audio_2", "eng/Commentary/2ch"),
            ("track_sub_1", "eng/Signs & Songs"),
            ("track_sub_2", "eng/Dubtitle"),
            ("track_sub_3", "eng/Full"),
        ] {
            anime.tracks.insert(key.into(), label.into());
        }
        // Commentary is never the default audio; with Japanese audio full
        // subs beat the dubtitle, which beats signs & songs.
        assert_eq!(anime.effective_selection(TrackKind::Audio).as_deref(), Some("1"));
        assert_eq!(anime.effective_selection(TrackKind::Sub).as_deref(), Some("3"));
        anime.tracks.remove("track_sub_3");
        assert_eq!(anime.effective_selection(TrackKind::Sub).as_deref(), Some("2"));
        anime.aid = Some("2".into());
        anime.tracks.insert("track_audio_2".into(), "eng/2ch".into());
        anime.tracks.insert("track_sub_3".into(), "eng/Full".into());
        assert_eq!(anime.effective_selection(TrackKind::Sub).as_deref(), Some("2")); // English dub: dubtitle
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
}
