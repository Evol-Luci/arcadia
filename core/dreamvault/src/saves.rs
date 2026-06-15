//! Save Management (v0.5, first cut).
//!
//! Backs up and restores a game's save data (battery saves + savestates). The
//! governing rule is Risk-5: **never last-writer-wins**. Every restore first
//! takes an automatic snapshot of the *current* live saves (a `pre-restore`
//! backup) before overwriting them, so an overwrite can always be undone and no
//! data is ever silently lost.
//!
//! Discovery is best-effort and currently covers RetroArch (the broadest
//! adapter); standalone emulators store saves in many places and slot in later
//! behind the same engine API. The backup/restore/snapshot core operates on
//! plain file paths, independent of any emulator, so it is fully unit-testable.

use crate::error::{EngineError, Result};
use crate::models::{Emulator, Game, InstallSource, SaveBackup};
use crate::{new_id, now_rfc3339, Engine};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// One backed-up file and where it came from, so a restore can return it to its
/// original location even if the game is later launched on a different machine
/// layout. Stored as `manifest.json` inside each backup directory.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    /// Basename as stored under the backup's `files/` directory.
    pub file: String,
    /// Absolute path the file was copied from (and will be restored to).
    pub original: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub entries: Vec<ManifestEntry>,
}

/// Bytes + file count copied, for recording on the backup row.
#[derive(Debug, Clone, Copy, Default)]
struct CopyStats {
    files: i64,
    bytes: i64,
}

// ---- Pure file-operation core (no DB, fully tempdir-testable) -------------

/// Copy each existing `original` into `files_dir`, keyed by basename, returning
/// the manifest and copy stats. Originals that don't exist are skipped (a save
/// type the user never created). On a basename collision within one backup the
/// later file gets a keep-both name so nothing is overwritten.
fn copy_originals_into(originals: &[PathBuf], files_dir: &Path) -> Result<(Manifest, CopyStats)> {
    std::fs::create_dir_all(files_dir)?;
    let mut manifest = Manifest::default();
    let mut stats = CopyStats::default();
    for original in originals {
        if !original.is_file() {
            continue;
        }
        let base = match original.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let mut dest = files_dir.join(&base);
        if dest.exists() {
            dest = unique_sibling(&dest);
        }
        let bytes = std::fs::copy(original, &dest)?;
        manifest.entries.push(ManifestEntry {
            file: dest
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&base)
                .to_string(),
            original: original.to_string_lossy().to_string(),
        });
        stats.files += 1;
        stats.bytes += bytes as i64;
    }
    Ok((manifest, stats))
}

/// Restore every file named in `manifest` from `files_dir` back to its original
/// path. Parent directories are created as needed. Overwriting the live file is
/// safe here only because the caller has already snapshotted it (Risk-5).
fn restore_from_manifest(manifest: &Manifest, files_dir: &Path) -> Result<i64> {
    let mut restored = 0i64;
    for entry in &manifest.entries {
        let src = files_dir.join(&entry.file);
        if !src.is_file() {
            continue;
        }
        let dest = PathBuf::from(&entry.original);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&src, &dest)?;
        restored += 1;
    }
    Ok(restored)
}

/// Produce a sibling path that does not yet exist by inserting " (1)", " (2)",
/// … before the extension. Implements the "keep both" conflict policy.
fn unique_sibling(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
    let ext = path.extension().and_then(|e| e.to_str());
    for n in 1.. {
        let name = match ext {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = parent.join(name);
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

/// The Flatpak application id of an emulator install, or `None` for a native
/// one. Flatpak installs store their `executable_path` as the `flatpak run …`
/// line, with the app id as the third whitespace-separated token; this pulls it
/// back out so save/state directories under `~/.var/app/<id>/` can be resolved.
pub(crate) fn flatpak_app_id_of(emulator: &Emulator) -> Option<String> {
    if InstallSource::from_str(&emulator.install_source) == InstallSource::Flatpak {
        emulator
            .executable_path
            .split_whitespace()
            .nth(2)
            .map(String::from)
    } else {
        None
    }
}

/// RetroArch save/savestate directories for a native or Flatpak install.
pub(crate) fn retroarch_save_dirs(flatpak_app_id: Option<&str>) -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let base = match flatpak_app_id {
        Some(app) => home.join(".var/app").join(app).join("config/retroarch"),
        None => home.join(".config/retroarch"),
    };
    vec![base.join("saves"), base.join("states")]
}

/// Files in `dirs` belonging to the ROM whose basename (without extension) is
/// `stem`. RetroArch names saves `<stem>.srm`, savestates `<stem>.state[N]`,
/// RTC `<stem>.rtc`, etc., so we match any file whose name starts with
/// `"<stem>."`. Pure and order-stable for testing.
fn match_save_files(dirs: &[PathBuf], stem: &str) -> Vec<PathBuf> {
    let prefix = format!("{stem}.");
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
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with(&prefix) {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    out
}

impl Engine {
    /// Locate the live save files for a game, best-effort. Resolves the game's
    /// emulator/adapter and, for RetroArch, matches files in the save/state
    /// dirs by the ROM's basename. Returns empty for adapters we don't yet map
    /// or when no saves exist yet.
    pub async fn save_files_for_game(&self, game: &Game) -> Result<Vec<PathBuf>> {
        let emulator = match &game.emulator_id {
            Some(id) => self.get_emulator(id).await?,
            None => self.emulator_for_platform(&game.platform).await?,
        };
        let Some(emulator) = emulator else {
            return Ok(Vec::new());
        };
        if emulator.adapter_id != "retroarch" {
            // Standalone save locations vary per emulator; not in the first cut.
            //
            // FUTURE (planned — see "Save discovery per adapter" in
            // ARCADIA_Build_Plan.md): extend backup to standalone emulators by
            // reusing the *same* keying that Recall State already uses to locate
            // savestates — no emulator-backend coupling, no format parsing:
            //   - stem-keyed (Snes9x): `<rom-stem>.*`
            //   - disc-key-keyed (PCSX2/DuckStation/Dolphin): serial/game-id from
            //     the disc image (`iso9660::disc_serial`, `nintendo_game_id`)
            //   - learned-key (Mupen64Plus): the `<GoodName>-<CRC>` base we
            //     observe-and-learn at session end and persist in `state_keys`.
            //     Battery saves (`.eep`/`.mpk`/`.sra`/`.fla`) share that base, so
            //     the learned row already locates them — this case is unblocked.
            // Implementation: replace this early-return with a per-adapter
            // resolver (adapter save dir + key) mirroring `save_states.rs`.
            return Ok(Vec::new());
        }
        let flatpak_app_id = flatpak_app_id_of(&emulator);
        let stem = Path::new(&game.rom_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&game.title);
        let dirs = retroarch_save_dirs(flatpak_app_id.as_deref());
        Ok(match_save_files(&dirs, stem))
    }

    /// Base directory for a game's backups: `<data_dir>/saves/<game_id>`.
    fn save_backup_root(&self, game_id: &str) -> PathBuf {
        self.paths.data_dir.join("saves").join(game_id)
    }

    /// Copy a set of live files into a fresh, uniquely-named backup directory and
    /// record it. Shared by manual backups and the automatic pre-restore
    /// snapshot. Returns `None` when there is nothing to back up.
    async fn record_backup(
        &self,
        game_id: &str,
        kind: &str,
        files: &[PathBuf],
        note: Option<&str>,
    ) -> Result<Option<SaveBackup>> {
        if files.is_empty() {
            return Ok(None);
        }
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let mut dir = self.save_backup_root(game_id).join(format!("{stamp}-{kind}"));
        if dir.exists() {
            dir = unique_sibling(&dir);
        }
        let files_dir = dir.join("files");
        let (manifest, stats) = copy_originals_into(files, &files_dir)?;
        if stats.files == 0 {
            // Every candidate vanished between discovery and copy; don't record
            // an empty backup or leave a stray dir.
            let _ = std::fs::remove_dir_all(&dir);
            return Ok(None);
        }
        let manifest_json = serde_json::to_string_pretty(&manifest)?;
        std::fs::write(dir.join("manifest.json"), manifest_json)?;

        let id = new_id();
        let created_at = now_rfc3339();
        let path = dir.to_string_lossy().to_string();
        sqlx::query(
            "INSERT INTO save_backups (id, game_id, kind, path, file_count, byte_size, note, created_at)
             VALUES (?,?,?,?,?,?,?,?)",
        )
        .bind(&id)
        .bind(game_id)
        .bind(kind)
        .bind(&path)
        .bind(stats.files)
        .bind(stats.bytes)
        .bind(note)
        .bind(&created_at)
        .execute(&self.pool)
        .await?;

        self.get_save_backup(&id).await
    }

    /// Manually back up a game's current saves. Returns `None` if the game has
    /// no save files yet (nothing to do).
    pub async fn backup_game_saves(
        &self,
        game_id: &str,
        note: Option<&str>,
    ) -> Result<Option<SaveBackup>> {
        let game = self
            .get_game(game_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("game {game_id}")))?;
        let files = self.save_files_for_game(&game).await?;
        self.record_backup(game_id, "manual", &files, note).await
    }

    pub async fn list_save_backups(&self, game_id: &str) -> Result<Vec<SaveBackup>> {
        Ok(sqlx::query_as::<_, SaveBackup>(
            "SELECT * FROM save_backups WHERE game_id = ? ORDER BY created_at DESC",
        )
        .bind(game_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_save_backup(&self, id: &str) -> Result<Option<SaveBackup>> {
        Ok(sqlx::query_as::<_, SaveBackup>("SELECT * FROM save_backups WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Restore a backup over the live saves. **Risk-5**: the current live saves
    /// (the files this backup's manifest will overwrite) are snapshotted into a
    /// new `pre-restore` backup *first*, so the restore is always reversible.
    /// Returns the pre-restore snapshot, if any was taken, so callers can tell
    /// the user their prior saves were preserved.
    pub async fn restore_save_backup(&self, backup_id: &str) -> Result<Option<SaveBackup>> {
        let backup = self
            .get_save_backup(backup_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("save backup {backup_id}")))?;
        let dir = PathBuf::from(&backup.path);
        let files_dir = dir.join("files");
        let manifest: Manifest = {
            let text = std::fs::read_to_string(dir.join("manifest.json"))
                .map_err(|e| EngineError::Invalid(format!("backup manifest unreadable: {e}")))?;
            serde_json::from_str(&text)
                .map_err(|e| EngineError::Invalid(format!("backup manifest corrupt: {e}")))?
        };

        // 1. Snapshot the current live state of exactly the paths we're about to
        //    overwrite. This is what makes the overwrite safe.
        let live: Vec<PathBuf> = manifest
            .entries
            .iter()
            .map(|e| PathBuf::from(&e.original))
            .filter(|p| p.is_file())
            .collect();
        let snapshot = self
            .record_backup(
                &backup.game_id,
                "pre-restore",
                &live,
                Some("auto-snapshot before restore"),
            )
            .await?;

        // 2. Now overwrite the live files from the chosen backup.
        restore_from_manifest(&manifest, &files_dir)?;
        Ok(snapshot)
    }

    /// Delete a backup: remove its directory and row.
    pub async fn delete_save_backup(&self, backup_id: &str) -> Result<()> {
        let backup = self
            .get_save_backup(backup_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("save backup {backup_id}")))?;
        let _ = std::fs::remove_dir_all(&backup.path);
        sqlx::query("DELETE FROM save_backups WHERE id = ?")
            .bind(backup_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_save_files_by_rom_stem() {
        let tmp = tempfile::tempdir().unwrap();
        let saves = tmp.path().join("saves");
        let states = tmp.path().join("states");
        std::fs::create_dir_all(&saves).unwrap();
        std::fs::create_dir_all(&states).unwrap();
        std::fs::write(saves.join("Super Mario 64.srm"), b"battery").unwrap();
        std::fs::write(states.join("Super Mario 64.state"), b"state").unwrap();
        std::fs::write(states.join("Super Mario 64.state1"), b"state1").unwrap();
        // A different game's save must NOT be picked up.
        std::fs::write(saves.join("Zelda.srm"), b"other").unwrap();

        let found = match_save_files(&[saves, states], "Super Mario 64");
        assert_eq!(found.len(), 3);
        assert!(found.iter().all(|p| p
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Super Mario 64.")));
    }

    #[test]
    fn unique_sibling_avoids_clobbering() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("game.srm");
        std::fs::write(&p, b"a").unwrap();
        let s1 = unique_sibling(&p);
        assert_eq!(s1.file_name().unwrap().to_str().unwrap(), "game (1).srm");
        std::fs::write(&s1, b"b").unwrap();
        let s2 = unique_sibling(&p);
        assert_eq!(s2.file_name().unwrap().to_str().unwrap(), "game (2).srm");
    }

    /// The Risk-5 guarantee, exercised end-to-end on the pure core: backing up
    /// "v1", letting the live save advance to "v2", then restoring "v1" must
    /// (a) put "v1" back AND (b) preserve "v2" in a snapshot — never lose data.
    #[test]
    fn restore_snapshots_live_state_before_overwriting() {
        let tmp = tempfile::tempdir().unwrap();
        let live = tmp.path().join("live/Game.srm");
        std::fs::create_dir_all(live.parent().unwrap()).unwrap();
        std::fs::write(&live, b"v1").unwrap();

        // Back up v1.
        let backup_files = tmp.path().join("backup/files");
        let (manifest, stats) =
            copy_originals_into(&[live.clone()], &backup_files).unwrap();
        assert_eq!(stats.files, 1);

        // Live save advances to v2.
        std::fs::write(&live, b"v2").unwrap();

        // Snapshot current (v2) before restoring — the pre-restore step.
        let snap_files = tmp.path().join("snapshot/files");
        let (snap_manifest, snap_stats) =
            copy_originals_into(&[live.clone()], &snap_files).unwrap();
        assert_eq!(snap_stats.files, 1);

        // Restore v1 over live.
        restore_from_manifest(&manifest, &backup_files).unwrap();
        assert_eq!(std::fs::read(&live).unwrap(), b"v1");

        // v2 is preserved in the snapshot — nothing was lost.
        let snap_file = snap_files.join(&snap_manifest.entries[0].file);
        assert_eq!(std::fs::read(snap_file).unwrap(), b"v2");
    }

    #[test]
    fn copy_skips_missing_originals() {
        let tmp = tempfile::tempdir().unwrap();
        let exists = tmp.path().join("a.srm");
        std::fs::write(&exists, b"x").unwrap();
        let missing = tmp.path().join("ghost.srm");
        let dest = tmp.path().join("out");
        let (manifest, stats) =
            copy_originals_into(&[exists, missing], &dest).unwrap();
        assert_eq!(stats.files, 1);
        assert_eq!(manifest.entries.len(), 1);
    }
}
