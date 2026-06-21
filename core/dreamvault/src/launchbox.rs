//! LaunchBox Games Database cover provider. Opt-in, fallback-after-libretro.
//!
//! LaunchBox has no API; the sanctioned path (endorsed by LaunchBox's founder)
//! is the daily `Metadata.zip` dump. We stream-parse it into a compact sidecar
//! SQLite index keyed by (arcadia-platform, normalized-name) -> image file
//! path, then resolve covers offline. Cover images only; text stays
//! ScreenScraper's job.

/// Sanctioned metadata dump (the whole catalogue; hundreds of MB).
pub const METADATA_URL: &str = "https://gamesdb.launchbox-app.com/Metadata.zip";
/// Base for image URLs built from a GameImage `FileName`.
pub const IMAGE_BASE: &str = "https://images.launchbox-app.com";

/// Lowercase, replace every non-alphanumeric run with a single space, trim.
/// Used as the join key so "Pokémon: Red!" and "pokemon red" collide.
pub(crate) fn normalize_name(s: &str) -> String {
    s.to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Map a LaunchBox `<Platform>` name to an Arcadia platform slug. `None` for
/// platforms Arcadia doesn't track (those games are skipped at index time).
pub(crate) fn arcadia_slug_for_launchbox(name: &str) -> Option<&'static str> {
    Some(match name {
        "Nintendo Entertainment System" => "nes",
        "Super Nintendo Entertainment System" => "snes",
        "Nintendo 64" => "n64",
        "Nintendo GameCube" => "gamecube",
        "Nintendo Wii" => "wii",
        "Nintendo Game Boy" => "gb",
        "Nintendo Game Boy Color" => "gbc",
        "Nintendo Game Boy Advance" => "gba",
        "Nintendo DS" => "nds",
        "Nintendo Virtual Boy" => "virtualboy",
        "Sega Genesis" => "genesis",
        "Sega Dreamcast" => "dreamcast",
        "Sony Playstation" => "ps1",
        "Sony Playstation 2" => "ps2",
        "Sony PSP" => "psp",
        "Sony Playstation 3" => "ps3",
        "Sega Saturn" => "saturn",
        "Sega CD" => "segacd",
        "Sega 32X" => "sega32x",
        "Sega Game Gear" => "gamegear",
        "Sega Master System" => "sms",
        "NEC TurboGrafx-16" => "pcengine",
        "Nintendo 3DS" => "n3ds",
        "Atari 2600" => "atari2600",
        "Atari 5200" => "atari5200",
        "Atari 7800" => "atari7800",
        "Atari Jaguar" => "jaguar",
        "Atari Lynx" => "lynx",
        "Commodore Amiga" => "amiga",
        "Commodore 64" => "c64",
        "ColecoVision" => "colecovision",
        "Microsoft MSX" => "msx",
        "Microsoft Xbox" => "xbox",
        "SNK Neo Geo Pocket" => "ngp",
        "WonderSwan" => "wonderswan",
        _ => return None,
    })
}

use crate::error::Result;
use crate::metadata::{MetadataPatch, MetadataProvider};
use crate::models::Game;
use crate::Engine;
use async_trait::async_trait;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use std::collections::HashMap;

#[derive(Debug, Clone)]
pub(crate) struct IndexRow {
    pub platform: String,
    pub norm_name: String,
    pub file_name: String,
}

/// Region preference for choosing among multiple "Box - Front" images of one
/// game. Higher wins; first-seen wins ties.
fn region_score(region: &str) -> i32 {
    match region {
        "North America" => 4,
        "World" => 3,
        "Europe" => 2,
        "Japan" => 1,
        _ => 0,
    }
}

#[derive(PartialEq)]
enum Section {
    None,
    Game,
    Alt,
    Image,
}

/// Stream-parse a LaunchBox `Metadata.xml` into index rows. Only games on a
/// platform Arcadia tracks and with a "Box - Front" image are emitted. Memory
/// is bounded by the catalogue of supported platforms, not the file size.
pub(crate) fn parse_metadata_xml<R: std::io::BufRead>(reader: R) -> Vec<IndexRow> {
    use quick_xml::events::Event;
    let mut xml = quick_xml::Reader::from_reader(reader);
    let mut buf = Vec::new();

    let mut section = Section::None;
    let mut field = String::new();

    // Per-record scratch.
    let (mut g_id, mut g_name, mut g_platform) = (String::new(), String::new(), String::new());
    let (mut a_id, mut a_name) = (String::new(), String::new());
    let (mut i_id, mut i_file, mut i_type, mut i_region) =
        (String::new(), String::new(), String::new(), String::new());

    // db_id -> arcadia slug (only supported platforms)
    let mut game_platform: HashMap<String, String> = HashMap::new();
    // db_id -> set of normalized names (primary + alternates)
    let mut game_names: HashMap<String, Vec<String>> = HashMap::new();
    // db_id -> (best file_name, best score) for Box - Front
    let mut box_front: HashMap<String, (String, i32)> = HashMap::new();

    loop {
        match xml.read_event_into(&mut buf) {
            Ok(Event::Start(e)) => {
                let name = e.name();
                let tag = String::from_utf8_lossy(name.as_ref()).to_string();
                match tag.as_str() {
                    "Game" => {
                        section = Section::Game;
                        g_id.clear(); g_name.clear(); g_platform.clear();
                    }
                    "GameAlternateName" => {
                        section = Section::Alt;
                        a_id.clear(); a_name.clear();
                    }
                    "GameImage" => {
                        section = Section::Image;
                        i_id.clear(); i_file.clear(); i_type.clear(); i_region.clear();
                    }
                    _ => field = tag,
                }
            }
            Ok(Event::Text(t)) => {
                let text = t.unescape().unwrap_or_default().to_string();
                match section {
                    Section::Game => match field.as_str() {
                        "DatabaseID" => g_id.push_str(&text),
                        "Name" => g_name.push_str(&text),
                        "Platform" => g_platform.push_str(&text),
                        _ => {}
                    },
                    Section::Alt => match field.as_str() {
                        "DatabaseID" => a_id.push_str(&text),
                        "AlternateName" => a_name.push_str(&text),
                        _ => {}
                    },
                    Section::Image => match field.as_str() {
                        "DatabaseID" => i_id.push_str(&text),
                        "FileName" => i_file.push_str(&text),
                        "Type" => i_type.push_str(&text),
                        "Region" => i_region.push_str(&text),
                        _ => {}
                    },
                    Section::None => {}
                }
            }
            Ok(Event::End(e)) => {
                let name = e.name();
                let tag = String::from_utf8_lossy(name.as_ref()).to_string();
                match tag.as_str() {
                    "Game" => {
                        if let Some(slug) = arcadia_slug_for_launchbox(g_platform.trim()) {
                            if !g_id.is_empty() && !g_name.trim().is_empty() {
                                game_platform.insert(g_id.clone(), slug.to_string());
                                game_names
                                    .entry(g_id.clone())
                                    .or_default()
                                    .push(normalize_name(&g_name));
                            }
                        }
                        section = Section::None;
                    }
                    "GameAlternateName" => {
                        if !a_id.is_empty() && !a_name.trim().is_empty() {
                            game_names
                                .entry(a_id.clone())
                                .or_default()
                                .push(normalize_name(&a_name));
                        }
                        section = Section::None;
                    }
                    "GameImage" => {
                        if i_type.trim() == "Box - Front" && !i_id.is_empty() && !i_file.trim().is_empty() {
                            let score = region_score(i_region.trim());
                            let better = match box_front.get(&i_id) {
                                Some((_, best)) => score > *best,
                                None => true,
                            };
                            if better {
                                box_front.insert(i_id.clone(), (i_file.trim().to_string(), score));
                            }
                        }
                        section = Section::None;
                    }
                    _ => field.clear(),
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => {
                tracing::warn!(error = %e, "launchbox metadata parse error; stopping");
                break;
            }
            _ => {}
        }
        buf.clear();
    }

    // Join games (with their platform + names) against the chosen box-front.
    let mut rows = Vec::new();
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    for (id, platform) in &game_platform {
        let Some((file_name, _)) = box_front.get(id) else { continue };
        let Some(names) = game_names.get(id) else { continue };
        for norm in names {
            if norm.is_empty() {
                continue;
            }
            let key = (platform.clone(), norm.clone());
            if seen.insert(key) {
                rows.push(IndexRow {
                    platform: platform.clone(),
                    norm_name: norm.clone(),
                    file_name: file_name.clone(),
                });
            }
        }
    }
    rows
}

/// Build a fresh sidecar index at `db_path`, replacing any existing file.
/// Journal mode is DELETE so the index is a single file the caller can rename
/// atomically. Returns the number of rows inserted.
pub(crate) async fn build_index_db(db_path: &Path, rows: &[IndexRow]) -> Result<usize> {
    // Start clean so a rebuild never merges into stale data.
    let _ = std::fs::remove_file(db_path);
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
        .map_err(|e| crate::error::EngineError::Invalid(format!("launchbox index path: {e}")))?
        .create_if_missing(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Delete);
    let pool = SqlitePoolOptions::new().max_connections(1).connect_with(opts).await?;

    sqlx::query("CREATE TABLE covers (platform TEXT NOT NULL, norm_name TEXT NOT NULL, file_name TEXT NOT NULL, PRIMARY KEY (platform, norm_name))")
        .execute(&pool)
        .await?;

    let mut tx = pool.begin().await?;
    let mut inserted = 0usize;
    for row in rows {
        let res = sqlx::query("INSERT OR IGNORE INTO covers (platform, norm_name, file_name) VALUES (?, ?, ?)")
            .bind(&row.platform)
            .bind(&row.norm_name)
            .bind(&row.file_name)
            .execute(&mut *tx)
            .await?;
        inserted += res.rows_affected() as usize;
    }
    tx.commit().await?;
    pool.close().await;
    Ok(inserted)
}

/// Open a read-only pool against an existing sidecar index.
pub(crate) async fn open_index_pool(db_path: &Path) -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))
        .map_err(|e| crate::error::EngineError::Invalid(format!("launchbox index path: {e}")))?
        .read_only(true);
    let pool = SqlitePoolOptions::new().max_connections(2).connect_with(opts).await?;
    Ok(pool)
}

/// Look up the stored image file path for a (platform, normalized name) key.
pub(crate) async fn lookup_cover(pool: &SqlitePool, platform: &str, norm_name: &str) -> Option<String> {
    sqlx::query_scalar::<_, String>("SELECT file_name FROM covers WHERE platform = ? AND norm_name = ?")
        .bind(platform)
        .bind(norm_name)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

/// Percent-encode each path segment of a GameImage `FileName`, preserving `/`.
fn image_url(file_name: &str) -> String {
    let encoded: Vec<String> = file_name
        .split('/')
        .map(|seg| {
            let mut out = String::with_capacity(seg.len());
            for &b in seg.as_bytes() {
                match b {
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
                    _ => out.push_str(&format!("%{b:02X}")),
                }
            }
            out
        })
        .collect();
    format!("{}/{}", IMAGE_BASE, encoded.join("/"))
}

/// Cover provider backed by the sidecar index. Construct only when the index
/// exists (see `Engine::launchbox_provider`).
pub struct LaunchBoxProvider {
    client: reqwest::Client,
    cache_root: PathBuf,
    pool: SqlitePool,
}

impl LaunchBoxProvider {
    pub fn new(cache_root: PathBuf, pool: SqlitePool) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("Arcadia/0.1 (+https://github.com/arcadia-project/arcadia)")
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        Self { client, cache_root, pool }
    }

    fn cache_path(&self, game: &Game, file_name: &str) -> PathBuf {
        let stem = Path::new(&game.rom_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&game.title);
        let safe: String = stem
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
            .collect();
        let ext = Path::new(file_name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("jpg")
            .to_ascii_lowercase();
        self.cache_root
            .join("launchbox")
            .join(&game.platform)
            .join(format!("{safe}.{ext}"))
    }
}

#[async_trait]
impl MetadataProvider for LaunchBoxProvider {
    fn id(&self) -> &'static str {
        "launchbox"
    }

    async fn fetch(&self, game: &Game) -> Result<MetadataPatch> {
        let mut patch = MetadataPatch::default();
        let norm = normalize_name(&game.title);
        let Some(file_name) = lookup_cover(&self.pool, &game.platform, &norm).await else {
            return Ok(patch);
        };

        let cached = self.cache_path(game, &file_name);
        if cached.is_file() {
            patch.cover_art = Some(cached.to_string_lossy().to_string());
            return Ok(patch);
        }

        let url = image_url(&file_name);
        let resp = match self.client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::debug!(%url, status = %r.status(), "launchbox image miss");
                return Ok(patch);
            }
            Err(e) => {
                tracing::debug!(%url, error = %e, "launchbox image request failed");
                return Ok(patch);
            }
        };
        let Ok(bytes) = resp.bytes().await else { return Ok(patch) };
        if let Some(parent) = cached.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&cached, &bytes).is_ok() {
            tracing::info!(game = %game.title, "fetched box art from launchbox");
            patch.cover_art = Some(cached.to_string_lossy().to_string());
        }
        Ok(patch)
    }
}

impl Engine {
    /// Path to the sidecar cover index.
    pub fn launchbox_index_path(&self) -> PathBuf {
        self.paths.artwork_dir().join("launchbox").join("index.sqlite")
    }

    /// Download the LaunchBox metadata dump, rebuild the sidecar index, and
    /// record the refresh time. This is the ONLY code path that downloads the
    /// dump. Returns the number of indexed (platform, name) entries.
    pub async fn refresh_launchbox_index(&self) -> Result<usize> {
        let index_path = self.launchbox_index_path();
        let dir = index_path
            .parent()
            .expect("launchbox index path always has a parent dir")
            .to_path_buf();
        std::fs::create_dir_all(&dir)?;

        // 1. Stream the (hundreds of MB) zip to a temp file on disk, chunk by
        //    chunk, so the whole dump is never held in memory at once. The zip
        //    central directory is at the end, so we need Seek to read entries.
        tracing::info!("downloading LaunchBox metadata dump");
        let client = reqwest::Client::builder()
            .user_agent("Arcadia/0.1 (+https://github.com/arcadia-project/arcadia)")
            .build()
            .unwrap_or_default();
        let mut resp = client.get(METADATA_URL).send().await?.error_for_status()?;
        let mut zip_tmp = tempfile::Builder::new().prefix("launchbox-").suffix(".zip").tempfile_in(&dir)?;
        while let Some(chunk) = resp.chunk().await? {
            std::io::Write::write_all(&mut zip_tmp, &chunk)?;
        }
        std::io::Write::flush(&mut zip_tmp)?;

        // 2. Stream-parse Metadata.xml out of the zip (never load it whole).
        let rows = {
            let file = zip_tmp.reopen()?;
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| crate::error::EngineError::Invalid(format!("launchbox zip: {e}")))?;
            let entry = archive
                .by_name("Metadata.xml")
                .map_err(|e| crate::error::EngineError::Invalid(format!("Metadata.xml missing: {e}")))?;
            parse_metadata_xml(std::io::BufReader::new(entry))
        };
        drop(zip_tmp); // deletes the temp zip
        tracing::info!(rows = rows.len(), "parsed LaunchBox metadata");

        // 3. Build into a temp sidecar, then atomically swap into place so a
        //    failed/partial build never replaces a working index.
        let build_tmp = index_path.with_extension("sqlite.tmp");
        let count = build_index_db(&build_tmp, &rows).await?;
        std::fs::rename(&build_tmp, &index_path)?;

        // 4. Record the refresh timestamp.
        let mut cfg = self.load_config();
        cfg.launchbox.last_refresh = Some(chrono::Utc::now().timestamp());
        let _ = self.save_config(&cfg);

        tracing::info!(count, "LaunchBox index rebuilt");
        Ok(count)
    }

    /// Build a provider if the feature is enabled AND an index already exists.
    /// Never downloads — returns `None` if the user hasn't refreshed yet.
    pub(crate) async fn launchbox_provider(&self) -> Option<LaunchBoxProvider> {
        if !self.load_config().launchbox.is_enabled() {
            return None;
        }
        let index_path = self.launchbox_index_path();
        if !index_path.is_file() {
            tracing::debug!("launchbox enabled but no index built yet; skipping");
            return None;
        }
        let pool = open_index_pool(&index_path).await.ok()?;
        Some(LaunchBoxProvider::new(self.paths.artwork_dir(), pool))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_collapses_punctuation_and_case() {
        assert_eq!(normalize_name("Pokémon: Red!"), "pok mon red");
        assert_eq!(normalize_name("  Super   Mario  64 "), "super mario 64");
        assert_eq!(normalize_name("The Legend of Zelda"), "the legend of zelda");
    }

    #[test]
    fn platform_mapping_known_and_unknown() {
        assert_eq!(arcadia_slug_for_launchbox("Super Nintendo Entertainment System"), Some("snes"));
        assert_eq!(arcadia_slug_for_launchbox("Sony Playstation"), Some("ps1"));
        assert_eq!(arcadia_slug_for_launchbox("Sega Pico"), None);
    }

    const SAMPLE_XML: &str = r#"<?xml version="1.0"?>
<LaunchBox>
  <Game>
    <Name>Super Mario World</Name>
    <DatabaseID>10</DatabaseID>
    <Platform>Super Nintendo Entertainment System</Platform>
  </Game>
  <Game>
    <Name>Sega Pico Game</Name>
    <DatabaseID>20</DatabaseID>
    <Platform>Sega Pico</Platform>
  </Game>
  <GameAlternateName>
    <AlternateName>Super Mario Bros 4</AlternateName>
    <DatabaseID>10</DatabaseID>
  </GameAlternateName>
  <GameImage>
    <DatabaseID>10</DatabaseID>
    <FileName>a/b/Super Mario World-01.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>Japan</Region>
  </GameImage>
  <GameImage>
    <DatabaseID>10</DatabaseID>
    <FileName>a/b/Super Mario World-NA.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>North America</Region>
  </GameImage>
  <GameImage>
    <DatabaseID>10</DatabaseID>
    <FileName>a/b/Super Mario World-screen.jpg</FileName>
    <Type>Screenshot - Gameplay</Type>
    <Region>North America</Region>
  </GameImage>
  <GameImage>
    <DatabaseID>20</DatabaseID>
    <FileName>x/y/pico-01.jpg</FileName>
    <Type>Box - Front</Type>
    <Region>Japan</Region>
  </GameImage>
</LaunchBox>"#;

    #[test]
    fn parse_picks_box_front_prefers_na_and_includes_alt_names() {
        let mut rows = parse_metadata_xml(std::io::Cursor::new(SAMPLE_XML));
        rows.sort_by(|a, b| (a.platform.clone(), a.norm_name.clone()).cmp(&(b.platform.clone(), b.norm_name.clone())));

        // Sega Pico is unmapped -> dropped entirely (no rows for db 20).
        assert!(rows.iter().all(|r| r.platform == "snes"));

        // Both the primary name and the alternate name map to the NA box-front.
        let main = rows.iter().find(|r| r.norm_name == "super mario world").unwrap();
        assert_eq!(main.file_name, "a/b/Super Mario World-NA.jpg");
        let alt = rows.iter().find(|r| r.norm_name == "super mario bros 4").unwrap();
        assert_eq!(alt.file_name, "a/b/Super Mario World-NA.jpg");

        // Exactly two rows (main + alt), no screenshot, no Pico.
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn image_url_percent_encodes_segments_but_keeps_slashes() {
        assert_eq!(
            image_url("a/b/Super Mario World-NA.jpg"),
            "https://images.launchbox-app.com/a/b/Super%20Mario%20World-NA.jpg"
        );
    }

    #[tokio::test]
    async fn index_build_and_lookup_roundtrip() {
        let dir = std::env::temp_dir().join(format!("arcadia_lb_idx_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("index.sqlite");

        let rows = vec![
            IndexRow { platform: "snes".into(), norm_name: "super mario world".into(), file_name: "a/b/smw.jpg".into() },
            IndexRow { platform: "snes".into(), norm_name: "super mario bros 4".into(), file_name: "a/b/smw.jpg".into() },
        ];
        let n = build_index_db(&db, &rows).await.unwrap();
        assert_eq!(n, 2);

        let pool = open_index_pool(&db).await.unwrap();
        assert_eq!(lookup_cover(&pool, "snes", "super mario world").await.as_deref(), Some("a/b/smw.jpg"));
        assert_eq!(lookup_cover(&pool, "snes", "super mario bros 4").await.as_deref(), Some("a/b/smw.jpg"));
        assert_eq!(lookup_cover(&pool, "snes", "unknown game").await, None);
        assert_eq!(lookup_cover(&pool, "n64", "super mario world").await, None);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
