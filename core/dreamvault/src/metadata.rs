//! Metadata Engine. Plugin-based by design; Phase 1 ships a Local provider.
//!
//! Online providers (ScreenScraper, IGDB, MobyGames) require user-supplied API
//! keys and have rate limits / caching obligations — they slot in behind the
//! same `MetadataProvider` trait in a later milestone. Keys stay user-owned.

use crate::config::ScreenScraperCredentials;
use crate::error::{EngineError, Result};
use crate::models::Game;
use crate::Engine;
use async_trait::async_trait;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Semaphore;

/// A partial set of metadata fields to merge onto a game. `None` fields are
/// left untouched, so providers never clobber better data with blanks.
#[derive(Debug, Clone, Default)]
pub struct MetadataPatch {
    pub cover_art: Option<String>,
    pub background_art: Option<String>,
    pub description: Option<String>,
    pub genre: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,
}

impl MetadataPatch {
    fn is_empty(&self) -> bool {
        self.cover_art.is_none()
            && self.background_art.is_none()
            && self.description.is_none()
            && self.genre.is_none()
            && self.developer.is_none()
            && self.publisher.is_none()
            && self.release_date.is_none()
    }
}

/// A single cover suggestion for the interactive "Find" picker, tagged by the
/// source it came from. `token` is what `apply_cover` needs to download it:
/// the libretro box-art name, or the LaunchBox image file path.
#[derive(Debug, Clone, Serialize)]
pub struct CoverCandidate {
    pub source: String, // "libretro" | "launchbox"
    pub label: String,  // title to display
    pub token: String,  // libretro name, or launchbox file_name
}

/// Merge ranked libretro names and ranked LaunchBox (name, file_name) pairs into
/// one tagged list, libretro first (primary source).
pub(crate) fn rank_cover_candidates(
    libretro: Vec<String>,
    launchbox: Vec<(String, String)>,
) -> Vec<CoverCandidate> {
    let mut out: Vec<CoverCandidate> = libretro
        .into_iter()
        .map(|name| CoverCandidate {
            source: "libretro".into(),
            label: name.clone(),
            token: name,
        })
        .collect();
    out.extend(launchbox.into_iter().map(|(name, file_name)| CoverCandidate {
        source: "launchbox".into(),
        label: name,
        token: file_name,
    }));
    out
}

#[async_trait]
pub trait MetadataProvider: Send + Sync {
    fn id(&self) -> &'static str;
    async fn fetch(&self, game: &Game) -> Result<MetadataPatch>;
}

/// Local provider: discovers artwork the user already has on disk next to the
/// ROM. No network, no API key, always available.
///
/// Looks for `<stem>.png|jpg|jpeg` beside the ROM and inside sibling
/// `media/`, `covers/`, `artwork/`, `boxart/` folders.
pub struct LocalProvider;

#[async_trait]
impl MetadataProvider for LocalProvider {
    fn id(&self) -> &'static str {
        "local"
    }

    async fn fetch(&self, game: &Game) -> Result<MetadataPatch> {
        Ok(local_cover_art(game))
    }
}

/// Locate cover art already sitting next to a ROM. Pure synchronous filesystem
/// probing (no network, no async) so it can be batched on a blocking thread —
/// each call does up to ~two dozen `is_file()` stats, which add up on a slow
/// external drive.
fn local_cover_art(game: &Game) -> MetadataPatch {
    // PS3 disc dumps ship their own art: PS3_GAME/ICON0.PNG (the XMB icon)
    // and PIC1.PNG (the background). libretro's PS3 set covers only ~70
    // titles, so the on-disc art is both better and always present.
    if game.platform == "ps3" {
        if let Some(patch) = ps3_disc_art(&game.rom_path) {
            return patch;
        }
    }

    let rom = std::path::Path::new(&game.rom_path);
    let stem = rom.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    let dir = rom.parent().unwrap_or_else(|| std::path::Path::new("."));

    let exts = ["png", "jpg", "jpeg", "webp"];
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    for ext in exts {
        candidates.push(dir.join(format!("{stem}.{ext}")));
    }
    for sub in ["media", "covers", "artwork", "boxart", "images"] {
        for ext in exts {
            candidates.push(dir.join(sub).join(format!("{stem}.{ext}")));
        }
    }

    let mut patch = MetadataPatch::default();
    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        patch.cover_art = Some(found.to_string_lossy().to_string());
    }
    patch
}

/// Locate a PS3 disc dump's on-disc art from its `rom_path`. The scanner stores
/// either `<root>/PS3_GAME/USRDIR/EBOOT.BIN` (boots the decrypted EBOOT) or the
/// disc root itself; both resolve to the same `PS3_GAME` directory. Returns a
/// patch with `cover_art` = ICON0.PNG and `background_art` = PIC1.PNG when they
/// exist (the canonical uppercase names every disc dump uses).
fn ps3_disc_art(rom_path: &str) -> Option<MetadataPatch> {
    let rom = std::path::Path::new(rom_path);
    let ps3_game = if rom
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n.eq_ignore_ascii_case("EBOOT.BIN"))
    {
        // .../PS3_GAME/USRDIR/EBOOT.BIN -> .../PS3_GAME
        rom.parent()?.parent()?.to_path_buf()
    } else {
        // Disc root -> .../PS3_GAME
        rom.join("PS3_GAME")
    };

    let find = |name: &str| -> Option<String> {
        let p = ps3_game.join(name);
        p.is_file().then(|| p.to_string_lossy().to_string())
    };
    let cover = find("ICON0.PNG");
    let background = find("PIC1.PNG");
    if cover.is_none() && background.is_none() {
        return None;
    }
    Some(MetadataPatch {
        cover_art: cover,
        background_art: background,
        ..Default::default()
    })
}

/// libretro thumbnail server: a no-auth, community-maintained static archive of
/// box art, title screens, and in-game snaps, keyed by the No-Intro / Redump
/// game name. We pull *Named_Boxarts* only. This is the open-source box-art
/// source; it needs no API key, but we still cache every download to disk so a
/// re-run is offline and we are polite to the server.
///
/// Matching is best-effort. libretro names files after the No-Intro DB, e.g.
/// "Mario Kart 64 (USA)", whereas a user's ROMs may use GoodTools naming like
/// "Mario Kart 64 (E) (V1.1) [!]" — an exact filename match would miss almost
/// everything. So we probe a small ordered list of candidates: the cleaned
/// title paired with each likely region word (region detected from the on-disk
/// name, then the usual fallbacks), then the raw on-disk name, then the bare
/// title. The first candidate that returns 200 wins; a total miss leaves the
/// cover blank.
pub struct LibretroThumbnailProvider {
    client: reqwest::Client,
    cache_root: PathBuf,
}

impl LibretroThumbnailProvider {
    const BASE: &'static str = "https://thumbnails.libretro.com";

    pub fn new(cache_root: PathBuf) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Arcadia/0.1 (+https://github.com/arcadia-project/arcadia)")
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .unwrap_or_default();
        Self { client, cache_root }
    }

    /// Map an Arcadia platform slug to the libretro system folder name. Returns
    /// `None` for platforms libretro doesn't index under a name we support.
    fn system_name(platform: &str) -> Option<&'static str> {
        Some(match platform {
            "nes" => "Nintendo - Nintendo Entertainment System",
            "snes" => "Nintendo - Super Nintendo Entertainment System",
            "n64" => "Nintendo - Nintendo 64",
            "gamecube" => "Nintendo - GameCube",
            "wii" => "Nintendo - Wii",
            "gb" => "Nintendo - Game Boy",
            "gbc" => "Nintendo - Game Boy Color",
            "gba" => "Nintendo - Game Boy Advance",
            "nds" => "Nintendo - Nintendo DS",
            "virtualboy" => "Nintendo - Virtual Boy",
            "genesis" => "Sega - Mega Drive - Genesis",
            "dreamcast" => "Sega - Dreamcast",
            "ps1" => "Sony - PlayStation",
            "ps2" => "Sony - PlayStation 2",
            "psp" => "Sony - PlayStation Portable",
            "ps3" => "Sony - PlayStation 3",
            "saturn" => "Sega - Saturn",
            "segacd" => "Sega - Mega-CD - Sega CD",
            "sega32x" => "Sega - 32X",
            "gamegear" => "Sega - Game Gear",
            "sms" => "Sega - Master System - Mark III",
            "pcengine" => "NEC - PC Engine - TurboGrafx 16",
            "n3ds" => "Nintendo - Nintendo 3DS",
            "atari2600" => "Atari - 2600",
            "atari5200" => "Atari - 5200",
            "atari7800" => "Atari - 7800",
            "jaguar" => "Atari - Jaguar",
            "lynx" => "Atari - Lynx",
            "amiga" => "Commodore - Amiga",
            "c64" => "Commodore - 64",
            "colecovision" => "Coleco - ColecoVision",
            "msx" => "Microsoft - MSX",
            "xbox" => "Microsoft - Xbox",
            "ngp" => "SNK - Neo Geo Pocket",
            "wonderswan" => "Bandai - WonderSwan",
            "arcade" => "MAME",
            _ => return None,
        })
    }

    /// libretro replaces these characters with `_` in thumbnail file names.
    fn libretro_filename(stem: &str) -> String {
        stem.chars()
            .map(|c| match c {
                '&' | '*' | '/' | ':' | '`' | '<' | '>' | '?' | '\\' | '|' => '_',
                other => other,
            })
            .collect()
    }

    /// Percent-encode one URL path segment (keep RFC3986 unreserved bytes).
    fn encode_segment(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for &b in s.as_bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char)
                }
                _ => out.push_str(&format!("%{b:02X}")),
            }
        }
        out
    }

    /// Region words to try, the one detected in `stem` first, then the usual
    /// fallbacks. No-Intro uses full words ("USA"); GoodTools uses letters
    /// ("(U)", "(E)", "(J)") — we recognise both.
    fn regions_for(stem: &str) -> Vec<&'static str> {
        let s = stem.to_ascii_lowercase();
        let mut out: Vec<&'static str> = Vec::new();
        let add = |r: &'static str, out: &mut Vec<&'static str>| {
            if !out.contains(&r) {
                out.push(r);
            }
        };
        if ["(u)", "(usa", "(ntsc", "(ue)", "(uj)", "(ub)"]
            .iter()
            .any(|n| s.contains(n))
        {
            add("USA", &mut out);
        }
        if ["(e)", "(eur", "(pal", "(ue)", "(ej)"].iter().any(|n| s.contains(n)) {
            add("Europe", &mut out);
        }
        if ["(j)", "(jap", "(jpn", "(jp)", "(uj)", "(ej)"]
            .iter()
            .any(|n| s.contains(n))
        {
            add("Japan", &mut out);
        }
        for r in ["USA", "Europe", "Japan", "World"] {
            add(r, &mut out);
        }
        out
    }

    /// Ordered, de-duplicated list of names to probe on the thumbnail server.
    fn candidate_names(stem: &str, title: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for region in Self::regions_for(stem) {
            names.push(format!("{title} ({region})"));
        }
        names.push(stem.to_string());
        names.push(title.to_string());
        names.dedup();
        names
    }

    /// Deterministic per-ROM cache path (offline re-runs regardless of which
    /// candidate name actually matched).
    fn cache_path(&self, game: &Game) -> PathBuf {
        let stem = Path::new(&game.rom_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&game.title);
        self.cache_root
            .join("libretro")
            .join(&game.platform)
            .join(format!("{}.png", Self::libretro_filename(stem)))
    }

    /// Probe `names` in order on the thumbnail server; the first 200 is written
    /// to the per-ROM cache and returned as a cover patch. A total miss yields an
    /// empty patch. Overwrites any existing cache file, so a correction sticks.
    async fn probe_and_cache(&self, game: &Game, names: Vec<String>) -> MetadataPatch {
        let mut patch = MetadataPatch::default();
        let Some(system) = Self::system_name(&game.platform) else {
            return patch;
        };
        let cached = self.cache_path(game);
        let sys_enc = Self::encode_segment(system);
        for candidate in names {
            let name = Self::libretro_filename(&candidate);
            let url = format!(
                "{}/{}/Named_Boxarts/{}.png",
                Self::BASE,
                sys_enc,
                Self::encode_segment(&name),
            );
            let resp = match self.client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => {
                    tracing::debug!(%url, error = %e, "libretro request failed");
                    continue;
                }
            };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(bytes) = resp.bytes().await else {
                continue;
            };
            if let Some(parent) = cached.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if std::fs::write(&cached, &bytes).is_ok() {
                tracing::info!(game = %game.title, matched = %candidate, "fetched box art from libretro");
                patch.cover_art = Some(cached.to_string_lossy().to_string());
            }
            return patch;
        }
        patch
    }

    /// Re-probe using an explicit, user-supplied name (and its region variants),
    /// bypassing the cache check so a correction takes effect. Used when the ROM
    /// filename doesn't match libretro's No-Intro title (e.g. a missing subtitle
    /// like "Wave Race 64 - Kawasaki Jet Ski").
    pub async fn fetch_named(&self, game: &Game, query: &str) -> MetadataPatch {
        self.probe_and_cache(game, Self::candidate_names(query, query))
            .await
    }

    /// The full list of Named_Boxarts titles libretro has for a platform, parsed
    /// from the server's directory index and cached to disk (the archive is
    /// stable, and the listing can be large). Best-effort: returns empty on any
    /// network/parse failure.
    async fn boxart_index(&self, platform: &str) -> Vec<String> {
        let Some(system) = Self::system_name(platform) else {
            return Vec::new();
        };
        let cache = self
            .cache_root
            .join("libretro")
            .join("_index")
            .join(format!("{platform}.txt"));
        if let Ok(text) = std::fs::read_to_string(&cache) {
            let names: Vec<String> = text.lines().map(String::from).filter(|l| !l.is_empty()).collect();
            if !names.is_empty() {
                return names;
            }
        }

        let url = format!("{}/{}/Named_Boxarts/", Self::BASE, Self::encode_segment(system));
        let body = match self.client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r.text().await.unwrap_or_default(),
            Ok(r) => {
                tracing::debug!(%url, status = %r.status(), "libretro index not available");
                return Vec::new();
            }
            Err(e) => {
                tracing::debug!(%url, error = %e, "libretro index request failed");
                return Vec::new();
            }
        };
        let names = parse_boxart_index(&body);
        if !names.is_empty() {
            if let Some(parent) = cache.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&cache, names.join("\n"));
        }
        names
    }

    /// Rank libretro's catalog for a platform against `query`, returning the
    /// closest names (without the `.png`) for the user to choose from. Empty if
    /// the index can't be fetched.
    pub async fn suggest(&self, platform: &str, query: &str, limit: usize) -> Vec<String> {
        let names = self.boxart_index(platform).await;
        if names.is_empty() {
            return Vec::new();
        }
        let qt = normalize_tokens(query);
        let qjoined = qt.join(" ");
        let mut scored: Vec<(f64, &String)> = names
            .iter()
            .map(|n| (match_score(&qt, &qjoined, n), n))
            .filter(|(s, _)| *s > 0.0)
            .collect();
        scored.sort_by(|a, b| {
            b.0.partial_cmp(&a.0)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.1.len().cmp(&b.1.len()))
        });
        scored.into_iter().take(limit).map(|(_, n)| n.clone()).collect()
    }
}

/// Lowercase, split on any non-alphanumeric, drop empties.
pub(crate) fn normalize_tokens(s: &str) -> Vec<String> {
    s.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .map(String::from)
        .collect()
}

/// Token-overlap score with a contiguous-substring bonus. Higher is closer.
pub(crate) fn match_score(qt: &[String], qjoined: &str, name: &str) -> f64 {
    if qt.is_empty() {
        return 0.0;
    }
    let nt = normalize_tokens(name);
    if nt.is_empty() {
        return 0.0;
    }
    let nset: std::collections::HashSet<&str> = nt.iter().map(String::as_str).collect();
    let matched = qt.iter().filter(|t| nset.contains(t.as_str())).count();
    let mut score = matched as f64 / qt.len() as f64;
    if nt.join(" ").contains(qjoined) {
        score += 0.5;
    }
    score
}

/// Parse libretro's directory index (nginx autoindex HTML) into decoded box-art
/// names, stripping the `.png` suffix.
fn parse_boxart_index(html: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for part in html.split("href=\"").skip(1) {
        let Some(end) = part.find('"') else { continue };
        let href = &part[..end];
        let Some(stem) = href.strip_suffix(".png") else {
            continue;
        };
        let name = percent_decode(stem);
        if !name.is_empty() && !name.contains('/') {
            out.push(name);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ps3_disc_art_from_eboot_and_root() {
        let tmp = std::env::temp_dir().join(format!("arcadia_ps3art_{}", std::process::id()));
        let ps3_game = tmp.join("PS3_GAME");
        std::fs::create_dir_all(ps3_game.join("USRDIR")).unwrap();
        std::fs::write(ps3_game.join("ICON0.PNG"), b"icon").unwrap();
        std::fs::write(ps3_game.join("PIC1.PNG"), b"bg").unwrap();
        let eboot = ps3_game.join("USRDIR").join("EBOOT.BIN");
        std::fs::write(&eboot, b"boot").unwrap();

        // rom_path = EBOOT.BIN
        let patch = ps3_disc_art(&eboot.to_string_lossy()).expect("art from eboot");
        assert!(patch.cover_art.as_deref().unwrap().ends_with("ICON0.PNG"));
        assert!(patch.background_art.as_deref().unwrap().ends_with("PIC1.PNG"));

        // rom_path = disc root
        let patch = ps3_disc_art(&tmp.to_string_lossy()).expect("art from root");
        assert!(patch.cover_art.as_deref().unwrap().ends_with("ICON0.PNG"));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn ps3_disc_art_absent_when_no_files() {
        let tmp = std::env::temp_dir().join(format!("arcadia_ps3noart_{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(ps3_disc_art(&tmp.to_string_lossy()).is_none());
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn libretro_system_name_maps_arcade_to_mame() {
        assert_eq!(LibretroThumbnailProvider::system_name("arcade"), Some("MAME"));
        assert_eq!(LibretroThumbnailProvider::system_name("snes"), Some("Nintendo - Super Nintendo Entertainment System"));
        assert_eq!(LibretroThumbnailProvider::system_name("not_a_platform"), None);
    }

    #[test]
    fn parses_screenscraper_text_fields_preferring_language_and_region() {
        let body = r#"{
            "response": { "jeu": {
                "synopsis": [
                    {"langue":"fr","text":"Texte francais"},
                    {"langue":"en","text":"An English synopsis."}
                ],
                "genres": [
                    {"noms":[{"langue":"fr","text":"Aventure"},{"langue":"en","text":"Adventure"}]}
                ],
                "developpeur": {"id":"1","text":"Nintendo EAD"},
                "editeur": {"id":"2","text":"Nintendo"},
                "dates": [
                    {"region":"jp","text":"1998-11-21"},
                    {"region":"us","text":"1998-11-23"},
                    {"region":"wor","text":"1998-11-21"}
                ]
            }}
        }"#;
        let p = parse_screenscraper_json(body, "en");
        assert_eq!(p.description.as_deref(), Some("An English synopsis."));
        assert_eq!(p.genre.as_deref(), Some("Adventure"));
        assert_eq!(p.developer.as_deref(), Some("Nintendo EAD"));
        assert_eq!(p.publisher.as_deref(), Some("Nintendo"));
        // "wor" is preferred over jp/us even though it appears last.
        assert_eq!(p.release_date.as_deref(), Some("1998-11-21"));
    }

    #[test]
    fn screenscraper_parse_falls_back_when_language_absent() {
        let body = r#"{"response":{"jeu":{
            "synopsis":[{"langue":"de","text":"Deutscher Text"}],
            "dates":[{"region":"kr","text":"1999-01-01"}]
        }}}"#;
        let p = parse_screenscraper_json(body, "en");
        assert_eq!(p.description.as_deref(), Some("Deutscher Text"));
        assert_eq!(p.release_date.as_deref(), Some("1999-01-01"));
        assert!(p.genre.is_none());
    }

    #[test]
    fn screenscraper_parse_empty_on_garbage_or_error_body() {
        assert!(parse_screenscraper_json("not json", "en").is_empty());
        assert!(parse_screenscraper_json(r#"{"response":{}}"#, "en").is_empty());
    }

    #[test]
    fn rank_cover_candidates_lists_libretro_first_and_tags_sources() {
        let out = rank_cover_candidates(
            vec!["Street Fighter II".into()],
            vec![("Street Fighter II Turbo".into(), "s/sf2t.jpg".into())],
        );
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].source, "libretro");
        assert_eq!(out[0].label, "Street Fighter II");
        assert_eq!(out[0].token, "Street Fighter II"); // libretro token == name
        assert_eq!(out[1].source, "launchbox");
        assert_eq!(out[1].label, "Street Fighter II Turbo");
        assert_eq!(out[1].token, "s/sf2t.jpg"); // launchbox token == file_name
    }
}

#[async_trait]
impl MetadataProvider for LibretroThumbnailProvider {
    fn id(&self) -> &'static str {
        "libretro-thumbnails"
    }

    async fn fetch(&self, game: &Game) -> Result<MetadataPatch> {
        let mut patch = MetadataPatch::default();
        if Self::system_name(&game.platform).is_none() {
            return Ok(patch);
        }

        // Offline if we've already matched this ROM before.
        let cached = self.cache_path(game);
        if cached.is_file() {
            patch.cover_art = Some(cached.to_string_lossy().to_string());
            return Ok(patch);
        }

        let stem = Path::new(&game.rom_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&game.title);
        patch = self
            .probe_and_cache(game, Self::candidate_names(stem, &game.title))
            .await;
        if patch.is_empty() {
            tracing::debug!(game = %game.title, platform = %game.platform, "no libretro box art matched");
        }
        Ok(patch)
    }
}

/// ScreenScraper (screenscraper.fr): a community game database with rich *text*
/// metadata — synopsis, genre, developer, publisher, release date. It requires
/// user-owned credentials (a per-app dev key plus, optionally, the user's own
/// account) and enforces per-account rate quotas, so we: (a) no-op entirely when
/// unconfigured, (b) cache every parsed response to disk so a re-run is offline,
/// and (c) populate *text only* — box art stays the libretro provider's job, to
/// avoid two sources fighting over the cover.
pub struct ScreenScraperProvider {
    client: reqwest::Client,
    creds: ScreenScraperCredentials,
    cache_root: PathBuf,
    /// Preferred synopsis/genre language (ScreenScraper uses ISO-639-1 codes).
    lang: String,
}

impl ScreenScraperProvider {
    const BASE: &'static str = "https://www.screenscraper.fr/api2";

    pub fn new(creds: ScreenScraperCredentials, cache_root: PathBuf) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Arcadia/0.1 (+https://github.com/arcadia-project/arcadia)")
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .unwrap_or_default();
        Self {
            client,
            creds,
            cache_root,
            lang: "en".to_string(),
        }
    }

    /// Map an Arcadia platform slug to ScreenScraper's numeric `systemeid`.
    /// `None` for platforms we don't have a stable id for (provider skips them).
    fn system_id(platform: &str) -> Option<u32> {
        Some(match platform {
            "nes" => 3,
            "snes" => 4,
            "n64" => 14,
            "gamecube" => 13,
            "wii" => 16,
            "gb" => 9,
            "gbc" => 10,
            "gba" => 12,
            "nds" => 15,
            "virtualboy" => 11,
            "genesis" => 1,
            "dreamcast" => 23,
            "ps1" => 57,
            "ps2" => 58,
            "psp" => 61,
            "ps3" => 59,
            "vita" => 62,
            "saturn" => 22,
            "segacd" => 20,
            "sega32x" => 19,
            "gamegear" => 21,
            "sms" => 2,
            "pcengine" => 31,
            "n3ds" => 17,
            "switch" => 225,
            "atari2600" => 26,
            "atari5200" => 40,
            "atari7800" => 41,
            "jaguar" => 27,
            "lynx" => 28,
            "amiga" => 64,
            "c64" => 66,
            "colecovision" => 48,
            "msx" => 113,
            "xbox" => 32,
            "ngp" => 25,
            "wonderswan" => 45,
            _ => return None,
        })
    }

    /// Per-game cache path for the raw JSON response.
    fn cache_path(&self, game: &Game) -> PathBuf {
        let stem = Path::new(&game.rom_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&game.title);
        let safe: String = stem
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        self.cache_root
            .join("screenscraper")
            .join(&game.platform)
            .join(format!("{safe}.json"))
    }

    /// Request jeuInfos.php for a game by name + system. Returns the raw body on
    /// 200, `None` on any failure (network, auth, not-found) — all non-fatal.
    async fn request(&self, game: &Game, system_id: u32) -> Option<String> {
        let q = |k: &str, v: &str| format!("{k}={}", urlencode(v));
        let mut params = vec![
            "output=json".to_string(),
            q("devid", &self.creds.dev_id),
            q("devpassword", &self.creds.dev_password),
            "softname=Arcadia".to_string(),
            format!("systemeid={system_id}"),
            q("romnom", &game.title),
        ];
        if !self.creds.user_id.trim().is_empty() {
            params.push(q("ssid", &self.creds.user_id));
            params.push(q("sspassword", &self.creds.user_password));
        }
        let url = format!("{}/jeuInfos.php?{}", Self::BASE, params.join("&"));
        match self.client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r.text().await.ok(),
            Ok(r) => {
                tracing::debug!(game = %game.title, status = %r.status(), "screenscraper miss");
                None
            }
            Err(e) => {
                tracing::debug!(game = %game.title, error = %e, "screenscraper request failed");
                None
            }
        }
    }
}

#[async_trait]
impl MetadataProvider for ScreenScraperProvider {
    fn id(&self) -> &'static str {
        "screenscraper"
    }

    async fn fetch(&self, game: &Game) -> Result<MetadataPatch> {
        let empty = MetadataPatch::default();
        if !self.creds.is_configured() {
            return Ok(empty);
        }
        let Some(system_id) = Self::system_id(&game.platform) else {
            return Ok(empty);
        };

        // Offline if we've already fetched this ROM's record.
        let cache = self.cache_path(game);
        if let Ok(text) = std::fs::read_to_string(&cache) {
            return Ok(parse_screenscraper_json(&text, &self.lang));
        }

        let Some(body) = self.request(game, system_id).await else {
            return Ok(empty);
        };
        let patch = parse_screenscraper_json(&body, &self.lang);
        // Cache only a response that actually parsed into something useful, so a
        // transient empty/error body doesn't poison future offline runs.
        if !patch.is_empty() {
            if let Some(parent) = cache.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&cache, &body);
            tracing::info!(game = %game.title, "fetched text metadata from screenscraper");
        }
        Ok(patch)
    }
}

/// Parse a ScreenScraper `jeuInfos` JSON body into a text-only patch. Navigates
/// defensively over `serde_json::Value` because the schema is loose (fields are
/// arrays of localized/regional variants). Preferred language `lang` wins for
/// synopsis/genre; release date prefers world/US/EU/JP regions in that order.
fn parse_screenscraper_json(body: &str, lang: &str) -> MetadataPatch {
    let mut patch = MetadataPatch::default();
    let Ok(root) = serde_json::from_str::<serde_json::Value>(body) else {
        return patch;
    };
    let jeu = &root["response"]["jeu"];
    if !jeu.is_object() {
        return patch;
    }

    // Synopsis: array of { langue, text }; prefer requested language.
    patch.description = pick_localized(&jeu["synopsis"], "langue", lang);

    // Genres: array of { noms: [{ langue, text }] }; take the first genre's
    // localized name.
    if let Some(first) = jeu["genres"].as_array().and_then(|a| a.first()) {
        patch.genre = pick_localized(&first["noms"], "langue", lang);
    }

    // Developer / publisher: single objects with a `text` field.
    patch.developer = string_field(&jeu["developpeur"]["text"]);
    patch.publisher = string_field(&jeu["editeur"]["text"]);

    // Dates: array of { region, text }; prefer world then the big regions.
    patch.release_date = pick_regional(&jeu["dates"], &["wor", "us", "eu", "jp", "ss"]);

    patch
}

/// From an array of `{ <key>: <code>, "text": <value> }`, return the `text`
/// whose code matches `want`, else the first non-empty `text`.
fn pick_localized(arr: &serde_json::Value, key: &str, want: &str) -> Option<String> {
    let items = arr.as_array()?;
    let mut fallback: Option<String> = None;
    for it in items {
        let text = it["text"].as_str().map(str::trim).filter(|s| !s.is_empty());
        let Some(text) = text else { continue };
        if it[key].as_str() == Some(want) {
            return Some(text.to_string());
        }
        fallback.get_or_insert_with(|| text.to_string());
    }
    fallback
}

/// From an array of `{ "region": <code>, "text": <value> }`, return the `text`
/// for the first preferred region present, else the first non-empty `text`.
fn pick_regional(arr: &serde_json::Value, prefer: &[&str]) -> Option<String> {
    let items = arr.as_array()?;
    for region in prefer {
        for it in items {
            if it["region"].as_str() == Some(*region) {
                if let Some(t) = it["text"].as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    return Some(t.to_string());
                }
            }
        }
    }
    items
        .iter()
        .find_map(|it| it["text"].as_str().map(str::trim).filter(|s| !s.is_empty()))
        .map(str::to_string)
}

fn string_field(v: &serde_json::Value) -> Option<String> {
    v.as_str().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// Minimal application/x-www-form-urlencoded encoder for query values.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

impl Engine {
    /// Enrich games that are missing a cover using the local provider. Returns
    /// the number of games updated.
    pub async fn enrich_metadata_local(&self, profile_id: &str) -> Result<usize> {
        let games = sqlx::query_as::<_, Game>(
            "SELECT * FROM games WHERE profile_id = ? AND cover_art IS NULL",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;
        if games.is_empty() {
            return Ok(0);
        }

        tracing::info!(count = games.len(), "local artwork pass started");

        // Probing for on-disk art is pure blocking filesystem work (dozens of
        // stats per game) and slow on external drives — run it off the async
        // runtime, then apply the resulting patches here.
        let patches = tokio::task::spawn_blocking(move || {
            games
                .into_iter()
                .filter_map(|game| {
                    let patch = local_cover_art(&game);
                    (!patch.is_empty()).then_some((game.id, patch))
                })
                .collect::<Vec<_>>()
        })
        .await
        .map_err(|e| EngineError::Invalid(format!("artwork probe panicked: {e}")))?;

        let mut updated = 0usize;
        for (id, patch) in patches {
            self.apply_patch(&id, &patch).await?;
            updated += 1;
        }
        tracing::info!(updated, "local artwork pass complete");
        Ok(updated)
    }

    /// Enrich games missing a cover: try local artwork first (free, offline),
    /// then fall back to the libretro thumbnail archive for whatever is still
    /// blank. Downloads are cached under the artwork dir. Returns the number of
    /// games that gained a cover. Best-effort: network failures are skipped.
    pub async fn enrich_metadata_online(&self, profile_id: &str) -> Result<usize> {
        let mut updated = self.enrich_metadata_local(profile_id).await?;

        let games = sqlx::query_as::<_, Game>(
            "SELECT * FROM games WHERE profile_id = ? AND cover_art IS NULL",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;
        if games.is_empty() {
            return Ok(updated);
        }

        // Fetch box art concurrently (network-bound, and many requests 404),
        // but cap concurrency to stay polite to the libretro CDN. DB writes are
        // kept serialized on this task — SQLite has a single writer.
        let total = games.len();
        self.emit_progress(profile_id, "artwork", "Fetching box art", 0, total, false);
        let provider = Arc::new(LibretroThumbnailProvider::new(self.paths.artwork_dir()));
        let sem = Arc::new(Semaphore::new(8));
        let mut set = tokio::task::JoinSet::new();
        for game in games {
            let provider = provider.clone();
            let sem = sem.clone();
            set.spawn(async move {
                let _permit = sem.acquire_owned().await.ok()?;
                let patch = provider.fetch(&game).await.ok()?;
                if patch.is_empty() {
                    return None;
                }
                Some((game.id, patch))
            });
        }

        let mut processed = 0usize;
        while let Some(res) = set.join_next().await {
            if let Ok(Some((id, patch))) = res {
                self.apply_patch(&id, &patch).await?;
                updated += 1;
            }
            processed += 1;
            self.emit_progress(profile_id, "artwork", "Fetching box art", processed, total, false);
        }
        self.emit_progress(profile_id, "artwork", "Box art complete", total, total, true);

        // LaunchBox cover fallback (opt-in). Only runs when enabled AND the
        // user has already downloaded an index — never triggers a download.
        if let Some(provider) = self.launchbox_provider().await {
            let games = sqlx::query_as::<_, Game>(
                "SELECT * FROM games WHERE profile_id = ? AND cover_art IS NULL",
            )
            .bind(profile_id)
            .fetch_all(&self.pool)
            .await?;
            if !games.is_empty() {
                let total = games.len();
                self.emit_progress(profile_id, "artwork", "LaunchBox covers", 0, total, false);
                let provider = Arc::new(provider);
                let sem = Arc::new(Semaphore::new(8));
                let mut set = tokio::task::JoinSet::new();
                for game in games {
                    let provider = provider.clone();
                    let sem = sem.clone();
                    set.spawn(async move {
                        let _permit = sem.acquire_owned().await.ok()?;
                        let patch = provider.fetch(&game).await.ok()?;
                        if patch.is_empty() {
                            return None;
                        }
                        Some((game.id, patch))
                    });
                }
                let mut processed = 0usize;
                while let Some(res) = set.join_next().await {
                    if let Ok(Some((id, patch))) = res {
                        self.apply_patch(&id, &patch).await?;
                        updated += 1;
                    }
                    processed += 1;
                    self.emit_progress(profile_id, "artwork", "LaunchBox covers", processed, total, false);
                }
                self.emit_progress(profile_id, "artwork", "LaunchBox covers complete", total, total, true);
            }
        }

        // Text metadata pass (synopsis/genre/developer/publisher/date). No-ops
        // unless the user has configured ScreenScraper credentials.
        self.enrich_text_metadata(profile_id).await?;
        Ok(updated)
    }

    /// Fill missing *text* metadata (description/genre/developer/publisher/
    /// release_date) from ScreenScraper. No-op without user credentials. Targets
    /// games still missing a description — the field that most signals an
    /// un-enriched record. Returns the number of games that gained text.
    pub async fn enrich_text_metadata(&self, profile_id: &str) -> Result<usize> {
        let creds = self.load_config().screenscraper;
        if !creds.is_configured() {
            return Ok(0);
        }
        let games = sqlx::query_as::<_, Game>(
            "SELECT * FROM games WHERE profile_id = ? AND description IS NULL",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;
        if games.is_empty() {
            return Ok(0);
        }

        // ScreenScraper enforces a per-account request rate; keep concurrency
        // low and serialize DB writes (single SQLite writer).
        let provider = Arc::new(ScreenScraperProvider::new(creds, self.paths.artwork_dir()));
        let sem = Arc::new(Semaphore::new(2));
        let mut set = tokio::task::JoinSet::new();
        for game in games {
            let provider = provider.clone();
            let sem = sem.clone();
            set.spawn(async move {
                let _permit = sem.acquire_owned().await.ok()?;
                let patch = provider.fetch(&game).await.ok()?;
                if patch.is_empty() {
                    return None;
                }
                Some((game.id, patch))
            });
        }
        let mut updated = 0usize;
        while let Some(res) = set.join_next().await {
            if let Ok(Some((id, patch))) = res {
                self.apply_patch(&id, &patch).await?;
                updated += 1;
            }
        }
        Ok(updated)
    }

    /// Manually set a game's cover from a local image file. The file is copied
    /// into the artwork cache (which the asset protocol is allowed to serve), so
    /// the override is stable even if the user later moves the original. Returns
    /// the new cover path.
    pub async fn set_game_cover(&self, game_id: &str, source_path: &str) -> Result<String> {
        let src = Path::new(source_path);
        if !src.is_file() {
            return Err(EngineError::Invalid(format!("not a file: {source_path}")));
        }
        let ext = src
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_ascii_lowercase();
        let dir = self.paths.artwork_dir().join("manual");
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(format!("{game_id}.{ext}"));
        std::fs::copy(src, &dest)?;
        let cover = dest.to_string_lossy().to_string();
        let patch = MetadataPatch {
            cover_art: Some(cover.clone()),
            ..Default::default()
        };
        self.apply_patch(game_id, &patch).await?;
        Ok(cover)
    }

    /// Apply a cover the user picked from `suggest_covers`. Routes by `source`:
    /// `"libretro"` re-fetches by the libretro box-art name; `"launchbox"`
    /// downloads the specific image file. Returns whether a cover was applied
    /// (false when the download produced nothing).
    pub async fn apply_cover(&self, game_id: &str, source: &str, token: &str) -> Result<bool> {
        let game = self
            .get_game(game_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("game {game_id}")))?;

        let patch = match source {
            "libretro" => {
                LibretroThumbnailProvider::new(self.paths.artwork_dir())
                    .fetch_named(&game, token)
                    .await
            }
            "launchbox" => match self.launchbox_provider().await {
                Some(provider) => provider.fetch_file(&game, token).await,
                None => MetadataPatch::default(),
            },
            other => return Err(EngineError::Invalid(format!("unknown cover source: {other}"))),
        };

        if patch.is_empty() {
            return Ok(false);
        }
        self.apply_patch(game_id, &patch).await?;
        Ok(true)
    }

    /// Suggest the closest box-art candidates for a game so the user can pick
    /// the right one when auto-matching fails. Libretro is always consulted;
    /// LaunchBox is appended only when the feature is enabled and an index
    /// exists. `query` overrides the search text; otherwise the title is used.
    pub async fn suggest_covers(&self, game_id: &str, query: Option<&str>) -> Result<Vec<CoverCandidate>> {
        let game = self
            .get_game(game_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("game {game_id}")))?;
        let q = query
            .map(str::trim)
            .filter(|q| !q.is_empty())
            .unwrap_or(&game.title);

        let libretro = LibretroThumbnailProvider::new(self.paths.artwork_dir())
            .suggest(&game.platform, q, 12)
            .await;

        let launchbox = match self.launchbox_provider().await {
            Some(provider) => provider.search(&game.platform, q, 12).await,
            None => Vec::new(),
        };

        Ok(rank_cover_candidates(libretro, launchbox))
    }

    async fn apply_patch(&self, game_id: &str, patch: &MetadataPatch) -> Result<()> {
        sqlx::query(
            r#"
            UPDATE games SET
                cover_art      = COALESCE(?, cover_art),
                background_art = COALESCE(?, background_art),
                description    = COALESCE(?, description),
                genre          = COALESCE(?, genre),
                developer      = COALESCE(?, developer),
                publisher      = COALESCE(?, publisher),
                release_date   = COALESCE(?, release_date)
            WHERE id = ?
            "#,
        )
        .bind(&patch.cover_art)
        .bind(&patch.background_art)
        .bind(&patch.description)
        .bind(&patch.genre)
        .bind(&patch.developer)
        .bind(&patch.publisher)
        .bind(&patch.release_date)
        .bind(game_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}
