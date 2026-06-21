# LaunchBox Cover Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the LaunchBox Games Database as an opt-in, fallback-after-libretro source of box-art covers, sourced from the sanctioned `Metadata.zip` dump and indexed into a compact sidecar SQLite file.

**Architecture:** A new `core/dreamvault/src/launchbox.rs` module owns everything: a streaming XML parser over the dump, a sidecar SQLite index keyed by `(arcadia-platform, normalized-name) → image file path`, and a `LaunchBoxProvider` implementing the existing `MetadataProvider` trait. A user-triggered `refresh_launchbox_index` is the *only* thing that ever downloads the dump. The metadata enrichment pass runs the provider as a third pass after libretro, but only when the feature is enabled **and** an index already exists — enrichment itself never downloads.

**Tech Stack:** Rust, `quick-xml` (streaming XML), `zip` (already a dep), `sqlx`/SQLite (already a dep), `reqwest` (already a dep), `tempfile` (already a dep); TypeScript/React + Tauri for the UI.

## Global Constraints

- Cover images **only**. Never write text fields (description/genre/etc.) from this source — that stays ScreenScraper's job.
- Default **disabled**. When disabled the provider is a hard no-op: zero network.
- The dump downloads **only** from the explicit `refresh_launchbox_index` path (the Settings "Refresh database" button). The enrichment pass must never trigger a download.
- Sanctioned endpoints, copied verbatim:
  - Dump: `https://gamesdb.launchbox-app.com/Metadata.zip`
  - Image base: `https://images.launchbox-app.com`
- HTTP client user agent: `Arcadia/0.1 (+https://github.com/arcadia-project/arcadia)` (match existing providers).
- Best-effort everywhere: download/unzip/parse failures log at `debug`/`warn` and leave covers blank; they must never crash enrichment.
- All Rust changes must keep `cargo test -p dreamvault` green.

---

### Task 1: Config — `LaunchBoxConfig` and the `quick-xml` dependency

**Files:**
- Modify: `Cargo.toml` (workspace `[workspace.dependencies]`)
- Modify: `core/dreamvault/Cargo.toml` (`[dependencies]`)
- Modify: `core/dreamvault/src/config.rs`

**Interfaces:**
- Produces: `LaunchBoxConfig { enabled: bool, last_refresh: Option<i64> }` with `fn is_enabled(&self) -> bool`; new field `AppConfig.launchbox: LaunchBoxConfig`.

- [ ] **Step 1: Add `quick-xml` to the workspace dependencies**

In `Cargo.toml`, under `[workspace.dependencies]` (near the `zip` line), add:

```toml
# Streaming XML reader for the LaunchBox metadata dump (multi-GB; must not be
# loaded whole into memory). Pure Rust, no C codecs.
quick-xml = "0.36"
```

- [ ] **Step 2: Reference it from the crate**

In `core/dreamvault/Cargo.toml`, under `[dependencies]` (after the `zip.workspace = true` line), add:

```toml
quick-xml.workspace = true
```

- [ ] **Step 3: Write the failing config test**

In `core/dreamvault/src/config.rs`, inside `mod tests`, add:

```rust
#[test]
fn launchbox_config_defaults_disabled_and_round_trips() {
    let dir = std::env::temp_dir().join(format!("arcadia_cfg_lb_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let mut cfg = AppConfig::default();
    assert!(!cfg.launchbox.is_enabled());
    cfg.launchbox.enabled = true;
    cfg.launchbox.last_refresh = Some(1_700_000_000);
    cfg.save(&dir).unwrap();
    let loaded = AppConfig::load(&dir);
    assert!(loaded.launchbox.is_enabled());
    assert_eq!(loaded.launchbox.last_refresh, Some(1_700_000_000));
    let _ = std::fs::remove_dir_all(&dir);
}
```

- [ ] **Step 4: Run the test to verify it fails**

Run: `cargo test -p dreamvault launchbox_config_defaults -- --nocapture`
Expected: FAIL to compile — `no field launchbox on AppConfig`.

- [ ] **Step 5: Add the struct and field**

In `core/dreamvault/src/config.rs`, add the new field to `AppConfig` (after the `sync` field, around line 23):

```rust
    #[serde(default)]
    pub launchbox: LaunchBoxConfig,
```

And add the struct next to `ScreenScraperCredentials` (e.g. just before `impl AppConfig`):

```rust
/// LaunchBox Games Database cover source. Opt-in: disabled until the user
/// enables it and explicitly downloads the catalogue. `last_refresh` is the
/// unix-seconds timestamp of the last successful index build (None = never).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LaunchBoxConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub last_refresh: Option<i64>,
}

impl LaunchBoxConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }
}
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test -p dreamvault launchbox_config_defaults`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml core/dreamvault/Cargo.toml core/dreamvault/src/config.rs
git commit -m "feat(launchbox): add opt-in LaunchBoxConfig and quick-xml dep"
```

---

### Task 2: Name normalization and platform mapping

**Files:**
- Create: `core/dreamvault/src/launchbox.rs`
- Modify: `core/dreamvault/src/lib.rs` (register `mod launchbox;`)

**Interfaces:**
- Produces:
  - `pub(crate) fn normalize_name(s: &str) -> String`
  - `pub(crate) fn arcadia_slug_for_launchbox(name: &str) -> Option<&'static str>`
  - constants `pub const METADATA_URL` and `pub const IMAGE_BASE`

- [ ] **Step 1: Create the module file with the failing tests**

Create `core/dreamvault/src/launchbox.rs`:

```rust
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
}
```

> **NOTE for the implementer:** the exact LaunchBox `<Platform>` strings above are taken from community docs. During Task 6's first real `refresh`, dump the distinct `<Platform>` values you actually encounter (log them) and reconcile any mismatches against this map. A wrong string only means that platform is skipped, never a crash.

- [ ] **Step 2: Register the module**

In `core/dreamvault/src/lib.rs`, add alongside the other `mod` declarations (e.g. near `mod metadata;`):

```rust
mod launchbox;
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test -p dreamvault launchbox::`
Expected: PASS (2 tests).

- [ ] **Step 4: Commit**

```bash
git add core/dreamvault/src/launchbox.rs core/dreamvault/src/lib.rs
git commit -m "feat(launchbox): name normalization and platform mapping"
```

---

### Task 3: Streaming XML parser → index rows

**Files:**
- Modify: `core/dreamvault/src/launchbox.rs`

**Interfaces:**
- Consumes: `normalize_name`, `arcadia_slug_for_launchbox` (Task 2).
- Produces:
  - `pub(crate) struct IndexRow { pub platform: String, pub norm_name: String, pub file_name: String }`
  - `pub(crate) fn parse_metadata_xml<R: std::io::BufRead>(reader: R) -> Vec<IndexRow>`

- [ ] **Step 1: Write the failing fixture test**

In `core/dreamvault/src/launchbox.rs`, inside `mod tests`, add:

```rust
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p dreamvault parse_picks_box_front`
Expected: FAIL to compile — `parse_metadata_xml` not found.

- [ ] **Step 3: Implement the parser**

In `core/dreamvault/src/launchbox.rs`, add (above `#[cfg(test)]`):

```rust
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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p dreamvault parse_picks_box_front`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/dreamvault/src/launchbox.rs
git commit -m "feat(launchbox): streaming Metadata.xml parser to index rows"
```

---

### Task 4: Sidecar SQLite index — build and lookup

**Files:**
- Modify: `core/dreamvault/src/launchbox.rs`

**Interfaces:**
- Consumes: `IndexRow` (Task 3).
- Produces:
  - `pub(crate) async fn build_index_db(db_path: &std::path::Path, rows: &[IndexRow]) -> crate::error::Result<usize>`
  - `pub(crate) async fn open_index_pool(db_path: &std::path::Path) -> crate::error::Result<sqlx::SqlitePool>`
  - `pub(crate) async fn lookup_cover(pool: &sqlx::SqlitePool, platform: &str, norm_name: &str) -> Option<String>`

- [ ] **Step 1: Write the failing test**

In `core/dreamvault/src/launchbox.rs` `mod tests`, add:

```rust
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
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p dreamvault index_build_and_lookup`
Expected: FAIL to compile — functions not found.

- [ ] **Step 3: Implement build + lookup**

In `core/dreamvault/src/launchbox.rs`, add `use` lines at the top of the file (with the existing imports):

```rust
use crate::error::Result;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;
```

Then add the functions (above `#[cfg(test)]`):

```rust
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
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p dreamvault index_build_and_lookup`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add core/dreamvault/src/launchbox.rs
git commit -m "feat(launchbox): sidecar SQLite index build and lookup"
```

---

### Task 5: `LaunchBoxProvider` — URL building and `MetadataProvider` impl

**Files:**
- Modify: `core/dreamvault/src/launchbox.rs`
- Modify: `core/dreamvault/src/metadata.rs` (make trait/types reachable — confirm `MetadataProvider`, `MetadataPatch` are `pub`; they already are)

**Interfaces:**
- Consumes: `MetadataProvider`, `MetadataPatch` from `crate::metadata`; `lookup_cover`, `open_index_pool` (Task 4).
- Produces:
  - `pub struct LaunchBoxProvider` with `pub fn new(cache_root: PathBuf, pool: SqlitePool) -> Self`
  - `fn image_url(file_name: &str) -> String` (module-private; unit-tested)

- [ ] **Step 1: Write the failing URL test**

In `core/dreamvault/src/launchbox.rs` `mod tests`, add:

```rust
#[test]
fn image_url_percent_encodes_segments_but_keeps_slashes() {
    assert_eq!(
        image_url("a/b/Super Mario World-NA.jpg"),
        "https://images.launchbox-app.com/a/b/Super%20Mario%20World-NA.jpg"
    );
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p dreamvault image_url_percent_encodes`
Expected: FAIL to compile — `image_url` not found.

- [ ] **Step 3: Implement the provider**

In `core/dreamvault/src/launchbox.rs`, add these imports to the top:

```rust
use crate::metadata::{MetadataPatch, MetadataProvider};
use crate::models::Game;
use async_trait::async_trait;
use std::path::PathBuf;
```

Add the URL helper and provider (above `#[cfg(test)]`):

```rust
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
```

- [ ] **Step 4: Run the test to verify it passes (and the crate compiles)**

Run: `cargo test -p dreamvault image_url_percent_encodes`
Expected: PASS. If the compiler complains that `MetadataProvider`/`MetadataPatch` aren't visible, confirm they are `pub` in `metadata.rs` (they are at lines ~18 and ~42) — no change should be needed.

- [ ] **Step 5: Commit**

```bash
git add core/dreamvault/src/launchbox.rs
git commit -m "feat(launchbox): LaunchBoxProvider with image URL build and cache"
```

---

### Task 6: Index refresh lifecycle on `Engine` (the only downloader)

**Files:**
- Modify: `core/dreamvault/src/launchbox.rs` (add `impl Engine` block)
- Modify: `core/dreamvault/src/lib.rs` (only if an import is needed; otherwise none)

**Interfaces:**
- Consumes: `parse_metadata_xml`, `build_index_db`, `open_index_pool` (Tasks 3-4); `Engine` (`self.paths`, `self.load_config`, `self.save_config`).
- Produces (on `impl Engine`):
  - `pub fn launchbox_index_path(&self) -> std::path::PathBuf`
  - `pub async fn refresh_launchbox_index(&self) -> crate::error::Result<usize>`
  - `pub(crate) async fn launchbox_provider(&self) -> Option<LaunchBoxProvider>`

- [ ] **Step 1: Add the `impl Engine` block**

In `core/dreamvault/src/launchbox.rs`, add at the bottom (above `#[cfg(test)]`), with `use crate::Engine;` added to the imports:

```rust
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
        let dir = index_path.parent().unwrap().to_path_buf();
        std::fs::create_dir_all(&dir)?;

        // 1. Stream the (hundreds of MB) zip to a temp file on disk — the zip
        //    central directory is at the end, so we need Seek to read entries.
        tracing::info!("downloading LaunchBox metadata dump");
        let client = reqwest::Client::builder()
            .user_agent("Arcadia/0.1 (+https://github.com/arcadia-project/arcadia)")
            .build()
            .unwrap_or_default();
        let resp = client.get(METADATA_URL).send().await?.error_for_status()?;
        let bytes = resp.bytes().await?;
        let mut zip_tmp = tempfile::Builder::new().prefix("launchbox-").suffix(".zip").tempfile_in(&dir)?;
        std::io::Write::write_all(&mut zip_tmp, &bytes)?;
        drop(bytes);

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
```

- [ ] **Step 2: Verify the crate compiles**

Run: `cargo build -p dreamvault`
Expected: builds clean. (No new unit test here — `refresh_launchbox_index` does live network/IO and is covered by manual verification; its pure pieces are already tested in Tasks 3-4.)

- [ ] **Step 3: Commit**

```bash
git add core/dreamvault/src/launchbox.rs
git commit -m "feat(launchbox): refresh_launchbox_index download + atomic index swap"
```

---

### Task 7: Wire the third enrichment pass

**Files:**
- Modify: `core/dreamvault/src/metadata.rs` (`enrich_metadata_online`, around lines 935-940)

**Interfaces:**
- Consumes: `Engine::launchbox_provider` (Task 6); existing `apply_patch`, `emit_progress`.

- [ ] **Step 1: Add the pass after the libretro loop, before the text pass**

In `core/dreamvault/src/metadata.rs`, in `enrich_metadata_online`, locate the block right after the libretro loop emits "Box art complete" (currently line ~935) and before `self.enrich_text_metadata(...)` (line ~939). Insert:

```rust
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

```

> The `MetadataProvider` trait is already in scope in this file. `Arc`, `Semaphore`, `Game`, `JoinSet` are already imported/used by the libretro loop above.

- [ ] **Step 2: Verify the crate builds and tests pass**

Run: `cargo test -p dreamvault`
Expected: builds clean; all existing + new tests PASS.

- [ ] **Step 3: Commit**

```bash
git add core/dreamvault/src/metadata.rs
git commit -m "feat(launchbox): run cover provider as third enrichment pass"
```

---

### Task 8: Tauri commands

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs`
- Modify: `desktop/src-tauri/src/main.rs` (register handlers)

**Interfaces:**
- Consumes: `Engine::refresh_launchbox_index`, `load_config`, `save_config`; `LaunchBoxConfig`.
- Produces commands: `launchbox_config`, `set_launchbox_enabled`, `refresh_launchbox_index`.

- [ ] **Step 1: Add the commands**

In `desktop/src-tauri/src/commands.rs`, after `set_screenscraper_credentials` (line ~166), add (and ensure `LaunchBoxConfig` is imported from `dreamvault::config` alongside `ScreenScraperCredentials`):

```rust
/// The user's LaunchBox cover settings (enable flag + last refresh time).
#[tauri::command]
pub fn launchbox_config(state: State<'_, AppState>) -> LaunchBoxConfig {
    state.engine.load_config().launchbox
}

/// Toggle the LaunchBox cover source on/off. Does not download anything.
#[tauri::command]
pub fn set_launchbox_enabled(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    let mut cfg = state.engine.load_config();
    cfg.launchbox.enabled = enabled;
    map(state.engine.save_config(&cfg))
}

/// Download the LaunchBox metadata dump and rebuild the cover index. Heavy
/// (hundreds of MB). Returns the number of indexed entries.
#[tauri::command]
pub async fn refresh_launchbox_index(state: State<'_, AppState>) -> CmdResult<usize> {
    map(state.engine.refresh_launchbox_index().await)
}
```

Check the existing import line near the top of `commands.rs` (it imports `ScreenScraperCredentials`) and extend it, e.g.:

```rust
use dreamvault::config::{LaunchBoxConfig, ScreenScraperCredentials};
```

(If the existing import path differs, match it — just add `LaunchBoxConfig` to the same group.)

- [ ] **Step 2: Register the handlers**

In `desktop/src-tauri/src/main.rs`, inside `tauri::generate_handler![ ... ]` (after `commands::set_screenscraper_credentials,` at line ~120), add:

```rust
            commands::launchbox_config,
            commands::set_launchbox_enabled,
            commands::refresh_launchbox_index,
```

- [ ] **Step 3: Verify the desktop crate builds**

Run: `cargo build -p dreamshell --features custom-protocol`
Expected: builds clean.

- [ ] **Step 4: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/main.rs
git commit -m "feat(launchbox): tauri commands for config and index refresh"
```

---

### Task 9: Frontend — types, API, and Settings UI

**Files:**
- Modify: `desktop/src/api/types.ts`
- Modify: `desktop/src/api/commands.ts`
- Modify: `desktop/src/views/Settings.tsx`

**Interfaces:**
- Consumes: commands from Task 8.

- [ ] **Step 1: Add the type**

In `desktop/src/api/types.ts`, near `ScreenScraperCredentials` (line ~163), add:

```typescript
export interface LaunchBoxConfig {
  enabled: boolean;
  last_refresh: number | null;
}
```

- [ ] **Step 2: Add the API methods**

In `desktop/src/api/commands.ts`, in the "Configuration" group (after `setScreenScraperCredentials`, line ~76), add — and add `LaunchBoxConfig` to the type import from `./types`:

```typescript
  launchBoxConfig: () => invoke<LaunchBoxConfig>("launchbox_config"),
  setLaunchBoxEnabled: (enabled: boolean) =>
    invoke<void>("set_launchbox_enabled", { enabled }),
  refreshLaunchBoxIndex: () => invoke<number>("refresh_launchbox_index"),
```

- [ ] **Step 3: Add the Settings section**

In `desktop/src/views/Settings.tsx`, mirror the existing ScreenScraper section. Add state and handlers near the other config state:

```typescript
const [launchbox, setLaunchbox] = useState<LaunchBoxConfig | null>(null);
const [lbRefreshing, setLbRefreshing] = useState(false);

useEffect(() => {
  api.launchBoxConfig().then(setLaunchbox).catch(() => {});
}, []);

const toggleLaunchbox = async (enabled: boolean) => {
  await api.setLaunchBoxEnabled(enabled);
  setLaunchbox((c) => (c ? { ...c, enabled } : c));
};

const refreshLaunchbox = async () => {
  setLbRefreshing(true);
  try {
    await api.refreshLaunchBoxIndex();
    setLaunchbox(await api.launchBoxConfig());
  } finally {
    setLbRefreshing(false);
  }
};
```

Add the section markup within the settings layout (match the surrounding component/class conventions used by the ScreenScraper block — reuse the same card/toggle/button components rather than raw elements):

```tsx
<section>
  <h2>LaunchBox Games Database</h2>
  <p>
    Optional fallback source for box-art covers, used only for games the
    primary source can't match. Enabling it does nothing until you download
    the catalogue below — a one-time download of several hundred megabytes.
  </p>
  <label>
    <input
      type="checkbox"
      checked={launchbox?.enabled ?? false}
      onChange={(e) => toggleLaunchbox(e.target.checked)}
    />
    Enable LaunchBox covers
  </label>
  <button onClick={refreshLaunchbox} disabled={lbRefreshing || !launchbox?.enabled}>
    {lbRefreshing ? "Downloading…" : "Refresh database (several hundred MB)"}
  </button>
  {launchbox?.last_refresh && (
    <p>Last updated: {new Date(launchbox.last_refresh * 1000).toLocaleString()}</p>
  )}
</section>
```

Add `LaunchBoxConfig` to the type import at the top of `Settings.tsx`.

- [ ] **Step 4: Type-check and build the frontend**

Run: `cd desktop && npm run build`
Expected: builds with no type errors.

- [ ] **Step 5: Commit**

```bash
git add desktop/src/api/types.ts desktop/src/api/commands.ts desktop/src/views/Settings.tsx
git commit -m "feat(launchbox): settings UI to enable and refresh cover database"
```

---

## Manual verification (after Task 9)

Per the project's interactive-test etiquette, do these with the user present:

1. Build + install: `cargo build -p dreamshell --release --features custom-protocol`, then atomic-install to `/usr/bin/arcadia` (temp + `mv -f`).
2. Settings → enable LaunchBox → click "Refresh database". Confirm it downloads, indexes, and shows a "Last updated" time. Check the log line reporting indexed count and any unmapped `<Platform>` values (reconcile Task 2's map if needed).
3. Find a game with no cover that libretro couldn't match; run metadata enrichment; confirm a LaunchBox cover appears.
4. Disable the toggle; confirm enrichment makes zero LaunchBox network calls.

## Self-review notes

- **Spec coverage:** opt-in config (T1), sidecar index (T4), streaming dump parse (T3), platform map + alt-name matching (T2/T3), box-front + region selection (T3), provider + URL build + cache (T5), refresh-only download + atomic swap (T6), fallback-after-libretro orchestration (T7), commands + UI with size warning (T8/T9), best-effort error handling (throughout), unit tests mirroring the screenscraper style (T2-T5). All spec sections map to a task.
- **Deviation from spec (intentional, safer):** the spec said the enrichment pass would "build the index if needed." This plan makes **refresh the only downloader**; enrichment runs the provider only if an index already exists, so merely running enrichment can never trigger a multi-hundred-MB download. This better honors the "explicit opt-in trigger" decision. Surface this to the user at review.
