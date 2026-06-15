use crate::error::{EngineError, Result};
use directories::ProjectDirs;
use std::path::PathBuf;

/// Resolved XDG base directories for Arcadia. We follow the XDG Base Directory
/// spec and never invent `~/.arcadia`. `directories` already honours the
/// `XDG_*` env overrides and Flatpak sandbox paths.
#[derive(Debug, Clone)]
pub struct ArcadiaPaths {
    /// $XDG_CONFIG_HOME/arcadia
    pub config_dir: PathBuf,
    /// $XDG_DATA_HOME/arcadia — sqlite db, metadata cache, save backups
    pub data_dir: PathBuf,
    /// $XDG_CACHE_HOME/arcadia — artwork/thumbnails (safe to delete)
    pub cache_dir: PathBuf,
    /// $XDG_STATE_HOME/arcadia — logs
    pub state_dir: PathBuf,
}

impl ArcadiaPaths {
    pub fn discover() -> Result<Self> {
        let pd = ProjectDirs::from("", "Arcadia", "arcadia").ok_or(EngineError::NoBaseDirs)?;
        // state_dir is None on some platforms; fall back to data_dir/state.
        let state_dir = pd
            .state_dir()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| pd.data_dir().join("state"));
        Ok(Self {
            config_dir: pd.config_dir().to_path_buf(),
            data_dir: pd.data_dir().to_path_buf(),
            cache_dir: pd.cache_dir().to_path_buf(),
            state_dir,
        })
    }

    /// Create all base directories if missing.
    pub fn ensure(&self) -> Result<()> {
        for dir in [
            &self.config_dir,
            &self.data_dir,
            &self.cache_dir,
            &self.state_dir,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }

    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join("arcadia.db")
    }

    /// Where cached cover art lives.
    pub fn artwork_dir(&self) -> PathBuf {
        self.cache_dir.join("artwork")
    }

    /// Where imported screenshots are copied so the webview can load them via
    /// the asset protocol (the emulator's own screenshot dir is outside scope).
    pub fn screenshots_dir(&self) -> PathBuf {
        self.cache_dir.join("screenshots")
    }

    /// Where generated multi-disc `.m3u` playlists live. Cache-grade: rebuilt
    /// from the DB on demand at launch, so it's safe to delete. Keeping them out
    /// of the user's ROM folders honours "never modify the user's files".
    pub fn playlists_dir(&self) -> PathBuf {
        self.cache_dir.join("playlists")
    }
}
