//! Launching + Statistics Engine.
//!
//! Launching a game records a `play_session`, spawns the emulator via its
//! adapter, and asynchronously waits for the process to exit to attribute
//! playtime. Stats are denormalised onto `games` for fast list rendering and
//! aggregated on demand for the dashboard ("Year in Review, always available").

use crate::adapters::LaunchContext;
use crate::error::{EngineError, Result};
use crate::models::{GamePlaytime, InstallSource, LibraryStats, PlatformUsage, SessionEnded};
use crate::{new_id, now_rfc3339, Engine};
use chrono::Utc;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct LaunchResult {
    pub session_id: String,
    pub pid: u32,
    pub emulator_id: String,
    pub emulator_name: String,
}

impl Engine {
    /// Launch a game. Resolves an emulator (explicit override → platform hint →
    /// any capable adapter), spawns it, opens a play session, and arranges for
    /// playtime to be recorded when the emulator exits.
    pub async fn launch_game(&self, game_id: &str) -> Result<LaunchResult> {
        let game = self
            .get_game(game_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("game {game_id}")))?;

        // Multi-disc set on an emulator family that understands `.m3u`: boot the
        // whole playlist so the emulator's own disc-control menu can swap discs
        // mid-game. Otherwise fall back to the primary image (disc 1).
        let discs = self.list_discs(&game.id).await?;
        let launch_rom = if discs.len() > 1 && platform_supports_m3u(&game.platform) {
            self.write_disc_playlist(&game.id, &discs)?
        } else {
            game.rom_path.clone()
        };
        self.launch_resolved(game, launch_rom, None).await
    }

    /// Launch a specific disc of a multi-disc game directly (no playlist),
    /// falling back to the primary image when the number isn't found. Lets the
    /// UI offer "Play Disc N" for manual control or emulators without `.m3u`.
    pub async fn launch_game_disc(&self, game_id: &str, disc_number: i64) -> Result<LaunchResult> {
        let game = self
            .get_game(game_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("game {game_id}")))?;
        let discs = self.list_discs(&game.id).await?;
        let launch_rom = discs
            .iter()
            .find(|d| d.disc_number == disc_number)
            .map(|d| d.rom_path.clone())
            .unwrap_or_else(|| game.rom_path.clone());
        self.launch_resolved(game, launch_rom, None).await
    }

    /// Launch a game booting straight into one of its discovered savestates.
    /// Resolves the slot's file path via the same adapter discovery the Recall
    /// State grid uses, then hands the adapter a [`LoadState`] so it can form its
    /// own startup-load flag (RetroArch `--entryslot`, Mupen64Plus `--savestate`).
    /// Falls back to a cold boot if the slot has since vanished from disk.
    ///
    /// [`LoadState`]: crate::adapters::LoadState
    pub async fn launch_game_into_state(
        &self,
        game_id: &str,
        slot: i64,
    ) -> Result<LaunchResult> {
        let game = self
            .get_game(game_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("game {game_id}")))?;

        // Re-discover the slot rather than trusting a caller-supplied path: the
        // file may have been overwritten or removed since the UI listed it, and
        // discovery is where the per-emulator keying (stem / disc-serial /
        // learned base) already lives.
        let load_state = self
            .list_save_states(game_id)
            .await?
            .into_iter()
            .find(|s| s.slot == slot)
            .map(|s| crate::adapters::LoadState {
                slot: s.slot,
                path: s.path,
            });

        // Boot into the state on the same image a normal launch would use. A
        // single-disc game's primary ROM is correct; multi-disc savestates are a
        // later concern, so we use the primary image here too.
        let launch_rom = game.rom_path.clone();
        self.launch_resolved(game, launch_rom, load_state).await
    }

    /// Materialise the ordered disc images into a cache `.m3u` playlist and
    /// return its path. Absolute paths inside, so the file lives in the cache
    /// dir and never touches the user's ROM folders.
    fn write_disc_playlist(
        &self,
        game_id: &str,
        discs: &[crate::models::GameDisc],
    ) -> Result<String> {
        let dir = self.paths.playlists_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{game_id}.m3u"));
        let mut body = String::new();
        for d in discs {
            body.push_str(&d.rom_path);
            body.push('\n');
        }
        std::fs::write(&path, body)?;
        Ok(path.to_string_lossy().into_owned())
    }

    async fn launch_resolved(
        &self,
        game: crate::models::Game,
        launch_rom: String,
        load_state: Option<crate::adapters::LoadState>,
    ) -> Result<LaunchResult> {
        let emulator = match &game.emulator_id {
            Some(id) => self.get_emulator(id).await?,
            None => self.emulator_for_platform(&game.platform).await?,
        }
        .ok_or_else(|| {
            EngineError::NotFound(format!("no emulator available for platform {}", game.platform))
        })?;

        let adapter = self.adapters.get(&emulator.adapter_id).ok_or_else(|| {
            EngineError::Invalid(format!("unknown adapter {}", emulator.adapter_id))
        })?;

        let source = InstallSource::from_str(&emulator.install_source);
        let flatpak_app_id = if source == InstallSource::Flatpak {
            // executable_path is "flatpak run <app_id>"
            emulator
                .executable_path
                .split_whitespace()
                .nth(2)
                .map(String::from)
        } else {
            None
        };

        // A zipped game on an emulator that can't read archives is transparently
        // extracted to a stable per-archive dir under the cache first (reused on
        // later launches, so the disc keeps the same absolute path — an emulator
        // savestate embeds that path and must be able to reopen it). Extraction
        // is blocking I/O, so it runs off the async runtime. On failure we fall
        // through to the raw `.zip` — no regression versus before.
        let needs_extract = rom_is_archive(&launch_rom) && !adapter.handles_archives();
        let extracted = if needs_extract {
            let zip_path = launch_rom.clone();
            tokio::task::spawn_blocking(move || {
                crate::archive::extract_for_launch(std::path::Path::new(&zip_path))
            })
            .await
            .ok()
            .flatten()
        } else {
            None
        };
        // If extraction was required but produced nothing, we're about to hand the
        // emulator the raw archive it can't read — the launch will "succeed"
        // (process spawns) but no game boots. Surface that clearly; the cause is
        // logged in `extract_for_launch` (most often a full temp filesystem).
        if needs_extract && extracted.is_none() {
            tracing::warn!(
                rom = %launch_rom, adapter = %emulator.adapter_id,
                "archive extraction failed — launching raw archive, which likely won't boot"
            );
        }
        let rom_path = extracted
            .as_ref()
            .map(|e| e.path.to_string_lossy().into_owned())
            .unwrap_or_else(|| launch_rom.clone());

        // Per-game launch overrides (extra args, env vars, core override). An
        // unconfigured game yields the empty default and contributes nothing.
        let settings = self.get_game_settings(&game.id).await?;

        let ctx = LaunchContext {
            executable_path: emulator.executable_path.clone(),
            install_source: source,
            flatpak_app_id,
            rom_path,
            platform: game.platform.clone(),
            extra_args: settings.extra_args,
            env: settings.env_vars.into_iter().collect(),
            core_override: settings.core_override,
            load_state,
            sdl_hidapi_workaround: self.load_config().controller.sdl_hidapi_workaround,
        };

        // Capture everything the post-session "learn the savestate key" step
        // needs *before* launch: a scan context and the session start. Some
        // emulators (Mupen64Plus) name their files from an internal database we
        // can't reproduce, so after the session the adapter reads back what it
        // actually wrote (files modified since `learn_since`) to learn the key.
        let learn_adapter = adapter.clone();
        let learn_ctx = crate::adapters::ScanContext {
            rom_path: ctx.rom_path.clone(),
            platform: ctx.platform.clone(),
            flatpak_app_id: ctx.flatpak_app_id.clone(),
            disc_key: None,
        };
        let learn_adapter_id = emulator.adapter_id.clone();
        let learn_since = std::time::SystemTime::now();

        let handle = adapter.launch(&ctx).await?;
        let pid = handle.pid;

        // Keep the emulator on the same monitor as Dreamshell. Capture the
        // focused workspace now (before the emulator window maps) and, under
        // Hyprland, relocate the window once it appears. Best-effort and silent
        // on other compositors — see `wm`.
        if crate::wm::hyprland() {
            if let Some(ws) = crate::wm::focused_workspace().await {
                crate::wm::relocate_to_workspace(pid, ws);
            }
        }

        // Open the session and bump launch stats immediately — the launch
        // happened even if the user quits in two seconds.
        let session_id = new_id();
        let started = now_rfc3339();
        sqlx::query(
            "INSERT INTO play_sessions (id, game_id, emulator_id, started_at) VALUES (?,?,?,?)",
        )
        .bind(&session_id)
        .bind(&game.id)
        .bind(&emulator.id)
        .bind(&started)
        .execute(&self.pool)
        .await?;

        sqlx::query("UPDATE games SET launch_count = launch_count + 1, last_played = ? WHERE id = ?")
            .bind(&started)
            .bind(&game.id)
            .execute(&self.pool)
            .await?;

        // Wait for exit off-thread and attribute playtime.
        let pool = self.pool.clone();
        let tx = self.session_tx.clone();
        let sid = session_id.clone();
        let gid = game.id.clone();
        let start_instant = Utc::now();
        let mut child = handle.child;
        tokio::spawn(async move {
            // The extraction now persists across launches (stable per-archive
            // dir), so there's no temp dir to clean up — just drop the handle.
            drop(extracted);
            let status = child.wait().await;
            // Measure in seconds, then round to the nearest minute — but any
            // session that actually ran counts as at least 1 minute, so a brief
            // play no longer reads as "never played" (whole-minute truncation
            // used to drop everything under 60s to zero).
            let seconds = (Utc::now() - start_instant).num_seconds().max(0);
            let minutes = if seconds == 0 {
                0
            } else {
                ((seconds as f64) / 60.0).round().max(1.0) as i64
            };
            tracing::info!(
                game = %gid,
                ?status,
                seconds,
                minutes,
                "play session ended; attributing playtime"
            );
            let ended = Utc::now().to_rfc3339();
            let _ = sqlx::query(
                "UPDATE play_sessions SET ended_at = ?, duration_minutes = ? WHERE id = ?",
            )
            .bind(&ended)
            .bind(minutes)
            .bind(&sid)
            .execute(&pool)
            .await;
            let _ = sqlx::query(
                "UPDATE games SET playtime_minutes = playtime_minutes + ? WHERE id = ?",
            )
            .bind(minutes)
            .bind(&gid)
            .execute(&pool)
            .await;
            // Learn (and persist) the key this emulator named its files by, if it
            // wrote any this session. Only database-keyed adapters (Mupen64Plus)
            // return one; the rest resolve their key from the ROM and report
            // None here. Latest session wins, so the row self-heals on rename.
            if let Some(key) = learn_adapter.learn_state_key(&learn_ctx, learn_since).await {
                let learned_at = Utc::now().to_rfc3339();
                let _ = sqlx::query(
                    "INSERT INTO state_keys (game_id, adapter_id, state_key, learned_at) \
                     VALUES (?,?,?,?) \
                     ON CONFLICT(game_id) DO UPDATE SET \
                       adapter_id = excluded.adapter_id, \
                       state_key = excluded.state_key, \
                       learned_at = excluded.learned_at",
                )
                .bind(&gid)
                .bind(&learn_adapter_id)
                .bind(&key)
                .bind(&learned_at)
                .execute(&pool)
                .await;
            }
            // Notify subscribers (the UI) that playtime changed. Errors mean no
            // receiver is listening, which is fine.
            let _ = tx.send(SessionEnded {
                session_id: sid,
                game_id: gid,
                duration_minutes: minutes,
            });
        });

        Ok(LaunchResult {
            session_id,
            pid,
            emulator_id: emulator.id,
            emulator_name: emulator.name,
        })
    }

    /// Aggregate dashboard statistics for a profile.
    pub async fn library_stats(&self, profile_id: &str) -> Result<LibraryStats> {
        let mut stats = LibraryStats::default();

        let row: (i64, i64, i64, i64, i64) = sqlx::query_as(
            r#"
            SELECT
                COUNT(*),
                COALESCE(SUM(playtime_minutes), 0),
                COALESCE(SUM(launch_count), 0),
                COUNT(DISTINCT platform),
                COALESCE(SUM(favorite), 0)
            FROM games WHERE profile_id = ?
            "#,
        )
        .bind(profile_id)
        .fetch_one(&self.pool)
        .await?;
        stats.total_games = row.0;
        stats.total_playtime_minutes = row.1;
        stats.total_launches = row.2;
        stats.platform_count = row.3;
        stats.favorite_count = row.4;

        stats.most_played = sqlx::query_as::<_, GamePlaytime>(
            r#"
            SELECT id, title, platform, cover_art, playtime_minutes, launch_count, last_played
            FROM games
            WHERE profile_id = ? AND playtime_minutes > 0
            ORDER BY playtime_minutes DESC
            LIMIT 10
            "#,
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;

        stats.recently_played = sqlx::query_as::<_, GamePlaytime>(
            r#"
            SELECT id, title, platform, cover_art, playtime_minutes, launch_count, last_played
            FROM games
            WHERE profile_id = ? AND last_played IS NOT NULL
            ORDER BY last_played DESC
            LIMIT 10
            "#,
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;

        stats.platform_usage = sqlx::query_as::<_, PlatformUsage>(
            r#"
            SELECT platform,
                   COUNT(*) AS game_count,
                   COALESCE(SUM(playtime_minutes), 0) AS playtime_minutes
            FROM games
            WHERE profile_id = ?
            GROUP BY platform
            ORDER BY playtime_minutes DESC, game_count DESC
            "#,
        )
        .bind(profile_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(stats)
    }
}

/// Is this ROM path a `.zip`/`.7z` we might need to extract before launch?
fn rom_is_archive(rom_path: &str) -> bool {
    std::path::Path::new(rom_path)
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("7z"))
}

/// Whether a platform's emulators read `.m3u` playlists for in-emulator disc
/// swapping. The CD-based systems below are covered by RetroArch's disc cores
/// and by the standalone emulators we ship (DuckStation, PCSX2, PPSSPP,
/// Flycast, mednafen-Saturn), all of which boot an `.m3u` and expose a
/// disc-control menu. Cartridge platforms never need it.
fn platform_supports_m3u(platform: &str) -> bool {
    matches!(
        platform,
        "ps1" | "ps2" | "psp" | "saturn" | "segacd" | "dreamcast" | "pcengine"
    )
}
