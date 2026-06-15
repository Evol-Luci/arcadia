//! Cloud Sync (v1.0) — orchestration, not a sync daemon.
//!
//! Arcadia does **not** implement its own transport. Per the build plan it
//! orchestrates existing systems: the user points it at a folder that Syncthing,
//! Nextcloud, Dropbox, etc. already keep in sync, and Arcadia mirrors its save
//! backups and config into that folder. The remote is just a directory.
//!
//! Conflict policy honours Risk-5 (*never last-writer-wins*): save backups are
//! immutable, timestamped directories, so push/pull is an **additive merge** —
//! an entry is copied only when the destination doesn't already exist, so
//! nothing is ever overwritten. The mutable `config.json` is synced keep-both:
//! a differing remote copy is preserved alongside (`config.remote.json`) rather
//! than clobbered.

use crate::config::SyncConfig;
use crate::error::{EngineError, Result};
use crate::Engine;
use serde::Serialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncStatus {
    pub configured: bool,
    pub folder: Option<String>,
    pub folder_exists: bool,
    pub local_backup_count: i64,
    pub remote_backup_count: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct SyncReport {
    pub files_copied: i64,
    pub conflicts: i64,
}

/// Recursively copy files from `src` into `dst`, creating directories as needed,
/// **without overwriting** any file that already exists at the destination.
/// Returns (files copied, conflicts skipped). Additive merge — safe by design.
fn merge_tree_additive(src: &Path, dst: &Path) -> Result<(i64, i64)> {
    let mut copied = 0i64;
    let mut conflicts = 0i64;
    if !src.is_dir() {
        return Ok((0, 0));
    }
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)?.flatten() {
        let from = entry.path();
        let name = entry.file_name();
        let to = dst.join(&name);
        if from.is_dir() {
            let (c, k) = merge_tree_additive(&from, &to)?;
            copied += c;
            conflicts += k;
        } else if from.is_file() {
            if to.exists() {
                // Same path already present remotely/locally. Backups are
                // immutable, so identical content is the norm; only flag a real
                // content difference as a conflict (and leave both intact).
                if files_differ(&from, &to) {
                    conflicts += 1;
                }
            } else {
                std::fs::copy(&from, &to)?;
                copied += 1;
            }
        }
    }
    Ok((copied, conflicts))
}

fn files_differ(a: &Path, b: &Path) -> bool {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x != y,
        _ => true,
    }
}

impl Engine {
    fn sync_config(&self) -> SyncConfig {
        self.load_config().sync
    }

    /// Resolve and validate the configured remote root (`<folder>/arcadia`).
    fn remote_root(&self) -> Result<PathBuf> {
        let cfg = self.sync_config();
        if !cfg.is_configured() {
            return Err(EngineError::Invalid("cloud sync is not configured".into()));
        }
        let folder = cfg.folder.unwrap();
        Ok(PathBuf::from(folder).join("arcadia"))
    }

    fn local_saves_root(&self) -> PathBuf {
        self.paths.data_dir.join("saves")
    }

    fn config_file(&self) -> PathBuf {
        self.paths.config_dir.join("config.json")
    }

    /// Count immediate subdirectories of a backups root (one per game).
    fn count_backup_dirs(root: &Path) -> i64 {
        let mut n = 0i64;
        if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    n += 1;
                }
            }
        }
        n
    }

    pub fn sync_status(&self) -> SyncStatus {
        let cfg = self.sync_config();
        let configured = cfg.is_configured();
        let folder = cfg.folder.clone();
        let remote_saves = folder
            .as_ref()
            .map(|f| PathBuf::from(f).join("arcadia").join("saves"));
        SyncStatus {
            configured,
            folder_exists: folder.as_ref().map(|f| Path::new(f).is_dir()).unwrap_or(false),
            local_backup_count: Self::count_backup_dirs(&self.local_saves_root()),
            remote_backup_count: remote_saves
                .map(|p| Self::count_backup_dirs(&p))
                .unwrap_or(0),
            folder,
        }
    }

    /// Push local save backups + config into the synced folder (additive).
    pub fn sync_push(&self) -> Result<SyncReport> {
        let remote = self.remote_root()?;
        std::fs::create_dir_all(&remote)?;
        let (copied, conflicts) =
            merge_tree_additive(&self.local_saves_root(), &remote.join("saves"))?;
        let conflicts = conflicts + self.sync_config_file(&self.config_file(), &remote.join("config.json"))?;
        Ok(SyncReport { files_copied: copied, conflicts })
    }

    /// Pull save backups + config from the synced folder into the local store
    /// (additive). Local backups are never overwritten.
    pub fn sync_pull(&self) -> Result<SyncReport> {
        let remote = self.remote_root()?;
        let (copied, conflicts) =
            merge_tree_additive(&remote.join("saves"), &self.local_saves_root())?;
        let conflicts = conflicts + self.sync_config_file(&remote.join("config.json"), &self.config_file())?;
        Ok(SyncReport { files_copied: copied, conflicts })
    }

    /// Keep-both config sync: if `dst` is absent, copy `src` over. If both exist
    /// and differ, preserve `src` beside `dst` as `<name>.incoming.json` rather
    /// than overwriting — the user reconciles credentials manually. Returns the
    /// conflict count (0 or 1).
    fn sync_config_file(&self, src: &Path, dst: &Path) -> Result<i64> {
        if !src.is_file() {
            return Ok(0);
        }
        if let Some(parent) = dst.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if !dst.exists() {
            std::fs::copy(src, dst)?;
            return Ok(0);
        }
        if files_differ(src, dst) {
            let stem = dst.file_stem().and_then(|s| s.to_str()).unwrap_or("config");
            let sidecar = dst.with_file_name(format!("{stem}.incoming.json"));
            std::fs::copy(src, sidecar)?;
            return Ok(1);
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additive_merge_never_overwrites_and_flags_conflicts() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        let dst = tmp.path().join("dst");
        std::fs::create_dir_all(src.join("gameA")).unwrap();
        std::fs::create_dir_all(dst.join("gameA")).unwrap();
        // New file -> copied.
        std::fs::write(src.join("gameA/new.srm"), b"new").unwrap();
        // Identical file present both sides -> skipped, no conflict.
        std::fs::write(src.join("gameA/same.srm"), b"same").unwrap();
        std::fs::write(dst.join("gameA/same.srm"), b"same").unwrap();
        // Differing file -> conflict, dst left intact.
        std::fs::write(src.join("gameA/diff.srm"), b"incoming").unwrap();
        std::fs::write(dst.join("gameA/diff.srm"), b"existing").unwrap();

        let (copied, conflicts) = merge_tree_additive(&src, &dst).unwrap();
        assert_eq!(copied, 1);
        assert_eq!(conflicts, 1);
        assert_eq!(std::fs::read(dst.join("gameA/new.srm")).unwrap(), b"new");
        // The differing destination file was NOT overwritten.
        assert_eq!(std::fs::read(dst.join("gameA/diff.srm")).unwrap(), b"existing");
    }
}
