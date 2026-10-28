// English subtitle download for videos that have none (replaces shelling out
// to subliminal, whose Fedora build only knows dead providers). Episodes
// try Gestdown first (Addic7ed mirror, keyless); everything then falls back
// to OpenSubtitles.com, which needs an API key and allows 5 downloads/day
// per IP anonymously, 20 with a free account. The result is saved as
// <video stem>.en.srt, which mpv loads automatically.

use crate::config::Subtitles as Config;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::time::Duration;
use ureq::Agent;

const GESTDOWN: &str = "https://api.gestdown.info";
const OPENSUBTITLES: &str = "https://api.opensubtitles.com/api/v1";

pub struct Fetcher {
    agent: Agent,
    config: Config,
    // Bearer token after logging in, when an account is configured.
    token: Option<String>,
    // Login failed (wrong password, throttled): stay anonymous this scan.
    login_failed: bool,
    // Out of OpenSubtitles downloads: skip it for the rest of this scan.
    quota_hit: bool,
}

// What a filename says about its video.
#[derive(Debug, PartialEq)]
struct Guess {
    title: String,
    year: Option<u32>,
    episode: Option<(u32, u32)>,
}

impl Fetcher {
    pub fn new(config: Config) -> Fetcher {
        let agent = Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent(concat!("tiny-media-center v", env!("CARGO_PKG_VERSION")))
            .http_status_as_error(false)
            .build()
            .into();
        Fetcher { agent, config, token: None, login_failed: false, quota_hit: false }
    }

    // Ok(true): saved a subtitle; Ok(false): none exists, don't ask again;
    // Err: transient (network, quota, missing API key), retry next scan.
    pub fn fetch(&mut self, video: &Path) -> Result<bool, String> {
        let guess = guess(video);
        let mut transient = None;
        if let Some((season, episode)) = guess.episode {
            match self.gestdown(&guess.title, season, episode, video) {
                Ok(Some(srt)) => return save(video, &srt).map(|()| true),
                Ok(None) => {}
                Err(e) => transient = Some(e),
            }
        }
        match self.opensubtitles(&guess, video) {
            Ok(Some(srt)) => save(video, &srt).map(|()| true),
            Ok(None) => transient.map_or(Ok(false), Err),
            Err(e) => Err(e),
        }
    }

    fn gestdown(&self, title: &str, season: u32, episode: u32, video: &Path) -> Result<Option<Vec<u8>>, String> {
        #[derive(Deserialize)]
        struct Shows {
            shows: Vec<Show>,
        }
        #[derive(Deserialize)]
        struct Show {
            id: String,
            name: String,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Episode {
            matching_subtitles: Vec<Sub>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Sub {
            version: String,
            hearing_impaired: bool,
            completed: bool,
            download_uri: String,
            download_count: u64,
        }

        let shows: Shows = match self.get_json(&format!("{GESTDOWN}/shows/search/{}", encode(title)), &[])? {
            Some(shows) => shows,
            None => return Ok(None),
        };
        let Some(show) = shows.shows.iter().find(|s| normalize(&s.name) == normalize(title)) else {
            return Ok(None);
        };
        let url = format!("{GESTDOWN}/subtitles/get/{}/{season}/{episode}/eng", show.id);
        let Some(found) = self.get_json::<Episode>(&url, &[])? else { return Ok(None) };
        let tokens = release_tokens(video);
        let best = found
            .matching_subtitles
            .iter()
            .filter(|s| s.completed)
            .max_by_key(|s| (!s.hearing_impaired, overlap(&tokens, &s.version), s.download_count));
        let Some(best) = best else { return Ok(None) };
        let mut response = self.agent.get(format!("{GESTDOWN}{}", best.download_uri)).call().map_err(|e| e.to_string())?;
        // Gestdown answers an exhausted Addic7ed quota with an HTML page.
        let is_html = response.headers().get("content-type").is_some_and(|v| v.as_bytes().starts_with(b"text/html"));
        if response.status() != 200 || is_html {
            return Err(format!("gestdown download: HTTP {}", response.status()));
        }
        response.body_mut().read_to_vec().map(Some).map_err(|e| e.to_string())
    }

    fn opensubtitles(&mut self, guess: &Guess, video: &Path) -> Result<Option<Vec<u8>>, String> {
        #[derive(Deserialize)]
        struct Results {
            data: Vec<Item>,
        }
        #[derive(Deserialize)]
        struct Item {
            attributes: Attributes,
        }
        #[derive(Deserialize)]
        struct Attributes {
            #[serde(default)]
            release: String,
            #[serde(default)]
            hearing_impaired: bool,
            #[serde(default)]
            foreign_parts_only: bool,
            #[serde(default)]
            moviehash_match: bool,
            #[serde(default)]
            download_count: u64,
            files: Vec<FileRef>,
            feature_details: Option<Feature>,
        }
        #[derive(Deserialize)]
        struct Feature {
            #[serde(default)]
            title: String,
            #[serde(default)]
            parent_title: String,
            year: Option<u32>,
        }
        #[derive(Deserialize)]
        struct FileRef {
            file_id: u64,
        }
        #[derive(Serialize)]
        struct DownloadRequest {
            file_id: u64,
            sub_format: &'static str,
        }
        #[derive(Deserialize)]
        struct Download {
            link: Option<String>,
            remaining: Option<i64>,
        }

        if self.config.opensubtitles_api_key.is_empty() {
            return Err("opensubtitles: no API key configured".into());
        }
        if self.quota_hit {
            return Err("opensubtitles: download quota used up".into());
        }
        if !self.login_failed && self.token.is_none() && !self.config.opensubtitles_username.is_empty() {
            if let Err(e) = self.login() {
                eprintln!("tiny-media-center: {e}; continuing without an account");
                self.login_failed = true;
            }
        }

        // Hash first (exact release match), then by name. Parameters lowercase
        // and alphabetical, as the API redirects otherwise.
        let mut searches = vec![];
        if let Some(hash) = moviehash(video) {
            searches.push(vec![("languages", "en".to_string()), ("moviehash", hash)]);
        }
        let mut by_name = vec![("languages", "en".to_string()), ("query", guess.title.to_lowercase())];
        match guess.episode {
            Some((season, episode)) => {
                by_name.insert(0, ("episode_number", episode.to_string()));
                by_name.push(("season_number", season.to_string()));
            }
            None => by_name.extend(guess.year.map(|y| ("year", y.to_string()))),
        }
        searches.push(by_name);

        let tokens = release_tokens(video);
        let mut candidates = vec![];
        for params in searches {
            let mut params = params;
            params.push(("machine_translated", "exclude".into()));
            params.push(("ai_translated", "exclude".into()));
            params.sort();
            let Some(results) = self.get_json::<Results>(&format!("{OPENSUBTITLES}/subtitles"), &params)? else {
                continue;
            };
            // Name searches are fuzzy ("house" finds Full House too): keep
            // results for this title (and year), unless the hash matched.
            let wanted = normalize(&guess.title);
            let is_this = |a: &Attributes| {
                let Some(f) = &a.feature_details else { return false };
                let title = if guess.episode.is_some() { &f.parent_title } else { &f.title };
                normalize(title) == wanted && (guess.episode.is_some() || guess.year.is_none() || f.year == guess.year)
            };
            candidates.extend(
                results.data.into_iter().map(|i| i.attributes).filter(|a| !a.foreign_parts_only && (a.moviehash_match || is_this(a))),
            );
            if !candidates.is_empty() {
                break;
            }
        }
        let best = candidates.iter().max_by_key(|a| {
            (a.moviehash_match, !a.hearing_impaired, overlap(&tokens, &a.release), a.download_count)
        });
        let Some(file_id) = best.and_then(|a| a.files.first()).map(|f| f.file_id) else { return Ok(None) };

        let mut request = self.agent.post(format!("{OPENSUBTITLES}/download")).header("Api-Key", &self.config.opensubtitles_api_key);
        if let Some(token) = &self.token {
            request = request.header("Authorization", &format!("Bearer {token}"));
        }
        let mut response =
            request.send_json(DownloadRequest { file_id, sub_format: "srt" }).map_err(|e| e.to_string())?;
        // 406/407: out of downloads for today.
        if matches!(response.status().as_u16(), 406 | 407) {
            self.quota_hit = true;
            return Err("opensubtitles: download quota used up".into());
        }
        if response.status() != 200 {
            return Err(format!("opensubtitles download: HTTP {}", response.status()));
        }
        let download: Download = response.body_mut().read_json().map_err(|e| e.to_string())?;
        if download.remaining.is_some_and(|r| r <= 0) {
            self.quota_hit = true;
        }
        let Some(link) = download.link else { return Err("opensubtitles: no download link".into()) };
        let mut response = self.agent.get(&link).call().map_err(|e| e.to_string())?;
        if response.status() != 200 {
            return Err(format!("opensubtitles file: HTTP {}", response.status()));
        }
        response.body_mut().read_to_vec().map(Some).map_err(|e| e.to_string())
    }

    // Log in once per scan, when an account is configured (raises the daily
    // download quota from 5 to 20).
    fn login(&mut self) -> Result<(), String> {
        #[derive(Serialize)]
        struct Login<'a> {
            username: &'a str,
            password: &'a str,
        }
        #[derive(Deserialize)]
        struct Token {
            token: String,
        }
        let mut response = self
            .agent
            .post(format!("{OPENSUBTITLES}/login"))
            .header("Api-Key", &self.config.opensubtitles_api_key)
            .send_json(Login { username: &self.config.opensubtitles_username, password: &self.config.opensubtitles_password })
            .map_err(|e| e.to_string())?;
        if response.status() != 200 {
            return Err(format!("opensubtitles login: HTTP {}", response.status()));
        }
        self.token = Some(response.body_mut().read_json::<Token>().map_err(|e| e.to_string())?.token);
        Ok(())
    }

    // GET + JSON; Ok(None) on 404 (nothing there), Err on anything else.
    fn get_json<T: serde::de::DeserializeOwned>(&self, url: &str, params: &[(&str, String)]) -> Result<Option<T>, String> {
        let mut request = self.agent.get(url).header("Accept", "application/json");
        if url.starts_with(OPENSUBTITLES) {
            request = request.header("Api-Key", &self.config.opensubtitles_api_key);
        }
        for (key, value) in params {
            request = request.query(*key, value);
        }
        let mut response = request.call().map_err(|e| e.to_string())?;
        match response.status().as_u16() {
            200 => response.body_mut().read_json().map(Some).map_err(|e| format!("{url}: {e}")),
            404 => Ok(None),
            status => Err(format!("{url}: HTTP {status}")),
        }
    }
}

// Saved byte for byte: mpv detects the charset itself (uchardet). Temp file
// + rename, so a crash can't leave a truncated subtitle that the next probe
// would accept as done.
fn save(video: &Path, srt: &[u8]) -> Result<(), String> {
    let path = video.with_extension("en.srt");
    let tmp = video.with_extension("en.srt.tmp");
    std::fs::write(&tmp, srt).and_then(|()| std::fs::rename(&tmp, &path)).map_err(|e| e.to_string())
}

// OpenSubtitles hash: file size plus the 64-bit little-endian words of the
// first and last 64 KiB, wrapping.
fn moviehash(path: &Path) -> Option<String> {
    const CHUNK: u64 = 65536;
    let mut file = File::open(path).ok()?;
    let size = file.metadata().ok()?.len();
    if size < 2 * CHUNK {
        return None;
    }
    let mut hash = size;
    for offset in [0, size - CHUNK] {
        let mut buf = vec![0u8; CHUNK as usize];
        file.seek(SeekFrom::Start(offset)).ok()?;
        file.read_exact(&mut buf).ok()?;
        for word in buf.chunks_exact(8) {
            hash = hash.wrapping_add(u64::from_le_bytes(word.try_into().unwrap()));
        }
    }
    Some(format!("{hash:016x}"))
}

// Title, year and SxxEyy from the file name; an episode file named only
// "S01E02 - Title.mkv" takes the show title from its folder.
// Simplification: no release-name parser (guessit), just the common shapes
// in this library: "Show.Name.S02E05", "Show (2004) S01E01 ...",
// "S01E01 - Episode", "Movie (1999)", "Movie.1999.1080p...".
fn guess(path: &Path) -> Guess {
    let stem = path.file_stem().unwrap_or_default().to_string_lossy().replace(['.', '_'], " ");
    let lower = stem.to_lowercase();
    let bytes = lower.as_bytes();
    let episode_at = (0..bytes.len()).find_map(|i| {
        let rest = &lower[i..];
        let digits = |s: &str| s.chars().take_while(char::is_ascii_digit).collect::<String>();
        let season = digits(rest.strip_prefix('s')?);
        let after = rest.get(1 + season.len()..)?.strip_prefix('e')?;
        let episode = digits(after);
        let boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if !boundary || season.len() > 2 {
            return None;
        }
        Some((i, (season.parse().ok()?, episode.parse().ok()?)))
    });
    let head = match episode_at {
        Some((i, _)) => &stem[..i],
        None => &stem[..],
    };
    // The last year-like number, so "Blade Runner 2049 (2017)" keeps its title.
    let year_at = head.char_indices().rfind(|&(i, _)| {
        let year = head.get(i..i + 4).and_then(|y| y.parse::<u32>().ok());
        let before_ok = i == 0 || !head.as_bytes()[i - 1].is_ascii_digit();
        let after_ok = head.as_bytes().get(i + 4).is_none_or(|b| !b.is_ascii_digit());
        i > 0 && before_ok && after_ok && year.is_some_and(|y| (1900..=2099).contains(&y))
    });
    let year = year_at.and_then(|(i, _)| head[i..i + 4].parse().ok());
    let title = year_at.map_or(head, |(i, _)| &head[..i]);
    let mut title = title.trim_matches(|c: char| c.is_whitespace() || "-([".contains(c)).to_string();
    if title.is_empty() {
        let folder = path.parent().and_then(Path::file_name).unwrap_or_default();
        title = folder.to_string_lossy().to_string();
    }
    Guess { title, year, episode: episode_at.map(|(_, e)| e) }
}

// Lowercase alphanumeric words, for comparing names.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

fn normalize(name: &str) -> String {
    words(name).join(" ")
}

fn release_tokens(video: &Path) -> Vec<String> {
    words(&video.file_name().unwrap_or_default().to_string_lossy())
}

// How many of a subtitle's release words appear in the video's file name.
fn overlap(tokens: &[String], release: &str) -> usize {
    words(release).iter().filter(|w| tokens.contains(w)).count()
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(path: &str) -> Guess {
        guess(Path::new(path))
    }

    #[test]
    fn guesses_library_names() {
        let ep = |title: &str, s, e| Guess { title: title.into(), year: None, episode: Some((s, e)) };
        assert_eq!(g("/m/Cyberpunk Edgerunners/Cyberpunk.Edgerunners.S02E05.mkv"), ep("Cyberpunk Edgerunners", 2, 5));
        assert_eq!(g("/m/Mr. Robot/S01E02 - eps1.1_ones-and-zer0es.mpeg.mkv"), ep("Mr. Robot", 1, 2));
        assert_eq!(
            g("/m/House MD/House (2004) S01E01 Pilot (1080p x265 10bit Joy).mkv"),
            Guess { title: "House".into(), year: Some(2004), episode: Some((1, 1)) }
        );
        assert_eq!(g("/m/Movies/Weapons (2025).mkv"), Guess { title: "Weapons".into(), year: Some(2025), episode: None });
        assert_eq!(
            g("/m/Movies/The.Matrix.1999.1080p.BluRay.x264.mkv"),
            Guess { title: "The Matrix".into(), year: Some(1999), episode: None }
        );
        // A title that is a year-like number isn't taken as the year.
        assert_eq!(g("/m/Movies/1917 (2019).mkv"), Guess { title: "1917".into(), year: Some(2019), episode: None });
        assert_eq!(g("/m/Movies/Heat.mkv"), Guess { title: "Heat".into(), year: None, episode: None });
        assert_eq!(
            g("/m/Movies/Blade Runner 2049 (2017).mkv"),
            Guess { title: "Blade Runner 2049".into(), year: Some(2017), episode: None }
        );
    }

    #[test]
    fn moviehash_matches_reference_algorithm() {
        let dir = std::env::temp_dir().join(format!("tmc-hash-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v.bin");
        // 3 chunks: first chunk all 1s, middle 0s, last all 2s (as bytes).
        let mut data = vec![1u8; 65536];
        data.extend(vec![0u8; 65536]);
        data.extend(vec![2u8; 65536]);
        std::fs::write(&path, &data).unwrap();
        let word = |b: u8| u64::from_le_bytes([b; 8]);
        let expected = (data.len() as u64)
            .wrapping_add(word(1).wrapping_mul(8192))
            .wrapping_add(word(2).wrapping_mul(8192));
        assert_eq!(moviehash(&path), Some(format!("{expected:016x}")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn release_overlap_counts_shared_words() {
        let tokens = release_tokens(Path::new("Mr.Robot.S01E01.PROPER.720p.HDTV.x264-LOL.mkv"));
        assert_eq!(overlap(&tokens, "PROPER.LOL"), 2);
        assert_eq!(overlap(&tokens, "WEB-DL-NTb"), 0);
    }
}
