# LaunchBox + Arcade in the Cover "Find" Search — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the interactive per-game "Find" cover search query the LaunchBox index (when downloaded) alongside libretro, present both sources' matches tagged by origin, and add arcade/MAME coverage to both sources.

**Architecture:** `Engine::suggest_covers` returns tagged `CoverCandidate`s (libretro first, then LaunchBox via `search_covers` — gated on `launchbox_provider()` being `Some`). A new `apply_cover(game_id, source, token)` engine method + Tauri command routes a chosen candidate to either the libretro or LaunchBox download path, replacing the now-dead `refetch_cover` end to end. Arcade is added to both platform maps.

**Tech Stack:** Rust (`core/dreamvault`), `sqlx`/SQLite sidecar index, `async_trait` providers, Tauri commands, React + TypeScript + `@tanstack/react-query` frontend.

## Global Constraints

- Cover-image only; text metadata stays ScreenScraper's job. Patches from these paths set `cover_art` and nothing else.
- LaunchBox is opt-in AND refresh-is-the-only-downloader: never download the dump from Find/apply paths. `launchbox_provider()` returns `Some` only when the feature is enabled AND an index file already exists.
- Libretro is the primary source: libretro candidates are listed first, LaunchBox candidates follow.
- Best-effort everywhere: a missing/disabled LaunchBox index makes Find behave exactly as today (libretro only), no error surfaced. A failed download yields an empty patch.
- `CoverCandidate` field names are single words so snake_case == camelCase across the Rust/TS IPC boundary: `source`, `label`, `token`.
- `source` is exactly `"libretro"` or `"launchbox"`.
- No MAME setname→full-title resolution (auto-scan arcade matching stays out of scope). Only the Find/apply paths and the two platform-map additions change behavior.
- No in-place DB migration: the new `name` column appears on the next "Refresh database" (refresh rebuilds the whole index).
- Create NEW commits, never amend. Never `--no-verify`. Never `git checkout`/`switch` to another branch during execution.

---

### Task 1: Platform maps — add arcade

**Files:**
- Modify: `core/dreamvault/src/launchbox.rs:29` (`arcadia_slug_for_launchbox`) and its test at `:491`
- Modify: `core/dreamvault/src/metadata.rs:169` (`system_name`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: arcade now maps to `"arcade"` (LaunchBox) and `"MAME"` (libretro). No new public symbols.

- [ ] **Step 1: Add a failing assertion to the LaunchBox platform-map test**

In `core/dreamvault/src/launchbox.rs`, edit the existing test `platform_mapping_known_and_unknown` (line ~490) to add an arcade case:

```rust
    #[test]
    fn platform_mapping_known_and_unknown() {
        assert_eq!(arcadia_slug_for_launchbox("Super Nintendo Entertainment System"), Some("snes"));
        assert_eq!(arcadia_slug_for_launchbox("Sony Playstation"), Some("ps1"));
        assert_eq!(arcadia_slug_for_launchbox("Arcade"), Some("arcade"));
        assert_eq!(arcadia_slug_for_launchbox("Sega Pico"), None);
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test -p dreamvault platform_mapping_known_and_unknown`
Expected: FAIL — `assertion failed: left == right`, left `None`, right `Some("arcade")`.

- [ ] **Step 3: Add the arcade entry to the LaunchBox map**

In `core/dreamvault/src/launchbox.rs`, in `arcadia_slug_for_launchbox`, add a line in the match (e.g. after the `"WonderSwan" => "wonderswan",` line at :64):

```rust
        "Arcade" => "arcade",
```

- [ ] **Step 4: Run it to verify it passes**

Run: `cargo test -p dreamvault platform_mapping_known_and_unknown`
Expected: PASS.

- [ ] **Step 5: Add a failing test for the libretro system_name arcade mapping**

`system_name` is a private associated fn on `LibretroThumbnailProvider`. Add a test inside the existing `#[cfg(test)] mod tests` in `core/dreamvault/src/metadata.rs` (it already has `use super::*;`):

```rust
    #[test]
    fn libretro_system_name_maps_arcade_to_mame() {
        assert_eq!(LibretroThumbnailProvider::system_name("arcade"), Some("MAME"));
        assert_eq!(LibretroThumbnailProvider::system_name("snes"), Some("Nintendo - Super Nintendo Entertainment System"));
        assert_eq!(LibretroThumbnailProvider::system_name("not_a_platform"), None);
    }
```

- [ ] **Step 6: Run it to verify it fails**

Run: `cargo test -p dreamvault libretro_system_name_maps_arcade_to_mame`
Expected: FAIL — left `None`, right `Some("MAME")`.

- [ ] **Step 7: Add the arcade entry to the libretro map**

In `core/dreamvault/src/metadata.rs`, in `system_name`, add a line in the match (e.g. after `"wonderswan" => "Bandai - WonderSwan",` at :205):

```rust
            "arcade" => "MAME",
```

- [ ] **Step 8: Run it to verify it passes**

Run: `cargo test -p dreamvault libretro_system_name_maps_arcade_to_mame`
Expected: PASS.

- [ ] **Step 9: Commit**

```bash
git add core/dreamvault/src/launchbox.rs core/dreamvault/src/metadata.rs
git commit -m "feat(covers): map arcade platform in libretro and launchbox"
```

---

### Task 2: LaunchBox index gains a display `name` column + `search_covers`

**Files:**
- Modify: `core/dreamvault/src/launchbox.rs` — `IndexRow` (:82), `parse_metadata_xml` (:111), `build_index_db` (:254), tests (:540, :566)
- Modify: `core/dreamvault/src/metadata.rs` — make `normalize_tokens` (:410) and `match_score` (:421) `pub(crate)`

**Interfaces:**
- Consumes: `arcadia_slug_for_launchbox` (Task 1), `normalize_name` (existing).
- Produces:
  - `IndexRow { platform: String, norm_name: String, name: String, file_name: String }`
  - `pub(crate) async fn search_covers(pool: &SqlitePool, platform: &str, query: &str, limit: usize) -> Vec<(String /*name*/, String /*file_name*/)>`
  - `pub(crate) fn normalize_tokens(s: &str) -> Vec<String>` and `pub(crate) fn match_score(qt: &[String], qjoined: &str, name: &str) -> f64` in `metadata.rs`.

- [ ] **Step 1: Make the libretro scorers reusable**

In `core/dreamvault/src/metadata.rs`, change the two free functions' visibility (only the `fn` keyword line changes):

```rust
/// Lowercase, split on any non-alphanumeric, drop empties.
pub(crate) fn normalize_tokens(s: &str) -> Vec<String> {
```

```rust
/// Token-overlap score with a contiguous-substring bonus. Higher is closer.
pub(crate) fn match_score(qt: &[String], qjoined: &str, name: &str) -> f64 {
```

- [ ] **Step 2: Verify the crate still compiles**

Run: `cargo build -p dreamvault`
Expected: builds (visibility widening is non-breaking).

- [ ] **Step 3: Add the `name` field to `IndexRow`**

In `core/dreamvault/src/launchbox.rs`, update the struct (:82):

```rust
#[derive(Debug, Clone)]
pub(crate) struct IndexRow {
    pub platform: String,
    pub norm_name: String,
    pub name: String,
    pub file_name: String,
}
```

- [ ] **Step 4: Carry original names through the parser**

In `parse_metadata_xml`, the `game_names` map currently stores only normalized names. Change it to store `(norm, original)` pairs so each row can keep its own display name.

Change the declaration (:128):

```rust
    // db_id -> list of (normalized name, original name) for primary + alternates
    let mut game_names: HashMap<String, Vec<(String, String)>> = HashMap::new();
```

In the `"Game"` end-tag arm (:185), push the pair:

```rust
                                game_names
                                    .entry(g_id.clone())
                                    .or_default()
                                    .push((normalize_name(&g_name), g_name.trim().to_string()));
```

In the `"GameAlternateName"` end-tag arm (:195), push the pair:

```rust
                            game_names
                                .entry(a_id.clone())
                                .or_default()
                                .push((normalize_name(&a_name), a_name.trim().to_string()));
```

In the join loop at the bottom (:234), destructure the pair and set `name`:

```rust
        for (norm, original) in names {
            if norm.is_empty() {
                continue;
            }
            let key = (platform.clone(), norm.clone());
            if seen.insert(key) {
                rows.push(IndexRow {
                    platform: platform.clone(),
                    norm_name: norm.clone(),
                    name: original.clone(),
                    file_name: file_name.clone(),
                });
            }
        }
```

- [ ] **Step 5: Add the `name` column to the schema and insert**

In `build_index_db`, update the CREATE TABLE (:263) and INSERT (:270):

```rust
    sqlx::query("CREATE TABLE covers (platform TEXT NOT NULL, norm_name TEXT NOT NULL, name TEXT NOT NULL, file_name TEXT NOT NULL, PRIMARY KEY (platform, norm_name))")
        .execute(&pool)
        .await?;
```

```rust
        let res = sqlx::query("INSERT OR IGNORE INTO covers (platform, norm_name, name, file_name) VALUES (?, ?, ?, ?)")
            .bind(&row.platform)
            .bind(&row.norm_name)
            .bind(&row.name)
            .bind(&row.file_name)
            .execute(&mut *tx)
            .await?;
```

- [ ] **Step 6: Add `search_covers`**

In `core/dreamvault/src/launchbox.rs`, add after `lookup_cover` (:301):

```rust
/// Fuzzy-search the index for a platform. Prefilters candidates with a SQL LIKE
/// on the longest normalized query token (the most selective), then ranks them
/// in Rust with the same token-overlap scoring libretro's suggest uses. Returns
/// up to `limit` (display name, image file path) pairs, best first.
pub(crate) async fn search_covers(
    pool: &SqlitePool,
    platform: &str,
    query: &str,
    limit: usize,
) -> Vec<(String, String)> {
    let qt = crate::metadata::normalize_tokens(query);
    if qt.is_empty() {
        return Vec::new();
    }
    let qjoined = qt.join(" ");
    let longest = qt.iter().max_by_key(|t| t.len()).cloned().unwrap_or_default();
    let like = format!("%{longest}%");
    let candidates: Vec<(String, String)> = sqlx::query_as::<_, (String, String)>(
        "SELECT name, file_name FROM covers WHERE platform = ? AND norm_name LIKE ? LIMIT 5000",
    )
    .bind(platform)
    .bind(&like)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let mut scored: Vec<(f64, String, String)> = candidates
        .into_iter()
        .map(|(name, file_name)| (crate::metadata::match_score(&qt, &qjoined, &name), name, file_name))
        .filter(|(s, _, _)| *s > 0.0)
        .collect();
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.len().cmp(&b.1.len()))
    });
    scored
        .into_iter()
        .take(limit)
        .map(|(_, name, file_name)| (name, file_name))
        .collect()
}
```

- [ ] **Step 7: Fix the existing roundtrip test fixtures + add a search test**

In `core/dreamvault/src/launchbox.rs` tests, update `index_build_and_lookup_roundtrip` (:572) to include `name` on each `IndexRow`:

```rust
        let rows = vec![
            IndexRow { platform: "snes".into(), norm_name: "super mario world".into(), name: "Super Mario World".into(), file_name: "a/b/smw.jpg".into() },
            IndexRow { platform: "snes".into(), norm_name: "super mario bros 4".into(), name: "Super Mario Bros. 4".into(), file_name: "a/b/smw.jpg".into() },
        ];
```

Then add a new test in the same module:

```rust
    #[tokio::test]
    async fn search_covers_ranks_token_overlap() {
        let dir = std::env::temp_dir().join(format!("arcadia_lb_search_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("index.sqlite");

        let rows = vec![
            IndexRow { platform: "arcade".into(), norm_name: "street fighter ii".into(), name: "Street Fighter II".into(), file_name: "s/sf2.jpg".into() },
            IndexRow { platform: "arcade".into(), norm_name: "street fighter alpha".into(), name: "Street Fighter Alpha".into(), file_name: "s/sfa.jpg".into() },
            IndexRow { platform: "arcade".into(), norm_name: "metal slug".into(), name: "Metal Slug".into(), file_name: "m/ms.jpg".into() },
        ];
        build_index_db(&db, &rows).await.unwrap();
        let pool = open_index_pool(&db).await.unwrap();

        let hits = search_covers(&pool, "arcade", "street fighter ii", 12).await;
        assert_eq!(hits.first().map(|(n, _)| n.as_str()), Some("Street Fighter II"));
        assert_eq!(hits.first().map(|(_, f)| f.as_str()), Some("s/sf2.jpg"));
        // "metal slug" shares no token with the query and is filtered out.
        assert!(hits.iter().all(|(n, _)| n != "Metal Slug"));

        // Wrong platform returns nothing.
        assert!(search_covers(&pool, "snes", "street fighter ii", 12).await.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 8: Run the launchbox tests to verify they pass**

Run: `cargo test -p dreamvault launchbox`
Expected: PASS — including `parse_picks_box_front_prefers_na_and_includes_alt_names`, `index_build_and_lookup_roundtrip`, and `search_covers_ranks_token_overlap`.

- [ ] **Step 9: Commit**

```bash
git add core/dreamvault/src/launchbox.rs core/dreamvault/src/metadata.rs
git commit -m "feat(launchbox): store display name and add fuzzy search_covers"
```

---

### Task 3: `LaunchBoxProvider::fetch_file` (refactor `fetch` to reuse it)

**Files:**
- Modify: `core/dreamvault/src/launchbox.rs` — `LaunchBoxProvider` impl (:329), `fetch` (:366), tests

**Interfaces:**
- Consumes: `image_url` (existing), `cache_path` (existing private method), `lookup_cover` (existing).
- Produces: `pub(crate) async fn fetch_file(&self, game: &Game, file_name: &str) -> MetadataPatch` on `LaunchBoxProvider` — downloads (or returns cached) a specific image, yielding a cover-only patch (empty on failure).

- [ ] **Step 1: Add a `sample_game` test fixture + a cache-hit test**

`fetch_file` doesn't exist yet, so this test won't compile — that is the failing state. Add to the `#[cfg(test)] mod tests` in `core/dreamvault/src/launchbox.rs`:

```rust
    fn sample_game(platform: &str, rom_path: &str, title: &str) -> Game {
        Game {
            id: "g1".into(),
            profile_id: "p1".into(),
            title: title.into(),
            custom_title: None,
            sort_title: title.to_ascii_lowercase(),
            platform: platform.into(),
            rom_path: rom_path.into(),
            file_size: 0,
            emulator_id: None,
            cover_art: None,
            background_art: None,
            description: None,
            genre: None,
            developer: None,
            publisher: None,
            release_date: None,
            playtime_minutes: 0,
            launch_count: 0,
            favorite: false,
            last_played: None,
            added_at: chrono::Utc::now(),
        }
    }

    #[tokio::test]
    async fn fetch_file_returns_cached_without_network() {
        let dir = std::env::temp_dir().join(format!("arcadia_lb_fetchfile_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("index.sqlite");
        build_index_db(&db, &[]).await.unwrap();
        let pool = open_index_pool(&db).await.unwrap();

        let cache_root = dir.join("art");
        let provider = LaunchBoxProvider::new(cache_root, pool);
        let game = sample_game("snes", "/roms/snes/Super Mario World.sfc", "Super Mario World");

        // Pre-create the exact cache file the provider will compute, so no
        // network call happens.
        let cached = provider.cache_path(&game, "a/b/smw.jpg");
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, b"img").unwrap();

        let patch = provider.fetch_file(&game, "a/b/smw.jpg").await;
        assert_eq!(patch.cover_art.as_deref(), Some(cached.to_string_lossy().as_ref()));

        let _ = std::fs::remove_dir_all(&dir);
    }
```

- [ ] **Step 2: Run it to verify it fails to compile**

Run: `cargo test -p dreamvault fetch_file_returns_cached_without_network`
Expected: FAIL — `no method named fetch_file found for struct LaunchBoxProvider`.

- [ ] **Step 3: Add `fetch_file` and refactor `fetch`**

In `core/dreamvault/src/launchbox.rs`, add `fetch_file` to the inherent `impl LaunchBoxProvider` block (alongside `cache_path`, before the closing brace at :358):

```rust
    /// Download a specific LaunchBox image `file_name` for `game`, caching it
    /// under the artwork dir. Returns a cover-only patch (empty if the download
    /// fails). Used by both auto-scan `fetch` and the interactive apply path.
    pub(crate) async fn fetch_file(&self, game: &Game, file_name: &str) -> MetadataPatch {
        let mut patch = MetadataPatch::default();
        let cached = self.cache_path(game, file_name);
        if cached.is_file() {
            patch.cover_art = Some(cached.to_string_lossy().to_string());
            return patch;
        }

        let url = image_url(file_name);
        let resp = match self.client.get(&url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                tracing::debug!(%url, status = %r.status(), "launchbox image miss");
                return patch;
            }
            Err(e) => {
                tracing::debug!(%url, error = %e, "launchbox image request failed");
                return patch;
            }
        };
        let Ok(bytes) = resp.bytes().await else { return patch };
        if let Some(parent) = cached.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if std::fs::write(&cached, &bytes).is_ok() {
            tracing::info!(game = %game.title, "fetched box art from launchbox");
            patch.cover_art = Some(cached.to_string_lossy().to_string());
        }
        patch
    }
```

Then replace the body of the trait `fetch` (:366) to reuse it:

```rust
    async fn fetch(&self, game: &Game) -> Result<MetadataPatch> {
        let norm = normalize_name(&game.title);
        let Some(file_name) = lookup_cover(&self.pool, &game.platform, &norm).await else {
            return Ok(MetadataPatch::default());
        };
        Ok(self.fetch_file(game, &file_name).await)
    }
```

- [ ] **Step 4: Run the launchbox tests to verify they pass**

Run: `cargo test -p dreamvault launchbox`
Expected: PASS — including `fetch_file_returns_cached_without_network`.

- [ ] **Step 5: Commit**

```bash
git add core/dreamvault/src/launchbox.rs
git commit -m "feat(launchbox): add fetch_file and route auto-scan fetch through it"
```

---

### Task 4: Engine `suggest_covers` → tagged candidates + `apply_cover`; remove `refetch_cover`

**Files:**
- Modify: `core/dreamvault/src/metadata.rs` — imports (:7-14), add `CoverCandidate` + `rank_cover_candidates`, rewrite `suggest_covers` (:1081), replace `refetch_cover` (:1060) with `apply_cover`, tests

**Interfaces:**
- Consumes: `LaunchBoxProvider::fetch_file` (Task 3), `launchbox::search_covers` (Task 2), `Engine::launchbox_provider()` (existing), `LibretroThumbnailProvider::{suggest, fetch_named}` (existing).
- Produces:
  - `pub struct CoverCandidate { pub source: String, pub label: String, pub token: String }` (`#[derive(Debug, Clone, Serialize)]`)
  - `pub(crate) fn rank_cover_candidates(libretro: Vec<String>, launchbox: Vec<(String, String)>) -> Vec<CoverCandidate>`
  - `pub async fn suggest_covers(&self, game_id: &str, query: Option<&str>) -> Result<Vec<CoverCandidate>>`
  - `pub async fn apply_cover(&self, game_id: &str, source: &str, token: &str) -> Result<bool>`
  - `refetch_cover` is removed.

- [ ] **Step 1: Add the `Serialize` import**

In `core/dreamvault/src/metadata.rs`, add to the imports near the top (after :11 `use async_trait::async_trait;`):

```rust
use serde::Serialize;
```

- [ ] **Step 2: Add `CoverCandidate` and the pure ranking helper, plus a unit test**

Add the type + helper near `MetadataPatch` (after its `impl` block, ~:39) in `core/dreamvault/src/metadata.rs`:

```rust
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
```

Add a test inside `#[cfg(test)] mod tests`:

```rust
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
```

- [ ] **Step 3: Run it to verify it passes**

Run: `cargo test -p dreamvault rank_cover_candidates_lists_libretro_first_and_tags_sources`
Expected: PASS.

- [ ] **Step 4: Rewrite `suggest_covers` to merge both sources**

Replace `suggest_covers` (:1081) in `core/dreamvault/src/metadata.rs`:

```rust
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
```

- [ ] **Step 5: Add a thin `search` method on `LaunchBoxProvider`**

`suggest_covers` calls `provider.search(...)`. The provider holds the `pool` privately, so expose a method that delegates to `search_covers`. In `core/dreamvault/src/launchbox.rs`, add to the inherent `impl LaunchBoxProvider` block (next to `fetch_file`):

```rust
    /// Fuzzy-search this provider's index for the platform. Thin wrapper over
    /// `search_covers` that uses the provider's own pool.
    pub(crate) async fn search(&self, platform: &str, query: &str, limit: usize) -> Vec<(String, String)> {
        search_covers(&self.pool, platform, query, limit).await
    }
```

- [ ] **Step 6: Replace `refetch_cover` with `apply_cover`**

In `core/dreamvault/src/metadata.rs`, delete the entire `refetch_cover` method (:1056-1075, including its doc comment) and add `apply_cover` in its place:

```rust
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
```

- [ ] **Step 7: Build the crate to confirm `refetch_cover` has no remaining engine-side callers**

Run: `cargo build -p dreamvault`
Expected: builds. (The Tauri-side caller is removed in Task 5; `dreamvault` itself has no internal callers of `refetch_cover`.)

- [ ] **Step 8: Run the dreamvault test suite**

Run: `cargo test -p dreamvault`
Expected: PASS (all existing + new tests).

- [ ] **Step 9: Commit**

```bash
git add core/dreamvault/src/metadata.rs core/dreamvault/src/launchbox.rs
git commit -m "feat(covers): suggest tagged candidates and apply_cover; drop refetch_cover"
```

---

### Task 5: Tauri commands — return tagged candidates, add `apply_cover`, remove `refetch_cover`

**Files:**
- Modify: `desktop/src-tauri/src/commands.rs` — `suggest_covers` (:297), `refetch_cover` (:307)
- Modify: `desktop/src-tauri/src/main.rs` — handler registration (:132-133)

**Interfaces:**
- Consumes: `Engine::suggest_covers -> Vec<CoverCandidate>` and `Engine::apply_cover` (Task 4).
- Produces: command `suggest_covers -> CmdResult<Vec<CoverCandidate>>`, command `apply_cover(game_id, source, token) -> CmdResult<bool>`. `refetch_cover` command removed.

- [ ] **Step 1: Import `CoverCandidate` in commands.rs**

`metadata` is a `pub mod` in `core/dreamvault/src/lib.rs`, so `CoverCandidate` (declared `pub` in Task 4) is reachable as `dreamvault::metadata::CoverCandidate`. No `lib.rs` re-export is needed. Add this import near the other `use dreamvault::...` lines at the top of `desktop/src-tauri/src/commands.rs` (e.g. after :5):

```rust
use dreamvault::metadata::CoverCandidate;
```

- [ ] **Step 2: Change the `suggest_covers` command return type**

In `desktop/src-tauri/src/commands.rs`, update (:297):

```rust
/// Closest box-art candidates for a game, ranked and tagged by source, for the
/// user to choose from in the Find picker.
#[tauri::command]
pub async fn suggest_covers(
    state: State<'_, AppState>,
    game_id: String,
    query: Option<String>,
) -> CmdResult<Vec<CoverCandidate>> {
    map(state.engine.suggest_covers(&game_id, query.as_deref()).await)
}
```

`CoverCandidate` resolves via the `use dreamvault::metadata::CoverCandidate;` added in Step 1.

- [ ] **Step 3: Replace the `refetch_cover` command with `apply_cover`**

In `desktop/src-tauri/src/commands.rs`, delete the `refetch_cover` command (:306-314) and add:

```rust
/// Apply a cover the user picked in the Find picker. `source` is "libretro" or
/// "launchbox"; `token` is the libretro box-art name or the launchbox file path.
#[tauri::command]
pub async fn apply_cover(
    state: State<'_, AppState>,
    game_id: String,
    source: String,
    token: String,
) -> CmdResult<bool> {
    map(state.engine.apply_cover(&game_id, &source, &token).await)
}
```

- [ ] **Step 4: Update the handler registration**

In `desktop/src-tauri/src/main.rs`, in the `generate_handler!` list, replace `commands::refetch_cover,` (:133) with:

```rust
            commands::apply_cover,
```

(Leave `commands::suggest_covers,` at :132 as-is — only its return type changed.)

- [ ] **Step 5: Build the Tauri crate**

Run: `cargo build -p dreamshell`
Expected: builds with no errors and no `refetch_cover` references remaining.

- [ ] **Step 6: Commit**

```bash
git add desktop/src-tauri/src/commands.rs desktop/src-tauri/src/main.rs
git commit -m "feat(covers): tauri suggest_covers candidates + apply_cover command"
```

---

### Task 6: Frontend — types, API, and GameDetail Find UI

**Files:**
- Modify: `desktop/src/api/types.ts` (after `RaGameCandidate`, :193)
- Modify: `desktop/src/api/commands.ts` — `suggestCovers` (:98), `refetchCover` (:100)
- Modify: `desktop/src/views/GameDetail.tsx` — `suggestions` state (:26), `suggest`/`applyCover` mutations (:185-207), suggestions render (:262-275)

**Interfaces:**
- Consumes: `suggest_covers -> CoverCandidate[]`, `apply_cover(gameId, source, token) -> boolean` (Task 5).
- Produces: `CoverCandidate` TS type; `api.suggestCovers` typed `CoverCandidate[]`; `api.applyCover`; `refetchCover` removed.

- [ ] **Step 1: Add the `CoverCandidate` type**

In `desktop/src/api/types.ts`, add after `RaGameCandidate` (:193):

```typescript
export interface CoverCandidate {
  source: "libretro" | "launchbox";
  label: string;
  token: string;
}
```

- [ ] **Step 2: Update the API bindings**

In `desktop/src/api/commands.ts`, change `suggestCovers` and replace `refetchCover` (:98-101):

```typescript
  suggestCovers: (gameId: string, query: string | null) =>
    invoke<CoverCandidate[]>("suggest_covers", { gameId, query }),
  applyCover: (gameId: string, source: string, token: string) =>
    invoke<boolean>("apply_cover", { gameId, source, token }),
```

Add `CoverCandidate` to the type import from `./types` at the top of `commands.ts` (find the existing `import { ... } from "./types"` and add `CoverCandidate`).

- [ ] **Step 3: Update GameDetail state and mutations**

In `desktop/src/views/GameDetail.tsx`:

Add `CoverCandidate` to the existing import from `../api/commands` or `../api/types` (the file imports `api, artworkUrl` from `../api/commands` at :4 — add a type import: `import type { CoverCandidate } from "../api/types";`).

Change the suggestions state (:26):

```typescript
  const [suggestions, setSuggestions] = useState<CoverCandidate[]>([]);
```

Replace the `applyCover` mutation (:195-207) so it takes a candidate and routes by source:

```typescript
  const applyCover = useMutation({
    mutationFn: (c: CoverCandidate) => api.applyCover(gameId, c.source, c.token),
    onSuccess: (found) => {
      setSuggestions([]);
      if (found) {
        setToast("Box art updated.");
        refreshCover();
      } else {
        setToast("Couldn't download that cover — try another.");
      }
    },
    onError: (e: unknown) => setToast(`Couldn't set cover: ${String(e)}`),
  });
```

The `suggest` mutation (:185-193) is unchanged in shape (`setSuggestions(list)` now sets `CoverCandidate[]`), but soften the empty-result copy since LaunchBox may also be consulted:

```typescript
  const suggest = useMutation({
    mutationFn: (query: string) => api.suggestCovers(gameId, query.trim() || null),
    onSuccess: (list) => {
      setSuggestions(list);
      if (list.length === 0)
        setToast("No close matches — try different words or pick a local file.");
    },
    onError: (e: unknown) => setToast(`Search failed: ${String(e)}`),
  });
```

- [ ] **Step 4: Render candidates with a source tag**

Replace the suggestions list (:262-275) in `desktop/src/views/GameDetail.tsx`:

```tsx
            {suggestions.length > 0 && (
              <ul className="flex max-h-72 flex-col gap-1 overflow-y-auto">
                {suggestions.map((c, i) => (
                  <Focusable
                    key={`${c.source}:${c.token}:${i}`}
                    onActivate={() => applyCover.mutate(c)}
                    ariaLabel={`Use ${c.label} from ${c.source}`}
                    className="glass flex items-center justify-between gap-2 rounded-lg px-2.5 py-1.5 text-left text-[11px] leading-snug"
                  >
                    <span className="min-w-0 flex-1 truncate">{c.label}</span>
                    <span className="shrink-0 text-[9px] uppercase tracking-wide text-ink-dim/70">
                      {c.source === "launchbox" ? "LaunchBox" : "libretro"}
                    </span>
                  </Focusable>
                ))}
              </ul>
            )}
```

- [ ] **Step 5: Type-check and build the frontend**

Run: `cd desktop && npm run build`
Expected: builds with no type errors; no remaining `refetchCover` reference.

- [ ] **Step 6: Commit**

```bash
git add desktop/src/api/types.ts desktop/src/api/commands.ts desktop/src/views/GameDetail.tsx
git commit -m "feat(covers): Find picker shows libretro + LaunchBox candidates tagged by source"
```

---

## Manual verification (after Task 6, with the user present)

Per the project's interactive-test etiquette, do these with the user:

1. Build + install: `cargo build -p dreamshell --release --features custom-protocol`, then atomic-install to `/usr/bin/arcadia` (temp copy + `mv -f`, never `cp` while running). Remember `npm run build` must run first so the new frontend is embedded.
2. Find a non-arcade game; type a title in the Find box; confirm libretro results appear first and (with LaunchBox enabled + index downloaded) LaunchBox results follow, each tagged.
3. Pick a libretro candidate → cover updates. Pick a LaunchBox candidate → cover updates.
4. Open an arcade game (e.g. "221B Baker Street"); type its real title; confirm arcade/MAME covers now appear from libretro and/or LaunchBox and apply.
5. Disable LaunchBox (or use a profile with no index); confirm Find still returns libretro-only with no error.

## Out of Scope

- MAME setname→full-title resolution for auto-scan.
- Thumbnail/image previews in the Find list (text + source tag only).
- Any change to the bulk enrichment pass or `lookup_cover` auto path.
