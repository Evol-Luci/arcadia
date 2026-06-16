//! # DreamVault — the Arcadia engine ("the truth layer").
//!
//! Storage, ROM index, emulator registry, metadata, saves, and stats. This
//! crate has **no UI and no Tauri dependency** so it can be tested and reasoned
//! about in isolation; the Dreamshell interface (a Tauri app) depends on it.
//!
//! Design rule it enforces everywhere: *Arcadia orchestrates existing systems,
//! it never replaces emulator functionality.*

pub mod achievements;
pub mod adapters;
pub mod archive;
pub mod config;
pub mod controller;
pub mod db;
pub mod error;
pub mod iso9660;
pub mod library;
pub mod metadata;
pub mod models;
pub mod paths;
pub mod platforms;
pub mod plugins;
pub mod registry;
pub mod save_states;
pub mod saves;
pub mod screenshots;
pub mod sfo;
pub mod stats;
pub mod sync;
pub mod wm;

pub use error::{EngineError, Result};
pub use paths::ArcadiaPaths;

use adapters::AdapterRegistry;
use db::Pool;
use models::{ProgressEvent, SessionEnded};
use tokio::sync::broadcast;

/// The DreamVault engine. Cheap to clone where needed (the pool is an `Arc`),
/// but normally held once in app state.
pub struct Engine {
    pub(crate) pool: Pool,
    pub(crate) adapters: AdapterRegistry,
    pub paths: ArcadiaPaths,
    /// Fan-out of "play session ended" notifications. Playtime is attributed
    /// asynchronously when an emulator exits; consumers (the Tauri shell)
    /// subscribe to refresh the UI without polling.
    pub(crate) session_tx: broadcast::Sender<SessionEnded>,
    /// Fan-out of background-job progress (library scan, box-art fetch) so the
    /// UI can render a live progress bar instead of a frozen spinner.
    pub(crate) progress_tx: broadcast::Sender<ProgressEvent>,
}

impl Engine {
    /// Bootstrap the engine against the real XDG data directory: open the
    /// database, run migrations, seed platforms, and ensure a default profile.
    pub async fn bootstrap() -> Result<Self> {
        let paths = ArcadiaPaths::discover()?;
        paths.ensure()?;
        let pool = db::connect(&paths.database_path()).await?;
        let (session_tx, _) = broadcast::channel(64);
        let (progress_tx, _) = broadcast::channel(256);
        let engine = Self {
            pool,
            adapters: AdapterRegistry::builtin(),
            paths,
            session_tx,
            progress_tx,
        };
        engine.ensure_platforms_seeded().await?;
        engine.ensure_default_profile().await?;
        Ok(engine)
    }

    /// Build an engine against an explicit pool (used by tests).
    pub async fn with_pool(pool: Pool) -> Result<Self> {
        let paths = ArcadiaPaths::discover()?;
        let (session_tx, _) = broadcast::channel(64);
        let (progress_tx, _) = broadcast::channel(256);
        let engine = Self {
            pool,
            adapters: AdapterRegistry::builtin(),
            paths,
            session_tx,
            progress_tx,
        };
        engine.ensure_platforms_seeded().await?;
        engine.ensure_default_profile().await?;
        Ok(engine)
    }

    pub fn adapter_descriptors(&self) -> Vec<adapters::AdapterDescriptor> {
        self.adapters.descriptors()
    }

    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Load user configuration (credentials, preferences) from the config dir.
    /// Returns defaults if the file is absent or unparsable.
    pub fn load_config(&self) -> config::AppConfig {
        config::AppConfig::load(&self.paths.config_dir)
    }

    /// Persist user configuration to the config dir.
    pub fn save_config(&self, cfg: &config::AppConfig) -> Result<()> {
        cfg.save(&self.paths.config_dir)
    }

    /// Subscribe to play-session-ended notifications. Each call returns an
    /// independent receiver; the Tauri shell uses one to push refresh events
    /// to the webview when an emulator exits.
    pub fn subscribe_sessions(&self) -> broadcast::Receiver<SessionEnded> {
        self.session_tx.subscribe()
    }

    /// Subscribe to background-job progress events (library scan, box-art fetch).
    pub fn subscribe_progress(&self) -> broadcast::Receiver<ProgressEvent> {
        self.progress_tx.subscribe()
    }

    /// Emit a progress event. A send error just means no receiver is attached
    /// (no UI listening) — progress is advisory, so we drop it silently.
    pub(crate) fn emit_progress(
        &self,
        profile_id: &str,
        kind: &str,
        label: &str,
        done: usize,
        total: usize,
        finished: bool,
    ) {
        let _ = self.progress_tx.send(ProgressEvent {
            profile_id: profile_id.to_string(),
            kind: kind.to_string(),
            label: label.to_string(),
            done,
            total,
            finished,
        });
    }
}

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

pub(crate) fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::GameQuery;

    async fn test_engine() -> Engine {
        let pool = db::connect_in_memory().await.unwrap();
        Engine::with_pool(pool).await.unwrap()
    }

    #[tokio::test]
    async fn bootstraps_default_profile_and_platforms() {
        let engine = test_engine().await;
        let profiles = engine.list_profiles().await.unwrap();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].is_default);

        let n64 = platforms::by_id("n64").unwrap();
        assert_eq!(n64.adapter_hint, "mupen64plus");
    }

    #[tokio::test]
    async fn scans_roms_and_indexes_by_platform() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();

        // Build a fake ROM tree.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Super Mario World (USA).sfc"), b"x").unwrap();
        std::fs::write(dir.path().join("Zelda_OoT.z64"), b"x").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"x").unwrap();

        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        let report = engine.scan_library(&profile.id).await.unwrap();
        assert_eq!(report.added, 2);
        assert_eq!(report.unrecognized, 1);

        let games = engine
            .list_games(&GameQuery {
                profile_id: profile.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(games.len(), 2);

        let snes = games.iter().find(|g| g.platform == "snes").unwrap();
        assert_eq!(snes.title, "Super Mario World");

        // Re-scan is idempotent.
        let report2 = engine.scan_library(&profile.id).await.unwrap();
        assert_eq!(report2.added, 0);
        assert_eq!(report2.skipped_existing, 2);
    }

    #[tokio::test]
    async fn ps3_folder_dump_indexed_once_with_sfo_title() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();
        let dir = tempfile::tempdir().unwrap();

        // A disc-folder dump: PS3_GAME/PARAM.SFO holds the real title, plus the
        // kind of junk that used to be mis-indexed as games (a bundled update
        // PKG and a licence RAP).
        let game = dir.path().join("BLES02166-[Call of Duty]");
        let usrdir = game.join("PS3_GAME").join("USRDIR");
        std::fs::create_dir_all(&usrdir).unwrap();
        std::fs::write(usrdir.join("EBOOT.BIN"), b"eboot").unwrap();
        let sfo = make_param_sfo("Call of Duty: Black Ops III");
        std::fs::write(game.join("PS3_GAME").join("PARAM.SFO"), &sfo).unwrap();
        std::fs::write(
            game.join("EP0002-BLES02166_00-BLACKOPS3TU00000-A0102-V0100-PE.pkg"),
            b"junk",
        )
        .unwrap();

        // A standalone PS3 ISO, recognised by its title ID in the name.
        std::fs::write(dir.path().join("BCES00081-[Killzone 2].iso"), b"x").unwrap();
        // A loose update PKG outside any game folder — not a game.
        std::fs::write(dir.path().join("UP0002-NPUB30584_00-UPDATE.pkg"), b"x").unwrap();

        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        let report = engine.scan_library(&profile.id).await.unwrap();

        let games = engine
            .list_games(&crate::library::GameQuery {
                profile_id: profile.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();

        // Exactly two PS3 games: the folder dump and the ISO. No PKG junk.
        assert_eq!(games.len(), 2, "{games:?}");
        assert_eq!(report.added, 2);
        assert!(games.iter().all(|g| g.platform == "ps3"));

        let folder = games
            .iter()
            .find(|g| g.rom_path.ends_with("EBOOT.BIN"))
            .expect("folder dump points at EBOOT.BIN");
        assert_eq!(folder.title, "Call of Duty: Black Ops III");

        assert!(games.iter().any(|g| g.rom_path.ends_with(".iso")));
        assert!(!games.iter().any(|g| g.rom_path.ends_with(".pkg")));

        // Re-scan is idempotent.
        let report2 = engine.scan_library(&profile.id).await.unwrap();
        assert_eq!(report2.added, 0);
    }

    #[tokio::test]
    async fn multi_disc_set_is_collapsed_into_one_game_with_discs() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();
        let dir = tempfile::tempdir().unwrap();

        // A three-disc PS1 game plus a one-disc game in the same folder.
        for n in 1..=3 {
            std::fs::write(
                dir.path().join(format!("Final Fantasy VII (USA) (Disc {n}).chd")),
                b"x",
            )
            .unwrap();
        }
        std::fs::write(dir.path().join("Crash Bandicoot (USA).chd"), b"x").unwrap();

        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        let report = engine.scan_library(&profile.id).await.unwrap();

        let games = engine
            .list_games(&GameQuery {
                profile_id: profile.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();

        // Two library cards: the collapsed FF7 set and Crash — not four.
        assert_eq!(games.len(), 2, "{games:?}");
        assert_eq!(report.added, 2);

        let ff7 = games.iter().find(|g| g.title == "Final Fantasy VII").unwrap();
        // The primary points at disc 1.
        assert!(ff7.rom_path.contains("Disc 1"));

        let discs = engine.list_discs(&ff7.id).await.unwrap();
        assert_eq!(discs.len(), 3);
        assert_eq!(discs[0].disc_number, 1);
        assert_eq!(discs[2].disc_number, 3);
        assert!(discs[1].rom_path.contains("Disc 2"));

        // The single-disc game has no disc rows.
        let crash = games.iter().find(|g| g.title == "Crash Bandicoot").unwrap();
        assert!(engine.list_discs(&crash.id).await.unwrap().is_empty());

        // Re-scan is idempotent: still two games, no new adds, discs unchanged.
        let report2 = engine.scan_library(&profile.id).await.unwrap();
        assert_eq!(report2.added, 0);
        assert_eq!(report2.skipped_existing, 2);
        assert_eq!(engine.list_discs(&ff7.id).await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn isos_are_routed_by_disc_structure() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();
        let dir = tempfile::tempdir().unwrap();

        // ISOs with names that give away nothing about the platform — the exact
        // case that used to dump everything into PS2. GameCube and Wii dumps are
        // raw images (not ISO 9660) and were the most recent casualty.
        std::fs::write(
            dir.path().join("Ace Combat X.iso"),
            crate::iso9660::testfixtures::psp_iso("Ace Combat X: Skies of Deception"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Baldurs Gate Dark Alliance.iso"),
            crate::iso9660::testfixtures::ps2_iso(),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Luigis Mansion.iso"),
            crate::iso9660::testfixtures::gamecube_iso("LUIGIS MANSION"),
        )
        .unwrap();
        std::fs::write(
            dir.path().join("Super Mario Galaxy.iso"),
            crate::iso9660::testfixtures::wii_iso("SUPER MARIO GALAXY"),
        )
        .unwrap();

        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        engine.scan_library(&profile.id).await.unwrap();

        let games = engine
            .list_games(&GameQuery {
                profile_id: profile.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(games.len(), 4);

        let psp = games.iter().find(|g| g.platform == "psp").expect("a PSP game");
        // Title comes from the disc's PARAM.SFO, not the bare filename.
        assert_eq!(psp.title, "Ace Combat X: Skies of Deception");
        assert!(games.iter().any(|g| g.platform == "ps2"));
        assert!(games.iter().any(|g| g.platform == "gamecube"));
        assert!(games.iter().any(|g| g.platform == "wii"));
    }

    #[tokio::test]
    async fn rescan_corrects_platform_but_keeps_user_data() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let iso = dir.path().join("BCES00081-[Killzone 2].iso");
        std::fs::write(&iso, b"x").unwrap();
        let rom_path = iso.to_str().unwrap();

        // Simulate a row left by the old extension-only rules: a PS3 ISO indexed
        // as ps2, which the user then favorited.
        sqlx::query(
            "INSERT INTO games (id, profile_id, title, sort_title, platform, rom_path, file_size, favorite, added_at)
             VALUES ('stale', ?, 'BCES00081-', 'bces00081-', 'ps2', ?, 0, 1, '2020-01-01T00:00:00Z')",
        )
        .bind(&profile.id)
        .bind(rom_path)
        .execute(engine.pool())
        .await
        .unwrap();

        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        engine.scan_library(&profile.id).await.unwrap();

        let games = engine
            .list_games(&crate::library::GameQuery {
                profile_id: profile.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(games.len(), 1);
        let g = &games[0];
        assert_eq!(g.platform, "ps3", "ISO should be re-routed to ps3");
        assert!(g.favorite, "user's favorite flag must survive a rescan");
        assert_eq!(g.id, "stale", "same row corrected in place, not duplicated");
    }

    #[tokio::test]
    async fn rescan_prunes_games_whose_files_vanished() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let rom = dir.path().join("Chrono Trigger (USA).sfc");
        std::fs::write(&rom, b"x").unwrap();
        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        assert_eq!(engine.scan_library(&profile.id).await.unwrap().added, 1);

        // The file disappears; a rescan should drop its row.
        std::fs::remove_file(&rom).unwrap();
        let report = engine.scan_library(&profile.id).await.unwrap();
        assert_eq!(report.removed, 1);
        let games = engine
            .list_games(&crate::library::GameQuery {
                profile_id: profile.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        assert!(games.is_empty());
    }

    /// Minimal single-field PARAM.SFO for tests (mirrors the real layout).
    fn make_param_sfo(title: &str) -> Vec<u8> {
        let key = b"TITLE\0";
        let mut value = title.as_bytes().to_vec();
        value.push(0);
        let key_table_start = 20 + 16;
        let data_table_start = key_table_start + key.len();

        let mut out = Vec::new();
        out.extend_from_slice(b"\x00PSF");
        out.extend_from_slice(&0x0101_0000u32.to_le_bytes());
        out.extend_from_slice(&(key_table_start as u32).to_le_bytes());
        out.extend_from_slice(&(data_table_start as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // key_offset
        out.extend_from_slice(&0x0204u16.to_le_bytes()); // utf-8
        out.extend_from_slice(&(value.len() as u32).to_le_bytes()); // data_len
        out.extend_from_slice(&(value.len() as u32).to_le_bytes()); // data_max
        out.extend_from_slice(&0u32.to_le_bytes()); // data_offset
        out.extend_from_slice(key);
        out.extend_from_slice(&value);
        out
    }

    #[tokio::test]
    async fn collection_membership_roundtrip() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Game.gba"), b"x").unwrap();
        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        engine.scan_library(&profile.id).await.unwrap();
        let game = engine
            .list_games(&GameQuery { profile_id: profile.id.clone(), ..Default::default() })
            .await
            .unwrap()
            .remove(0);

        let coll = engine.create_collection(&profile.id, "Handheld").await.unwrap();
        engine.add_game_to_collection(&coll.id, &game.id).await.unwrap();
        // Idempotent.
        engine.add_game_to_collection(&coll.id, &game.id).await.unwrap();

        assert_eq!(engine.collection_games(&coll.id).await.unwrap().len(), 1);
        assert_eq!(engine.game_collections(&game.id).await.unwrap(), vec![coll.id.clone()]);
        let summaries = engine.list_collection_summaries(&profile.id).await.unwrap();
        assert_eq!(summaries[0].game_count, 1);

        engine.remove_game_from_collection(&coll.id, &game.id).await.unwrap();
        assert!(engine.collection_games(&coll.id).await.unwrap().is_empty());

        // Deleting a game cascades its membership; deleting a collection too.
        engine.add_game_to_collection(&coll.id, &game.id).await.unwrap();
        engine.remove_collection(&coll.id).await.unwrap();
        assert!(engine.list_collections(&profile.id).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn game_settings_roundtrip() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Game.gba"), b"x").unwrap();
        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        engine.scan_library(&profile.id).await.unwrap();
        let game = engine
            .list_games(&GameQuery { profile_id: profile.id.clone(), ..Default::default() })
            .await
            .unwrap()
            .remove(0);

        // Unconfigured game yields the empty default, not an error.
        let empty = engine.get_game_settings(&game.id).await.unwrap();
        assert!(empty.extra_args.is_empty());
        assert!(empty.env_vars.is_empty());
        assert!(empty.core_override.is_none());

        // Persist a mix of args, env vars, and a core override; read it back.
        let mut env = std::collections::BTreeMap::new();
        env.insert("__NV_PRIME_RENDER_OFFLOAD".to_string(), "1".to_string());
        engine
            .set_game_settings(&crate::models::GameSettings {
                game_id: game.id.clone(),
                extra_args: vec!["--fullscreen".to_string()],
                env_vars: env.clone(),
                core_override: Some("mgba".to_string()),
            })
            .await
            .unwrap();
        let got = engine.get_game_settings(&game.id).await.unwrap();
        assert_eq!(got.extra_args, vec!["--fullscreen".to_string()]);
        assert_eq!(got.env_vars, env);
        assert_eq!(got.core_override.as_deref(), Some("mgba"));

        // Clearing everything removes the row (resolves back to engine defaults).
        engine
            .set_game_settings(&crate::models::GameSettings {
                game_id: game.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        let cleared = engine.get_game_settings(&game.id).await.unwrap();
        assert!(cleared.extra_args.is_empty());
        assert!(cleared.env_vars.is_empty());
        assert!(cleared.core_override.is_none());

        // Deleting the game cascades the settings row away (no orphan).
        engine
            .set_game_settings(&crate::models::GameSettings {
                game_id: game.id.clone(),
                core_override: Some("mgba".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        engine.remove_game(&game.id).await.unwrap();
        let after = engine.get_game_settings(&game.id).await.unwrap();
        assert!(after.core_override.is_none());
    }

    #[tokio::test]
    async fn favorite_toggle_and_stats() {
        let engine = test_engine().await;
        let profile = engine.ensure_default_profile().await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Game.gba"), b"x").unwrap();
        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        engine.scan_library(&profile.id).await.unwrap();

        let games = engine
            .list_games(&GameQuery {
                profile_id: profile.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap();
        let g = &games[0];
        engine.set_favorite(&g.id, true).await.unwrap();

        let stats = engine.library_stats(&profile.id).await.unwrap();
        assert_eq!(stats.total_games, 1);
        assert_eq!(stats.favorite_count, 1);
    }
}
