// Port of the media-tree half of main.lua: the Node tree, .tmc metadata
// sidecars, watch percentage, and the background scan thread
// (find + preflight + subliminal in the Lua version).

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::mpsc::Sender;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Directory,
    Video,
    WiiGame,
    Script,
}

// .tmc sidecar contents. Kept byte-compatible with the files written by the
// Lua version: duration/position are TOML numbers, aid/sid are strings, and
// track labels live in flattened `track_<type>_<id>` string keys.
#[derive(Serialize, Deserialize, Default, Clone)]
pub struct Meta {
    #[serde(default, deserialize_with = "num", skip_serializing_if = "Option::is_none")]
    pub duration: Option<f64>,
    #[serde(default, deserialize_with = "num", skip_serializing_if = "Option::is_none")]
    pub position: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sid: Option<String>,
    #[serde(flatten)]
    pub tracks: BTreeMap<String, String>,
}

// Accept both TOML integers and floats for duration/position.
fn num<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum N {
        F(f64),
        I(i64),
    }
    Ok(Option::<N>::deserialize(d)?.map(|n| match n {
        N::F(f) => f,
        N::I(i) => i as f64,
    }))
}

pub struct Node {
    pub name: String,
    pub path: Vec<String>,
    pub kind: Kind,
    pub children: BTreeMap<String, Node>,
    pub meta: Meta,
}

impl Node {
    pub fn root() -> Node {
        Node {
            name: String::new(),
            path: vec![],
            kind: Kind::Directory,
            children: BTreeMap::new(),
            meta: Meta::default(),
        }
    }
}

pub fn strip_extension(filename: &str) -> &str {
    match filename.rfind('.') {
        Some(i) if i > 0 && i + 1 < filename.len() => &filename[..i],
        _ => filename,
    }
}

fn tmc_path(media_path: &str, node_path: &[String]) -> String {
    let rel = node_path.join("/");
    format!("{}/{}.tmc", media_path, strip_extension(&rel))
}

pub fn save_metadata(media_path: &str, node: &Node) {
    if let Ok(s) = toml::to_string(&node.meta) {
        let _ = std::fs::write(tmc_path(media_path, &node.path), s);
    }
}

pub fn get_node<'a>(root: &'a Node, path: &[String]) -> Option<&'a Node> {
    let mut node = root;
    for seg in path {
        if node.kind != Kind::Directory {
            return None;
        }
        node = node.children.get(seg)?;
    }
    Some(node)
}

pub fn get_node_mut<'a>(root: &'a mut Node, path: &[String]) -> Option<&'a mut Node> {
    let mut node = root;
    for seg in path {
        if node.kind != Kind::Directory {
            return None;
        }
        node = node.children.get_mut(seg)?;
    }
    Some(node)
}

// Port of getVideoContext: walks the path and returns the first video node on
// it (the remainder being a virtual segment like ":audio").
pub fn video_context<'a>(root: &'a Node, path: &[String]) -> Option<&'a Node> {
    for i in 0..path.len() {
        if path[i].starts_with(':') {
            return get_node(root, &path[..i]);
        }
        let node = get_node(root, &path[..=i])?;
        if node.kind == Kind::Video {
            return Some(node);
        }
    }
    None
}

pub fn watch_pct(node: &Node) -> i32 {
    if node.kind != Kind::Video {
        return 0;
    }
    let Some(duration) = node.meta.duration else { return 0 };
    let pct = ((node.meta.position.unwrap_or(0.0) / duration) * 100.0 + 0.5).floor() as i32;
    if pct >= 90 {
        100
    } else {
        pct
    }
}

pub enum ScanMsg {
    Status(String),
    Entry { parts: Vec<String>, kind: Kind, meta: Meta },
    Done,
}

fn kind_by_extension(name: &str) -> Option<Kind> {
    match name.rsplit_once('.')?.1 {
        "mp4" | "mkv" | "avi" | "mp3" => Some(Kind::Video),
        "rvz" | "wbfs" => Some(Kind::WiiGame),
        "sh" => Some(Kind::Script),
        _ => None,
    }
}

// Port of buildMediaTree's worker thread: walk the media dir, and for videos
// without known duration run an mpv preflight probe (embedded now, not a
// spawned process) and a subliminal subtitle download.
pub fn spawn_scan(media_path: String, tx: Sender<ScanMsg>) {
    std::thread::spawn(move || {
        let mut probe_mpv = None;
        for entry in walkdir::WalkDir::new(&media_path)
            .into_iter()
            .filter_entry(|e| !e.file_name().to_string_lossy().starts_with('.') || e.depth() == 0)
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_file() {
                continue;
            }
            let Ok(rel) = entry.path().strip_prefix(&media_path) else { continue };
            let parts: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            let Some(kind) = kind_by_extension(&parts[parts.len() - 1]) else { continue };

            let mut meta = Meta::default();
            if kind == Kind::Video {
                let tmc = tmc_path(&media_path, &parts);
                meta = std::fs::read_to_string(&tmc)
                    .ok()
                    .and_then(|s| toml::from_str(&s).ok())
                    .unwrap_or_default();
                if meta.duration.is_none() {
                    let rel_str = parts.join("/");
                    let full = format!("{media_path}/{rel_str}");
                    let _ = tx.send(ScanMsg::Status(rel_str));
                    if let Some((duration, tracks)) = crate::player::probe(&mut probe_mpv, &full) {
                        meta.duration = Some(duration);
                        meta.tracks = tracks;
                        if let Ok(s) = toml::to_string(&meta) {
                            let _ = std::fs::write(&tmc, s);
                        }
                    }
                    let _ = std::process::Command::new("subliminal")
                        .args(["download", "-l", "en", "-HI", "-FO", &full])
                        .status();
                }
            }

            let _ = tx.send(ScanMsg::Entry { parts, kind, meta });
        }
        let _ = tx.send(ScanMsg::Done);
    });
}

// Port of processScanResults' tree assembly.
pub fn insert(root: &mut Node, parts: Vec<String>, kind: Kind, meta: Meta) {
    let mut cur = root;
    for i in 0..parts.len() - 1 {
        let path: Vec<String> = parts[..=i].to_vec();
        cur = cur
            .children
            .entry(parts[i].clone())
            .or_insert_with(|| Node {
                name: parts[i].clone(),
                path,
                kind: Kind::Directory,
                children: BTreeMap::new(),
                meta: Meta::default(),
            });
    }
    let name = parts[parts.len() - 1].clone();
    cur.children.insert(
        name.clone(),
        Node { name, path: parts, kind, children: BTreeMap::new(), meta },
    );
}

// The Lua version launched Dolphin through the systemd user manager; see the
// comment there (kept in the PR description). Preserved as-is.
pub fn external_command(media_path: &str, node_path: &[String], kind: Kind) -> String {
    let full = format!("{}/{}", media_path, node_path.join("/"));
    match kind {
        Kind::WiiGame => format!(
            r#"g="{full}"; if command -v systemd-run >/dev/null; then systemd-run --user --wait --collect --same-dir dolphin-emu --batch -C Dolphin.Display.Fullscreen=True -C Dolphin.Interface.ConfirmStop=False --exec="$g"; else dolphin-emu --batch -C Dolphin.Display.Fullscreen=True -C Dolphin.Interface.ConfirmStop=False --exec="$g"; fi"#
        ),
        _ => format!(r#"bash "{full}""#),
    }
}

pub fn media_file(media_path: &str, node_path: &[String]) -> String {
    format!("{}/{}", media_path, node_path.join("/"))
}
