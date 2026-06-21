//! ROM Library Engine + workspace profiles + ROM source folders.
//!
//! ROMs are **read in place** at user-specified locations; Arcadia never
//! relocates a user's files, and never downloads ROMs.

use crate::error::{EngineError, Result};
use crate::models::{Collection, CollectionSummary, Game, GameSettings, Profile, RomSource};
use crate::{new_id, now_rfc3339, platforms, Engine};
use std::collections::HashSet;
use walkdir::WalkDir;

/// Result of a library scan.
#[derive(Debug, Default, serde::Serialize)]
pub struct ScanReport {
    pub scanned_files: usize,
    pub added: usize,
    pub skipped_existing: usize,
    pub unrecognized: usize,
    /// Stale rows pruned: games under a scanned source whose file is gone or
    /// which the current rules no longer treat as a game (e.g. `.pkg` blobs
    /// indexed before PS3 detection became folder-aware).
    pub removed: usize,
}

/// Filters / sort applied to a game listing.
#[derive(Debug, Default, serde::Deserialize)]
pub struct GameQuery {
    pub profile_id: String,
    pub platform: Option<String>,
    pub favorites_only: Option<bool>,
    pub search: Option<String>,
    pub sort: Option<String>, // "title" | "recent" | "playtime"
    pub limit: Option<i64>,
}

impl Engine {
    // ---- Profiles ---------------------------------------------------------

    /// Ensure a default profile exists and return it. Called at startup.
    pub async fn ensure_default_profile(&self) -> Result<Profile> {
        if let Some(p) = sqlx::query_as::<_, Profile>(
            "SELECT * FROM profiles WHERE is_default = 1 LIMIT 1",
        )
        .fetch_optional(&self.pool)
        .await?
        {
            return Ok(p);
        }
        let id = new_id();
        let now = now_rfc3339();
        sqlx::query(
            "INSERT INTO profiles (id, name, is_default, created_at) VALUES (?, ?, 1, ?)",
        )
        .bind(&id)
        .bind("Default")
        .bind(&now)
        .execute(&self.pool)
        .await?;
        self.get_profile(&id)
            .await?
            .ok_or_else(|| EngineError::NotFound("default profile".into()))
    }

    pub async fn list_profiles(&self) -> Result<Vec<Profile>> {
        Ok(sqlx::query_as::<_, Profile>(
            "SELECT * FROM profiles ORDER BY is_default DESC, name COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_profile(&self, id: &str) -> Result<Option<Profile>> {
        Ok(
            sqlx::query_as::<_, Profile>("SELECT * FROM profiles WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn create_profile(&self, name: &str) -> Result<Profile> {
        let name = name.trim();
        if name.is_empty() {
            return Err(EngineError::Invalid("profile name is empty".into()));
        }
        let id = new_id();
        let now = now_rfc3339();
        sqlx::query("INSERT INTO profiles (id, name, is_default, created_at) VALUES (?, ?, 0, ?)")
            .bind(&id)
            .bind(name)
            .bind(&now)
            .execute(&self.pool)
            .await?;
        self.get_profile(&id)
            .await?
            .ok_or_else(|| EngineError::NotFound("profile".into()))
    }

    // ---- ROM sources ------------------------------------------------------

    pub async fn add_rom_source(&self, profile_id: &str, path: &str) -> Result<RomSource> {
        let p = std::path::Path::new(path);
        if !p.is_dir() {
            return Err(EngineError::Invalid(format!("not a directory: {path}")));
        }
        let id = new_id();
        let now = now_rfc3339();
        sqlx::query(
            "INSERT INTO rom_sources (id, profile_id, path, added_at) VALUES (?, ?, ?, ?)
             ON CONFLICT(profile_id, path) DO NOTHING",
        )
        .bind(&id)
        .bind(profile_id)
        .bind(path)
        .bind(&now)
        .execute(&self.pool)
        .await?;
        let row = sqlx::query_as::<_, RomSource>(
            "SELECT * FROM rom_sources WHERE profile_id = ? AND path = ?",
        )
        .bind(profile_id)
        .bind(path)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn list_rom_sources(&self, profile_id: &str) -> Result<Vec<RomSource>> {
        Ok(sqlx::query_as::<_, RomSource>(
            "SELECT * FROM rom_sources WHERE profile_id = ? ORDER BY added_at",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Remove a ROM source and every game that was indexed from under it.
    ///
    /// Games discovered elsewhere are left alone even if a second source still
    /// covers them. A game counts as "under" the removed folder only on a path
    /// boundary (`/roms/snes` does not swallow `/roms/snes-extra`). `game_discs`
    /// rows clear via `ON DELETE CASCADE`; we also delete them explicitly so the
    /// cleanup holds on connections without foreign keys enforced (tests).
    pub async fn remove_rom_source(&self, id: &str) -> Result<()> {
        let Some((profile_id, path)) = sqlx::query_as::<_, (String, String)>(
            "SELECT profile_id, path FROM rom_sources WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?
        else {
            return Ok(());
        };

        let games = sqlx::query_as::<_, (String, String)>(
            "SELECT id, rom_path FROM games WHERE profile_id = ?",
        )
        .bind(&profile_id)
        .fetch_all(&self.pool)
        .await?;

        let prefix = format!("{}/", path.trim_end_matches('/'));
        let mut tx = self.pool.begin().await?;
        for (game_id, rom_path) in games {
            if rom_path == path || rom_path.starts_with(&prefix) {
                sqlx::query("DELETE FROM game_discs WHERE game_id = ?")
                    .bind(&game_id)
                    .execute(&mut *tx)
                    .await?;
                sqlx::query("DELETE FROM games WHERE id = ?")
                    .bind(&game_id)
                    .execute(&mut *tx)
                    .await?;
            }
        }
        sqlx::query("DELETE FROM rom_sources WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }

    // ---- Scanning ---------------------------------------------------------

    /// Walk every ROM source for a profile, indexing recognised ROMs. Existing
    /// games (same profile + path) are left untouched.
    pub async fn scan_library(&self, profile_id: &str) -> Result<ScanReport> {
        // The indexing burst can fail mid-scan (e.g. a constraint violation on a
        // pathological disc set). The progress channel is fire-and-forget: if we
        // return early on `?` without a terminal event, the sidebar bar is left
        // parked at its last `done/total` and looks frozen rather than failed.
        // Wrap the body so any error emits a terminal `finished` event first,
        // letting the UI clear the indicator and surface the failure.
        match self.scan_library_inner(profile_id).await {
            Ok(report) => Ok(report),
            Err(e) => {
                tracing::error!(error = %e, "library scan failed");
                self.emit_progress(profile_id, "scan", "Scan failed", 0, 0, true);
                Err(e)
            }
        }
    }

    async fn scan_library_inner(&self, profile_id: &str) -> Result<ScanReport> {
        let sources = self.list_rom_sources(profile_id).await?;
        let source_paths: Vec<String> = sources.iter().map(|s| s.path.clone()).collect();
        let now = now_rfc3339();

        tracing::info!(profile = %profile_id, sources = sources.len(), "library scan started");
        // Indeterminate while the filesystem walk runs — its size isn't known
        // until it finishes, but the sidebar should show the scan is alive.
        self.emit_progress(profile_id, "scan", "Scanning ROM folders", 0, 0, false);

        // The walk is filesystem-bound — often minutes on an external USB drive
        // with tens of thousands of files — and uses fully blocking IO. Run it
        // off the async runtime so the app (and Tauri command) stays responsive,
        // then apply the DB writes here. `walk_sources` logs periodic progress
        // so a long scan isn't mistaken for a hang.
        let (candidates, mut report) = tokio::task::spawn_blocking(move || walk_sources(&sources))
            .await
            .map_err(|e| EngineError::Invalid(format!("scan walk panicked: {e}")))?;

        tracing::info!(
            matched = candidates.len(),
            scanned = report.scanned_files,
            "scan walk done; writing index"
        );

        // Collapse multi-disc sets: candidates carrying a disc marker in their
        // filename ("(Disc 1)", "- Disc 2", ...) are grouped by platform + base
        // title into ONE game; everything else is a standalone game.
        let mut disc_groups: std::collections::HashMap<(String, String), Vec<Candidate>> =
            std::collections::HashMap::new();
        let mut singles: Vec<Candidate> = Vec::new();
        for c in candidates {
            let stem = std::path::Path::new(&c.rom_path)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            if disc_number(stem).is_some() {
                let key = (c.platform.to_string(), sort_key(&base_title(&c.title)));
                disc_groups.entry(key).or_default().push(c);
            } else {
                singles.push(c);
            }
        }

        // Every rom_path we still consider a *primary* game record this run;
        // anything under a scanned source but absent here is stale and gets
        // pruned afterwards. Non-primary disc images are intentionally NOT marked
        // seen — that lets older per-disc game rows get pruned on the first
        // rescan after grouping lands.
        let index_total = singles.len() + disc_groups.len();
        // Indexing is a tight DB-write burst — thousands of items in well under a
        // second on a big library. Emitting per item floods the progress channel
        // and IPC bridge, which drops the tail (the bar froze a few short). Cap to
        // ~100 updates, but always emit the final item so the bar reliably lands
        // on 100% before the `finished` event clears it.
        let emit_step = (index_total / 100).max(1);
        let mut indexed = 0usize;
        let mut seen: HashSet<String> = HashSet::with_capacity(index_total);
        // Batch the index into bounded transactions instead of one giant one.
        // Under WAL + synchronous=NORMAL a commit is a cheap WAL append (no
        // fsync until checkpoint), so committing every `COMMIT_CHUNK` games turns
        // tens of thousands of auto-commits into a couple dozen — the difference
        // between a frozen scan and a fast one — WITHOUT holding a single
        // connection and the write lock across thousands of await round-trips to
        // the sqlite worker thread (that long-held transaction wedged the scan
        // future at the commit boundary on large libraries). Periodic commits
        // also let the UI see partial progress and keep the page cache / WAL
        // bounded. ON CONFLICT upserts are idempotent, so a partial scan is safe.
        const COMMIT_CHUNK: usize = 1000;
        let mut since_commit = 0usize;
        let mut tx = self.pool.begin().await?;
        for c in singles {
            let gid = self
                .insert_game(&mut tx, profile_id, &c.title, c.platform, &c.rom_path, c.size, &now, &mut report)
                .await?;
            self.replace_discs(&mut tx, &gid, &[]).await?;
            seen.insert(c.rom_path);
            indexed += 1;
            if indexed % emit_step == 0 || indexed == index_total {
                self.emit_progress(profile_id, "scan", "Indexing games", indexed, index_total, false);
            }
            since_commit += 1;
            if since_commit >= COMMIT_CHUNK {
                tx.commit().await?;
                tx = self.pool.begin().await?;
                since_commit = 0;
            }
        }
        for ((_platform, _key), mut group) in disc_groups {
            group.sort_by_key(|c| {
                let stem = std::path::Path::new(&c.rom_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                disc_number(stem).unwrap_or(i64::MAX)
            });
            // A lone disc marker isn't really a set — index it as a plain game.
            if group.len() == 1 {
                let c = &group[0];
                let gid = self
                    .insert_game(
                        &mut tx, profile_id, &c.title, c.platform, &c.rom_path, c.size, &now,
                        &mut report,
                    )
                    .await?;
                self.replace_discs(&mut tx, &gid, &[]).await?;
                seen.insert(c.rom_path.clone());
                indexed += 1;
                if indexed % emit_step == 0 || indexed == index_total {
                    self.emit_progress(profile_id, "scan", "Indexing games", indexed, index_total, false);
                }
                since_commit += 1;
                if since_commit >= COMMIT_CHUNK {
                    tx.commit().await?;
                    tx = self.pool.begin().await?;
                    since_commit = 0;
                }
                continue;
            }

            let primary = &group[0];
            let title = base_title(&primary.title);
            let gid = self
                .insert_game(
                    &mut tx, profile_id, &title, primary.platform, &primary.rom_path, primary.size,
                    &now, &mut report,
                )
                .await?;
            // Assign each disc a UNIQUE number. Even after the grouping fixes,
            // a legitimate set can carry duplicate parsed numbers (e.g.
            // "Game (Disc 1)" alongside "Game (Disc 1) (Rev A)"); bumping
            // collisions to the next free slot keeps every disc and guarantees
            // we never violate the (game_id, disc_number) UNIQUE constraint,
            // which would otherwise abort the entire library scan.
            let mut used: HashSet<i64> = HashSet::new();
            let mut discs: Vec<(i64, String, String)> = Vec::with_capacity(group.len());
            for (i, c) in group.iter().enumerate() {
                let stem = std::path::Path::new(&c.rom_path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("");
                let mut n = disc_number(stem).unwrap_or((i + 1) as i64);
                while !used.insert(n) {
                    n += 1;
                }
                discs.push((n, c.rom_path.clone(), format!("Disc {n}")));
            }
            self.replace_discs(&mut tx, &gid, &discs).await?;
            seen.insert(primary.rom_path.clone());
            indexed += 1;
            if indexed % emit_step == 0 || indexed == index_total {
                self.emit_progress(profile_id, "scan", "Indexing games", indexed, index_total, false);
            }
            since_commit += 1;
            if since_commit >= COMMIT_CHUNK {
                tx.commit().await?;
                tx = self.pool.begin().await?;
                since_commit = 0;
            }
        }

        // Commit the final partial chunk (may be empty — committing an empty
        // transaction is a cheap no-op).
        tx.commit().await?;

        report.removed = self.prune_stale(profile_id, &source_paths, &seen).await?;
        self.emit_progress(profile_id, "scan", "Scan complete", index_total, index_total, true);
        self.ensure_platforms_seeded().await?;
        tracing::info!(
            scanned = report.scanned_files,
            added = report.added,
            updated = report.skipped_existing,
            removed = report.removed,
            "library scan complete"
        );
        Ok(report)
    }

    /// Index one game, deriving its sort key and tallying the scan report. On a
    /// rescan the row's *derived* fields (title, platform, size) are refreshed
    /// so changed detection rules take effect — e.g. an EBOOT or ISO previously
    /// mis-tagged ps2 is corrected to ps3. User-owned fields (favorite,
    /// playtime, cover art, emulator override, added_at) are preserved.
    /// Upsert one game keyed on `(profile_id, rom_path)` and return its id.
    #[allow(clippy::too_many_arguments)]
    async fn insert_game(
        &self,
        conn: &mut sqlx::SqliteConnection,
        profile_id: &str,
        title: &str,
        platform: &str,
        rom_path: &str,
        size: i64,
        now: &str,
        report: &mut ScanReport,
    ) -> Result<String> {
        let sort_title = sort_key(title);
        let existing_id = sqlx::query_scalar::<_, String>(
            "SELECT id FROM games WHERE profile_id = ? AND rom_path = ?",
        )
        .bind(profile_id)
        .bind(rom_path)
        .fetch_optional(&mut *conn)
        .await?;

        let id = existing_id.clone().unwrap_or_else(new_id);
        sqlx::query(
            r#"
            INSERT INTO games (
                id, profile_id, title, sort_title, platform, rom_path,
                file_size, added_at
            ) VALUES (?,?,?,?,?,?,?,?)
            ON CONFLICT(profile_id, rom_path) DO UPDATE SET
                title      = excluded.title,
                sort_title = CASE
                               WHEN custom_title IS NOT NULL AND custom_title <> ''
                               THEN games.sort_title
                               ELSE excluded.sort_title
                             END,
                platform   = excluded.platform,
                file_size  = excluded.file_size
            "#,
        )
        .bind(&id)
        .bind(profile_id)
        .bind(title)
        .bind(&sort_title)
        .bind(platform)
        .bind(rom_path)
        .bind(size)
        .bind(now)
        .execute(&mut *conn)
        .await?;

        if existing_id.is_some() {
            report.skipped_existing += 1;
        } else {
            report.added += 1;
        }
        Ok(id)
    }

    /// Replace a game's disc list with `discs` (ordered `(disc_number, path,
    /// label)`). Used for multi-disc sets; a no-op list clears the rows, turning
    /// the game back into a plain single-disc entry.
    async fn replace_discs(
        &self,
        conn: &mut sqlx::SqliteConnection,
        game_id: &str,
        discs: &[(i64, String, String)],
    ) -> Result<()> {
        sqlx::query("DELETE FROM game_discs WHERE game_id = ?")
            .bind(game_id)
            .execute(&mut *conn)
            .await?;
        for (number, path, label) in discs {
            sqlx::query(
                "INSERT INTO game_discs (game_id, disc_number, rom_path, label) VALUES (?,?,?,?)",
            )
            .bind(game_id)
            .bind(number)
            .bind(path)
            .bind(label)
            .execute(&mut *conn)
            .await?;
        }
        Ok(())
    }

    /// The ordered disc list for a game (empty for single-disc games).
    pub async fn list_discs(&self, game_id: &str) -> Result<Vec<crate::models::GameDisc>> {
        Ok(sqlx::query_as::<_, crate::models::GameDisc>(
            "SELECT game_id, disc_number, rom_path, label FROM game_discs
             WHERE game_id = ? ORDER BY disc_number",
        )
        .bind(game_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Delete games that live under a scanned source root but weren't seen in
    /// the latest walk — files the user deleted, and rows the old extension-only
    /// rules wrongly indexed (loose `.pkg` updates/DLC). Games outside every
    /// scanned root are left alone. Returns how many were removed.
    async fn prune_stale(
        &self,
        profile_id: &str,
        source_paths: &[String],
        seen: &HashSet<String>,
    ) -> Result<usize> {
        if source_paths.is_empty() {
            return Ok(0);
        }
        let existing = sqlx::query_as::<_, (String, String)>(
            "SELECT id, rom_path FROM games WHERE profile_id = ?",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;

        let mut removed = 0usize;
        for (id, rom_path) in existing {
            let under_scan = source_paths.iter().any(|root| rom_path.starts_with(root));
            if under_scan && !seen.contains(&rom_path) {
                sqlx::query("DELETE FROM games WHERE id = ?")
                    .bind(&id)
                    .execute(&self.pool)
                    .await?;
                removed += 1;
            }
        }
        Ok(removed)
    }

    /// Seed the platforms table with any platform referenced by the catalogue.
    /// Cheap and idempotent; keeps foreign keys valid for games.
    pub async fn ensure_platforms_seeded(&self) -> Result<()> {
        for def in platforms::PLATFORMS {
            sqlx::query(
                "INSERT INTO platforms (id, name, manufacturer, generation) VALUES (?,?,?,?)
                 ON CONFLICT(id) DO NOTHING",
            )
            .bind(def.id)
            .bind(def.name)
            .bind(def.manufacturer)
            .bind(def.generation)
            .execute(&self.pool)
            .await?;
        }
        Ok(())
    }

    // ---- Game queries -----------------------------------------------------

    pub async fn list_games(&self, q: &GameQuery) -> Result<Vec<Game>> {
        let mut sql = String::from("SELECT * FROM games WHERE profile_id = ?");
        if q.platform.is_some() {
            sql.push_str(" AND platform = ?");
        }
        if q.favorites_only.unwrap_or(false) {
            sql.push_str(" AND favorite = 1");
        }
        if q.search.is_some() {
            sql.push_str(" AND sort_title LIKE ?");
        }
        let order = match q.sort.as_deref() {
            Some("recent") => " ORDER BY last_played DESC NULLS LAST, sort_title",
            Some("playtime") => " ORDER BY playtime_minutes DESC, sort_title",
            _ => " ORDER BY sort_title",
        };
        sql.push_str(order);
        if let Some(limit) = q.limit {
            sql.push_str(&format!(" LIMIT {}", limit.max(0)));
        }

        let mut query = sqlx::query_as::<_, Game>(&sql).bind(&q.profile_id);
        if let Some(p) = &q.platform {
            query = query.bind(p);
        }
        if let Some(s) = &q.search {
            query = query.bind(format!("%{}%", s.to_lowercase()));
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn get_game(&self, id: &str) -> Result<Option<Game>> {
        Ok(sqlx::query_as::<_, Game>("SELECT * FROM games WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn set_favorite(&self, game_id: &str, favorite: bool) -> Result<()> {
        sqlx::query("UPDATE games SET favorite = ? WHERE id = ?")
            .bind(favorite)
            .bind(game_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Override which emulator launches a game (NULL = engine picks).
    pub async fn set_game_emulator(&self, game_id: &str, emulator_id: Option<&str>) -> Result<()> {
        sqlx::query("UPDATE games SET emulator_id = ? WHERE id = ?")
            .bind(emulator_id)
            .bind(game_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Set or clear a game's user display-name override. A non-empty (trimmed)
    /// name becomes `custom_title` and drives `sort_title` (so sort and search
    /// follow the new name). Passing `None` or a whitespace-only name clears the
    /// override and reverts `sort_title` to the scanned `title`'s key.
    pub async fn set_custom_title(&self, game_id: &str, title: Option<&str>) -> Result<()> {
        let trimmed = title.map(str::trim).filter(|s| !s.is_empty());
        match trimmed {
            Some(name) => {
                sqlx::query("UPDATE games SET custom_title = ?, sort_title = ? WHERE id = ?")
                    .bind(name)
                    .bind(sort_key(name))
                    .bind(game_id)
                    .execute(&self.pool)
                    .await?;
            }
            None => {
                let derived: Option<String> =
                    sqlx::query_scalar("SELECT title FROM games WHERE id = ?")
                        .bind(game_id)
                        .fetch_optional(&self.pool)
                        .await?;
                if let Some(derived) = derived {
                    sqlx::query(
                        "UPDATE games SET custom_title = NULL, sort_title = ? WHERE id = ?",
                    )
                    .bind(sort_key(&derived))
                    .bind(game_id)
                    .execute(&self.pool)
                    .await?;
                }
            }
        }
        Ok(())
    }

    /// Per-game launch overrides (extra args, env vars, core override). Returns
    /// the empty default when the game has no row, so callers never special-case
    /// "unset" — an unconfigured game just contributes nothing at launch.
    pub async fn get_game_settings(&self, game_id: &str) -> Result<GameSettings> {
        let row: Option<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT extra_args, env_vars, core_override FROM game_settings WHERE game_id = ?",
        )
        .bind(game_id)
        .fetch_optional(&self.pool)
        .await?;
        match row {
            Some((args, env, core)) => Ok(GameSettings {
                game_id: game_id.to_string(),
                extra_args: serde_json::from_str(&args)?,
                env_vars: serde_json::from_str(&env)?,
                core_override: core,
            }),
            None => Ok(GameSettings {
                game_id: game_id.to_string(),
                ..Default::default()
            }),
        }
    }

    /// Upsert a game's launch overrides. When everything is empty the row is
    /// deleted instead of stored, so a cleared-out game resolves back to engine
    /// defaults rather than carrying a dead, all-empty row.
    pub async fn set_game_settings(&self, settings: &GameSettings) -> Result<()> {
        let empty = settings.extra_args.is_empty()
            && settings.env_vars.is_empty()
            && settings.core_override.is_none();
        if empty {
            sqlx::query("DELETE FROM game_settings WHERE game_id = ?")
                .bind(&settings.game_id)
                .execute(&self.pool)
                .await?;
            return Ok(());
        }
        let args = serde_json::to_string(&settings.extra_args)?;
        let env = serde_json::to_string(&settings.env_vars)?;
        sqlx::query(
            "INSERT INTO game_settings (game_id, extra_args, env_vars, core_override, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(game_id) DO UPDATE SET
                extra_args = excluded.extra_args,
                env_vars = excluded.env_vars,
                core_override = excluded.core_override,
                updated_at = excluded.updated_at",
        )
        .bind(&settings.game_id)
        .bind(&args)
        .bind(&env)
        .bind(&settings.core_override)
        .bind(now_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_game(&self, game_id: &str) -> Result<()> {
        sqlx::query("DELETE FROM games WHERE id = ?")
            .bind(game_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    // ---- Collections ------------------------------------------------------

    pub async fn list_collections(&self, profile_id: &str) -> Result<Vec<Collection>> {
        Ok(sqlx::query_as::<_, Collection>(
            "SELECT * FROM collections WHERE profile_id = ? ORDER BY name COLLATE NOCASE",
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn create_collection(&self, profile_id: &str, name: &str) -> Result<Collection> {
        let id = new_id();
        let now = now_rfc3339();
        sqlx::query(
            "INSERT INTO collections (id, profile_id, name, created_at) VALUES (?,?,?,?)",
        )
        .bind(&id)
        .bind(profile_id)
        .bind(name.trim())
        .bind(&now)
        .execute(&self.pool)
        .await?;
        sqlx::query_as::<_, Collection>("SELECT * FROM collections WHERE id = ?")
            .bind(&id)
            .fetch_one(&self.pool)
            .await
            .map_err(Into::into)
    }

    /// Collections with their game counts, for the Collections grid.
    pub async fn list_collection_summaries(
        &self,
        profile_id: &str,
    ) -> Result<Vec<CollectionSummary>> {
        Ok(sqlx::query_as::<_, CollectionSummary>(
            r#"
            SELECT c.id, c.name, c.created_at,
                   COUNT(cg.game_id) AS game_count
            FROM collections c
            LEFT JOIN collection_games cg ON cg.collection_id = c.id
            WHERE c.profile_id = ?
            GROUP BY c.id
            ORDER BY c.name COLLATE NOCASE
            "#,
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn remove_collection(&self, id: &str) -> Result<()> {
        // collection_games rows cascade via the FK.
        sqlx::query("DELETE FROM collections WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Add a game to a collection. Idempotent (the PK is the pair).
    pub async fn add_game_to_collection(&self, collection_id: &str, game_id: &str) -> Result<()> {
        sqlx::query(
            "INSERT INTO collection_games (collection_id, game_id) VALUES (?, ?)
             ON CONFLICT(collection_id, game_id) DO NOTHING",
        )
        .bind(collection_id)
        .bind(game_id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn remove_game_from_collection(
        &self,
        collection_id: &str,
        game_id: &str,
    ) -> Result<()> {
        sqlx::query("DELETE FROM collection_games WHERE collection_id = ? AND game_id = ?")
            .bind(collection_id)
            .bind(game_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Games in a collection, ordered for display.
    pub async fn collection_games(&self, collection_id: &str) -> Result<Vec<Game>> {
        Ok(sqlx::query_as::<_, Game>(
            r#"
            SELECT g.* FROM games g
            JOIN collection_games cg ON cg.game_id = g.id
            WHERE cg.collection_id = ?
            ORDER BY g.sort_title
            "#,
        )
        .bind(collection_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Collection ids a game belongs to (for membership toggles in the UI).
    pub async fn game_collections(&self, game_id: &str) -> Result<Vec<String>> {
        Ok(
            sqlx::query_scalar::<_, String>(
                "SELECT collection_id FROM collection_games WHERE game_id = ?",
            )
            .bind(game_id)
            .fetch_all(&self.pool)
            .await?,
        )
    }
}

/// Turn a ROM filename into a human title: drop the extension, strip common
/// region/dump tags in (parens) and [brackets], normalise separators.
/// One indexed game produced by the blocking walk, ready for a DB write.
struct Candidate {
    title: String,
    platform: &'static str,
    rom_path: String,
    size: i64,
}

/// Walk every ROM source, classify files, and collect game candidates. Pure
/// filesystem + CPU work with no DB access, so it can run on a blocking thread.
/// Emits a `tracing` progress line every few seconds so a slow scan over a large
/// external drive is visibly making progress rather than appearing hung.
fn walk_sources(sources: &[RomSource]) -> (Vec<Candidate>, ScanReport) {
    let mut report = ScanReport::default();
    let mut candidates: Vec<Candidate> = Vec::new();

    for source in sources {
        tracing::info!(source = %source.path, "scanning ROM source");
        // PS3 disc-folder dumps detected during this source's walk. Files
        // beneath a root (EBOOT chunks, licences, bundled updates/DLC) are part
        // of the game, not separate games, so they're skipped. WalkDir yields
        // directories before their contents, so a root is always recorded before
        // we reach anything inside it.
        let mut ps3_roots: Vec<std::path::PathBuf> = Vec::new();
        let mut last_log = std::time::Instant::now();

        for entry in WalkDir::new(&source.path)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();

            if entry.file_type().is_dir() {
                let sfo = path.join("PS3_GAME").join("PARAM.SFO");
                if sfo.is_file() {
                    ps3_roots.push(path.to_path_buf());
                    let title =
                        crate::sfo::read_title(&sfo).unwrap_or_else(|| derive_title(path));
                    // rpcs3 boots the decrypted EBOOT directly; fall back to the
                    // disc root when the EBOOT isn't where we expect.
                    let eboot = path.join("PS3_GAME").join("USRDIR").join("EBOOT.BIN");
                    let (rom_path, size) = if eboot.is_file() {
                        let sz = std::fs::metadata(&eboot).map(|m| m.len() as i64).unwrap_or(0);
                        (eboot.to_string_lossy().to_string(), sz)
                    } else {
                        (path.to_string_lossy().to_string(), 0)
                    };
                    candidates.push(Candidate { title, platform: "ps3", rom_path, size });
                }
                continue;
            }

            report.scanned_files += 1;
            if last_log.elapsed().as_secs() >= 5 {
                tracing::info!(
                    scanned = report.scanned_files,
                    matched = candidates.len(),
                    "scan progress"
                );
                last_log = std::time::Instant::now();
            }

            // Inside a detected PS3 game folder — not a separate game.
            if ps3_roots.iter().any(|r| path.starts_with(r)) {
                continue;
            }

            let ext = match path.extension().and_then(|e| e.to_str()) {
                Some(e) => e.to_ascii_lowercase(),
                None => {
                    report.unrecognized += 1;
                    continue;
                }
            };
            let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
            let matched = match platforms::resolve_rom(path, stem, &ext) {
                Some(m) => m,
                None => {
                    report.unrecognized += 1;
                    continue;
                }
            };

            let rom_path = path.to_string_lossy().to_string();
            // Prefer a title read from the disc (PSP PARAM.SFO) over the filename
            // guess, mirroring how PS3 dumps use their SFO title.
            let title = matched.title.unwrap_or_else(|| derive_title(path));
            let size = entry.metadata().map(|m| m.len() as i64).unwrap_or(0);
            candidates.push(Candidate { title, platform: matched.platform, rom_path, size });
        }
    }

    (candidates, report)
}

/// Disc-number keywords, in match priority. "cd" is last because it's the most
/// collision-prone (it appears inside ordinary words).
const DISC_KEYWORDS: [&str; 3] = ["disc", "disk", "cd"];

/// True if `kw` occurs in `lower` at `idx` on a word boundary (not glued to a
/// surrounding letter/digit), so "disc" in "discovery" or "cd" in "arcade"
/// doesn't read as a disc marker.
fn at_word_boundary(lower: &str, idx: usize, kw: &str) -> bool {
    let before_ok = idx == 0
        || !lower[..idx]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    let after = &lower[idx + kw.len()..];
    let after_ok = after
        .chars()
        .next()
        .is_none_or(|c| !c.is_ascii_alphabetic());
    before_ok && after_ok
}

/// Find the byte index of a disc keyword in `lower` whose first following
/// non-separator characters are digits, returning `(idx, keyword, number)`.
fn find_disc_marker(lower: &str) -> Option<(usize, &'static str, i64)> {
    for kw in DISC_KEYWORDS {
        let mut from = 0;
        while let Some(rel) = lower[from..].find(kw) {
            let idx = from + rel;
            from = idx + kw.len();
            if !at_word_boundary(lower, idx, kw) {
                continue;
            }
            // "cd" is dangerously ambiguous: it appears in hardware names far
            // more often than as a disc tag ("Mega-CD 2", "Sega CD 2",
            // "Sega CDX"). A genuine disc marker spelled "cd" is universally
            // parenthesised/bracketed ("(CD 2)", "(CD2)"), so only honour it
            // there. Otherwise unrelated BIOS dumps collapse to one "game" with
            // colliding disc numbers and the whole scan aborts on the
            // (game_id, disc_number) UNIQUE constraint.
            if kw == "cd"
                && !matches!(lower[..idx].chars().next_back(), Some('(') | Some('['))
            {
                continue;
            }
            let rest = lower[idx + kw.len()..]
                .trim_start_matches([' ', '_', '-', '.', '#', ':']);
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = digits.parse::<i64>() {
                return Some((idx, kw, n));
            }
        }
    }
    None
}

/// The disc number encoded in a filename stem, if any: `(Disc 1)`, `(Disk 2)`,
/// `(CD 1)`, ` - Disc 1`, `Disc 1 of 3`, etc. Case-insensitive.
fn disc_number(stem: &str) -> Option<i64> {
    find_disc_marker(&stem.to_ascii_lowercase()).map(|(_, _, n)| n)
}

/// A title with its disc marker (and the separator/bracket leading into it)
/// removed, so every disc of a set normalises to the same base title — the
/// grouping key and the displayed name. `derive_title` already strips
/// parenthesised markers; this also handles bare ` - Disc 1` / `Disc 1` forms.
fn base_title(title: &str) -> String {
    let lower = title.to_ascii_lowercase();
    if let Some((idx, _, _)) = find_disc_marker(&lower) {
        let head = title[..idx]
            .trim_end()
            .trim_end_matches(['-', '_', '(', '[', ':'])
            .trim_end();
        if !head.is_empty() {
            return head.to_string();
        }
    }
    title.trim().to_string()
}

fn derive_title(path: &std::path::Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Unknown");

    let mut out = String::with_capacity(stem.len());
    let mut depth_paren = 0i32;
    let mut depth_brack = 0i32;
    for c in stem.chars() {
        match c {
            '(' => depth_paren += 1,
            ')' => depth_paren = (depth_paren - 1).max(0),
            '[' => depth_brack += 1,
            ']' => depth_brack = (depth_brack - 1).max(0),
            '_' | '.' if depth_paren == 0 && depth_brack == 0 => out.push(' '),
            _ if depth_paren == 0 && depth_brack == 0 => out.push(c),
            _ => {}
        }
    }
    let title = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if title.is_empty() {
        stem.to_string()
    } else {
        title
    }
}

/// Sort key: lowercase, ignore a leading "the ".
fn sort_key(title: &str) -> String {
    let lower = title.to_lowercase();
    lower
        .strip_prefix("the ")
        .map(|s| s.to_string())
        .unwrap_or(lower)
}

#[cfg(test)]
mod tests {
    use super::{base_title, disc_number, ScanReport};
    use crate::{db, Engine};

    async fn insert_game(engine: &Engine, profile_id: &str, id: &str, rom_path: &str) {
        sqlx::query(
            "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, added_at)
             VALUES (?, ?, ?, ?, 'snes', ?, '2026-01-01T00:00:00Z')",
        )
        .bind(id)
        .bind(profile_id)
        .bind(id)
        .bind(id)
        .bind(rom_path)
        .execute(&engine.pool)
        .await
        .unwrap();
    }

    async fn game_ids(engine: &Engine, profile_id: &str) -> Vec<String> {
        sqlx::query_as::<_, (String,)>("SELECT id FROM games WHERE profile_id = ? ORDER BY id")
            .bind(profile_id)
            .fetch_all(&engine.pool)
            .await
            .unwrap()
            .into_iter()
            .map(|(id,)| id)
            .collect()
    }

    #[tokio::test]
    async fn new_games_have_no_custom_title() {
        let pool = db::connect_in_memory().await.unwrap();
        let engine = Engine::with_pool(pool).await.unwrap();
        let profile = engine.ensure_default_profile().await.unwrap();

        sqlx::query(
            "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, added_at)
             VALUES ('g1', ?, 'Mario', 'mario', 'snes', '/roms/snes/Mario.sfc', '2026-01-01T00:00:00Z')",
        )
        .bind(&profile.id)
        .execute(&engine.pool)
        .await
        .unwrap();

        let game = engine.get_game("g1").await.unwrap().unwrap();
        assert_eq!(game.custom_title, None);
    }

    #[tokio::test]
    async fn remove_rom_source_deletes_only_games_under_that_folder() {
        let pool = db::connect_in_memory().await.unwrap();
        let engine = Engine::with_pool(pool).await.unwrap();
        let profile = engine.ensure_default_profile().await.unwrap();

        // Two real source folders plus a sibling path that shares a prefix.
        let src = sqlx::query_as::<_, crate::models::RomSource>(
            "INSERT INTO rom_sources (id, profile_id, path, added_at)
             VALUES ('s1', ?, '/roms/snes', '2026-01-01T00:00:00Z') RETURNING *",
        )
        .bind(&profile.id)
        .fetch_one(&engine.pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO rom_sources (id, profile_id, path, added_at)
             VALUES ('s2', ?, '/roms/nes', '2026-01-01T00:00:00Z')",
        )
        .bind(&profile.id)
        .execute(&engine.pool)
        .await
        .unwrap();

        insert_game(&engine, &profile.id, "under", "/roms/snes/Game.sfc").await;
        insert_game(&engine, &profile.id, "exact", "/roms/snes").await;
        insert_game(&engine, &profile.id, "sibling", "/roms/snes-extra/Game.sfc").await;
        insert_game(&engine, &profile.id, "other", "/roms/nes/Game.nes").await;

        // The removed game has discs; they must go too.
        sqlx::query(
            "INSERT INTO game_discs (game_id, disc_number, rom_path) VALUES ('under', 1, '/roms/snes/Game.sfc')",
        )
        .execute(&engine.pool)
        .await
        .unwrap();

        engine.remove_rom_source(&src.id).await.unwrap();

        let remaining = game_ids(&engine, &profile.id).await;
        assert_eq!(remaining, vec!["other".to_string(), "sibling".to_string()]);

        let discs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM game_discs WHERE game_id = 'under'")
            .fetch_one(&engine.pool)
            .await
            .unwrap();
        assert_eq!(discs, 0);

        let sources: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM rom_sources WHERE id = 's1'")
            .fetch_one(&engine.pool)
            .await
            .unwrap();
        assert_eq!(sources, 0);
    }

    #[tokio::test]
    async fn remove_unknown_source_is_a_noop() {
        let pool = db::connect_in_memory().await.unwrap();
        let engine = Engine::with_pool(pool).await.unwrap();
        engine.remove_rom_source("does-not-exist").await.unwrap();
    }

    #[test]
    fn disc_number_reads_common_conventions() {
        assert_eq!(disc_number("Final Fantasy VII (USA) (Disc 1)"), Some(1));
        assert_eq!(disc_number("Final Fantasy VII (USA) (Disc 2)"), Some(2));
        assert_eq!(disc_number("Some Game (Disk 3)"), Some(3));
        assert_eq!(disc_number("Some Game (CD 2)"), Some(2));
        assert_eq!(disc_number("Some Game (CD2)"), Some(2));
        assert_eq!(disc_number("Chrono Cross - Disc 1 of 2"), Some(1));
        assert_eq!(disc_number("Game Disc 4"), Some(4));
    }

    #[test]
    fn disc_number_ignores_non_disc_filenames() {
        // No digit after the keyword, and keywords glued inside ordinary words
        // must not register as disc markers.
        assert_eq!(disc_number("Discworld"), None);
        assert_eq!(disc_number("Arcade Classics"), None);
        assert_eq!(disc_number("Tony Hawk Pro Skater"), None);
        assert_eq!(disc_number("Final Fantasy VII (USA)"), None);
    }

    #[test]
    fn base_title_strips_bare_disc_markers() {
        // Parenthesised markers are already gone by the time titles reach here
        // (derive_title removes them), so base_title is a no-op for those.
        assert_eq!(base_title("Final Fantasy VII"), "Final Fantasy VII");
        // Bare markers (no parens) are the case base_title actually cleans up.
        assert_eq!(base_title("Chrono Cross - Disc 1"), "Chrono Cross");
        assert_eq!(base_title("Parasite Eve Disc 2"), "Parasite Eve");
    }

    #[tokio::test]
    async fn set_and_clear_custom_title_updates_sort_key() {
        let pool = db::connect_in_memory().await.unwrap();
        let engine = Engine::with_pool(pool).await.unwrap();
        let profile = engine.ensure_default_profile().await.unwrap();

        sqlx::query(
            "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, added_at)
             VALUES ('g1', ?, 'SLU1654', 'slu1654', 'ps2', '/roms/ps2/SLU1654.iso', '2026-01-01T00:00:00Z')",
        )
        .bind(&profile.id)
        .execute(&engine.pool)
        .await
        .unwrap();

        // Set a custom name (with surrounding whitespace to prove trimming).
        engine
            .set_custom_title("g1", Some("  The Final Fantasy X  "))
            .await
            .unwrap();
        let row: (Option<String>, String, String) =
            sqlx::query_as("SELECT custom_title, sort_title, title FROM games WHERE id = 'g1'")
                .fetch_one(&engine.pool)
                .await
                .unwrap();
        assert_eq!(row.0.as_deref(), Some("The Final Fantasy X"));
        // sort_key lowercases and strips a leading "the ".
        assert_eq!(row.1, "final fantasy x");
        assert_eq!(row.2, "SLU1654"); // derived title untouched

        // Clear it -> sort_title reverts to the derived title's key.
        engine.set_custom_title("g1", None).await.unwrap();
        let row: (Option<String>, String) =
            sqlx::query_as("SELECT custom_title, sort_title FROM games WHERE id = 'g1'")
                .fetch_one(&engine.pool)
                .await
                .unwrap();
        assert_eq!(row.0, None);
        assert_eq!(row.1, "slu1654");
    }

    #[tokio::test]
    async fn empty_custom_title_clears_override() {
        let pool = db::connect_in_memory().await.unwrap();
        let engine = Engine::with_pool(pool).await.unwrap();
        let profile = engine.ensure_default_profile().await.unwrap();

        sqlx::query(
            "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, added_at)
             VALUES ('g1', ?, 'SLU1654', 'slu1654', 'ps2', '/roms/ps2/SLU1654.iso', '2026-01-01T00:00:00Z')",
        )
        .bind(&profile.id)
        .execute(&engine.pool)
        .await
        .unwrap();

        engine.set_custom_title("g1", Some("Renamed")).await.unwrap();
        engine.set_custom_title("g1", Some("   ")).await.unwrap(); // whitespace-only => clear
        let custom: Option<String> =
            sqlx::query_scalar("SELECT custom_title FROM games WHERE id = 'g1'")
                .fetch_one(&engine.pool)
                .await
                .unwrap();
        assert_eq!(custom, None);
    }

    #[tokio::test]
    async fn rescan_preserves_custom_title_and_its_sort_key() {
        let pool = db::connect_in_memory().await.unwrap();
        let engine = Engine::with_pool(pool).await.unwrap();
        let profile = engine.ensure_default_profile().await.unwrap();

        let mut report = ScanReport::default();

        // First index (insert path). insert_game takes a transaction-backed
        // connection, mirroring the scanner (`self.pool.begin()` -> `&mut tx`).
        let mut tx = engine.pool.begin().await.unwrap();
        let id = engine
            .insert_game(
                &mut tx,
                &profile.id,
                "SLU1654",
                "ps2",
                "/roms/ps2/SLU1654.iso",
                123,
                "2026-01-01T00:00:00Z",
                &mut report,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();

        engine
            .set_custom_title(&id, Some("Final Fantasy X"))
            .await
            .unwrap();

        // Re-index the same ROM (same profile_id + rom_path) -> upsert path.
        let mut tx = engine.pool.begin().await.unwrap();
        engine
            .insert_game(
                &mut tx,
                &profile.id,
                "SLU1654",
                "ps2",
                "/roms/ps2/SLU1654.iso",
                456,
                "2026-02-01T00:00:00Z",
                &mut report,
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let row: (Option<String>, String, String, i64) = sqlx::query_as(
            "SELECT custom_title, sort_title, title, file_size FROM games WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&engine.pool)
        .await
        .unwrap();
        assert_eq!(row.0.as_deref(), Some("Final Fantasy X")); // override kept
        assert_eq!(row.1, "final fantasy x"); // custom sort key kept
        assert_eq!(row.2, "SLU1654"); // derived title still refreshed
        assert_eq!(row.3, 456); // other derived fields still refreshed
    }
}
