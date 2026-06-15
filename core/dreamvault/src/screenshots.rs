//! Screenshot indexing (v0.5).
//!
//! Emulators write screenshots to their own directories. Arcadia *indexes* what
//! the user already has — it never takes screenshots itself. Discovery is
//! best-effort and currently covers RetroArch (the broadest adapter); other
//! emulators slot in behind the same engine API.
//!
//! Discovered images are copied into `<cache_dir>/screenshots/<game_id>/` so the
//! webview can load them through the Tauri asset protocol (the emulator's own
//! directory is outside the granted asset scope). The original is never touched;
//! the cache copy is disposable. Re-scans are idempotent: a `(game_id,
//! source_path)` uniqueness constraint means an already-imported shot is skipped.

use crate::error::{EngineError, Result};
use crate::models::{Game, InstallSource, Screenshot};
use crate::{new_id, now_rfc3339, Engine};
use std::path::{Path, PathBuf};

/// RetroArch's screenshot output directory for a native or Flatpak install.
fn retroarch_screenshot_dirs(flatpak_app_id: Option<&str>) -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let base = match flatpak_app_id {
        Some(app) => home.join(".var/app").join(app).join("config/retroarch"),
        None => home.join(".config/retroarch"),
    };
    vec![base.join("screenshots")]
}

/// Image files in `dirs` belonging to the ROM whose basename (without extension)
/// is `stem`. RetroArch names a shot after the content (`<stem>.png`) and adds a
/// suffix for subsequent captures (`<stem>-<n>.png`, `<stem> 2024-...png`), so we
/// match any image whose name starts with the stem. Pure and order-stable.
fn match_screenshot_files(dirs: &[PathBuf], stem: &str) -> Vec<PathBuf> {
    const EXTS: [&str; 4] = ["png", "jpg", "jpeg", "bmp"];
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let ext_ok = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| EXTS.contains(&e.to_ascii_lowercase().as_str()))
                .unwrap_or(false);
            if !ext_ok {
                continue;
            }
            if let Some(name) = path.file_stem().and_then(|n| n.to_str()) {
                if name == stem || name.starts_with(&format!("{stem}-")) || name.starts_with(&format!("{stem} ")) {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    out
}

/// RFC3339 mtime of a file, best-effort.
fn file_mtime_rfc3339(path: &Path) -> Option<String> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let dt: chrono::DateTime<chrono::Utc> = modified.into();
    Some(dt.to_rfc3339())
}

impl Engine {
    /// Locate the live screenshot files for a game, best-effort. Resolves the
    /// game's emulator/adapter and, for RetroArch, matches files in the
    /// screenshot dir by the ROM's basename. Empty for unmapped adapters.
    pub async fn screenshot_files_for_game(&self, game: &Game) -> Result<Vec<PathBuf>> {
        let emulator = match &game.emulator_id {
            Some(id) => self.get_emulator(id).await?,
            None => self.emulator_for_platform(&game.platform).await?,
        };
        let Some(emulator) = emulator else {
            return Ok(Vec::new());
        };
        if emulator.adapter_id != "retroarch" {
            return Ok(Vec::new());
        }
        let flatpak_app_id = if InstallSource::from_str(&emulator.install_source)
            == InstallSource::Flatpak
        {
            emulator.executable_path.split_whitespace().nth(2).map(String::from)
        } else {
            None
        };
        let stem = Path::new(&game.rom_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&game.title);
        let dirs = retroarch_screenshot_dirs(flatpak_app_id.as_deref());
        Ok(match_screenshot_files(&dirs, stem))
    }

    /// Import one screenshot for a game: copy it into the cache and record a row.
    /// Idempotent on `(game_id, source_path)`. Returns the row, or `None` if it
    /// was already indexed.
    async fn import_screenshot(&self, game_id: &str, source: &Path) -> Result<Option<Screenshot>> {
        let source_str = source.to_string_lossy().to_string();
        let existing: Option<(String,)> =
            sqlx::query_as("SELECT id FROM screenshots WHERE game_id = ? AND source_path = ?")
                .bind(game_id)
                .bind(&source_str)
                .fetch_optional(&self.pool)
                .await?;
        if existing.is_some() {
            return Ok(None);
        }

        let dest_dir = self.paths.screenshots_dir().join(game_id);
        std::fs::create_dir_all(&dest_dir)?;
        let base = source
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("screenshot.png");
        // Prefix with the row id to avoid collisions between same-named sources.
        let id = new_id();
        let dest = dest_dir.join(format!("{}-{base}", &id[..8]));
        let byte_size = std::fs::copy(source, &dest)? as i64;

        let created_at = now_rfc3339();
        let captured_at = file_mtime_rfc3339(source);
        sqlx::query(
            "INSERT INTO screenshots (id, game_id, path, source_path, byte_size, captured_at, created_at)
             VALUES (?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(game_id)
        .bind(dest.to_string_lossy().to_string())
        .bind(&source_str)
        .bind(byte_size)
        .bind(&captured_at)
        .bind(&created_at)
        .execute(&self.pool)
        .await?;

        self.get_screenshot(&id).await
    }

    /// Scan every game in a profile for new screenshots, importing any not yet
    /// indexed. Returns the number of newly-imported shots.
    pub async fn scan_screenshots(&self, profile_id: &str) -> Result<usize> {
        let games = self
            .list_games(&crate::library::GameQuery {
                profile_id: profile_id.to_string(),
                ..Default::default()
            })
            .await?;
        let mut imported = 0usize;
        for game in &games {
            let files = match self.screenshot_files_for_game(game).await {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(game = %game.id, error = %e, "screenshot discovery failed");
                    continue;
                }
            };
            for file in files {
                match self.import_screenshot(&game.id, &file).await {
                    Ok(Some(_)) => imported += 1,
                    Ok(None) => {}
                    Err(e) => tracing::warn!(?file, error = %e, "screenshot import failed"),
                }
            }
        }
        Ok(imported)
    }

    pub async fn get_screenshot(&self, id: &str) -> Result<Option<Screenshot>> {
        Ok(
            sqlx::query_as::<_, Screenshot>("SELECT * FROM screenshots WHERE id = ?")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn list_screenshots(&self, game_id: &str) -> Result<Vec<Screenshot>> {
        Ok(sqlx::query_as::<_, Screenshot>(
            "SELECT * FROM screenshots WHERE game_id = ? ORDER BY captured_at DESC, created_at DESC",
        )
        .bind(game_id)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Most-recent screenshots across a whole profile, for a gallery view.
    pub async fn recent_screenshots(&self, profile_id: &str, limit: i64) -> Result<Vec<Screenshot>> {
        Ok(sqlx::query_as::<_, Screenshot>(
            "SELECT s.* FROM screenshots s
             JOIN games g ON g.id = s.game_id
             WHERE g.profile_id = ?
             ORDER BY s.captured_at DESC, s.created_at DESC
             LIMIT ?",
        )
        .bind(profile_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    /// Delete an indexed screenshot: remove the cached copy and the row. The
    /// emulator's original is left untouched.
    pub async fn delete_screenshot(&self, id: &str) -> Result<()> {
        let shot = self
            .get_screenshot(id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("screenshot {id}")))?;
        let _ = std::fs::remove_file(&shot.path);
        sqlx::query("DELETE FROM screenshots WHERE id = ?")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_screenshots_by_rom_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("screenshots");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("Super Mario 64.png"), b"img").unwrap();
        std::fs::write(dir.join("Super Mario 64-001.png"), b"img").unwrap();
        std::fs::write(dir.join("Super Mario 64 2024-01-01.jpg"), b"img").unwrap();
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        // Another game's shot must NOT match.
        std::fs::write(dir.join("Zelda.png"), b"img").unwrap();

        let found = match_screenshot_files(&[dir], "Super Mario 64");
        assert_eq!(found.len(), 3, "{found:?}");
    }

    #[tokio::test]
    async fn scan_and_list_roundtrip_is_idempotent() {
        let pool = crate::db::connect_in_memory().await.unwrap();
        let engine = Engine::with_pool(pool).await.unwrap();
        let profile = engine.ensure_default_profile().await.unwrap();

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Game.gba"), b"x").unwrap();
        engine
            .add_rom_source(&profile.id, dir.path().to_str().unwrap())
            .await
            .unwrap();
        engine.scan_library(&profile.id).await.unwrap();
        let game = engine
            .list_games(&crate::library::GameQuery {
                profile_id: profile.id.clone(),
                ..Default::default()
            })
            .await
            .unwrap()
            .remove(0);

        // Manually import a shot (bypasses adapter discovery, which needs a real
        // RetroArch install) and confirm listing + idempotent re-import.
        let shot_dir = tempfile::tempdir().unwrap();
        let shot = shot_dir.path().join("Game.png");
        std::fs::write(&shot, b"PNGDATA").unwrap();

        let first = engine.import_screenshot(&game.id, &shot).await.unwrap();
        assert!(first.is_some());
        let again = engine.import_screenshot(&game.id, &shot).await.unwrap();
        assert!(again.is_none(), "re-import must be a no-op");

        let list = engine.list_screenshots(&game.id).await.unwrap();
        assert_eq!(list.len(), 1);
        assert!(Path::new(&list[0].path).is_file(), "cache copy exists");

        engine.delete_screenshot(&list[0].id).await.unwrap();
        assert!(engine.list_screenshots(&game.id).await.unwrap().is_empty());
        assert!(!Path::new(&list[0].path).is_file(), "cache copy removed");
    }
}
