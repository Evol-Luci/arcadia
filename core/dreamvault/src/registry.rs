//! Emulator Registry — tracks all installed emulators and honours install
//! ownership. Detection delegates to the native adapters; the registry owns
//! identity and persistence.

use crate::error::Result;
use crate::models::{Emulator, InstallSource};
use crate::{now_rfc3339, Engine};
use serde::Serialize;

/// The set of installed emulators able to launch one platform, plus the user's
/// configured default for it. Drives the "Default Emulators" settings panel.
#[derive(Debug, Clone, Serialize)]
pub struct PlatformEmulators {
    /// Platform slug (e.g. "snes").
    pub platform: String,
    /// Human-readable platform name.
    pub name: String,
    /// Adapter id Arcadia auto-prefers for this platform when no default is set.
    pub hint_adapter_id: String,
    /// The user's chosen default emulator id, if any.
    pub default_emulator_id: Option<String>,
    /// Installed emulators whose adapter advertises this platform.
    pub emulators: Vec<Emulator>,
}

impl Engine {
    /// Run detection across every built-in adapter and upsert the results.
    /// Returns the full, current emulator list. Idempotent: re-running updates
    /// versions in place rather than duplicating rows.
    pub async fn detect_emulators(&self) -> Result<Vec<Emulator>> {
        let now = now_rfc3339();
        for adapter in self.adapters.all() {
            let installs = adapter.detect().await?;
            for inst in installs {
                let id = uuid::Uuid::new_v4().to_string();
                let desc = adapter.descriptor();
                let caps = desc.capabilities;
                // Upsert keyed by (adapter_id, executable_path).
                sqlx::query(
                    r#"
                    INSERT INTO emulators (
                        id, name, version, install_source, owner, executable_path,
                        adapter_id, supports_savestates, supports_saves,
                        supports_achievements, supports_screenshots, detected_at
                    ) VALUES (?,?,?,?,?,?,?,?,?,?,?,?)
                    ON CONFLICT(adapter_id, executable_path) DO UPDATE SET
                        name = excluded.name,
                        version = excluded.version,
                        install_source = excluded.install_source,
                        owner = excluded.owner,
                        supports_savestates = excluded.supports_savestates,
                        supports_saves = excluded.supports_saves,
                        supports_achievements = excluded.supports_achievements,
                        supports_screenshots = excluded.supports_screenshots,
                        detected_at = excluded.detected_at
                    "#,
                )
                .bind(&id)
                .bind(&inst.name)
                .bind(&inst.version)
                .bind(inst.install_source.as_str())
                .bind(inst.install_source.owner())
                .bind(&inst.executable_path)
                .bind(&inst.adapter_id)
                .bind(caps.savestates)
                .bind(caps.saves)
                .bind(caps.achievements)
                .bind(caps.screenshots)
                .bind(&now)
                .execute(&self.pool)
                .await?;
            }
        }
        self.list_emulators().await
    }

    pub async fn list_emulators(&self) -> Result<Vec<Emulator>> {
        let rows = sqlx::query_as::<_, Emulator>(
            "SELECT * FROM emulators ORDER BY name COLLATE NOCASE",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_emulator(&self, id: &str) -> Result<Option<Emulator>> {
        let row = sqlx::query_as::<_, Emulator>("SELECT * FROM emulators WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Find an emulator able to launch `platform`, preferring the user's
    /// configured default, then the platform's adapter hint, then any adapter
    /// that lists the platform.
    pub async fn emulator_for_platform(&self, platform: &str) -> Result<Option<Emulator>> {
        let installed = self.list_emulators().await?;
        if installed.is_empty() {
            return Ok(None);
        }

        // A user-pinned default wins — but only while it's still installed; a
        // stale id (DB reset, emulator removed) silently defers to the hints.
        if let Some(emu_id) = self.load_config().default_emulators.get(platform) {
            if let Some(e) = installed.iter().find(|e| &e.id == emu_id) {
                return Ok(Some(e.clone()));
            }
        }

        if let Some(def) = crate::platforms::by_id(platform) {
            if let Some(e) = installed.iter().find(|e| e.adapter_id == def.adapter_hint) {
                return Ok(Some(e.clone()));
            }
        }

        // Fall back to any installed emulator whose adapter supports the platform.
        for e in &installed {
            if let Some(adapter) = self.adapters.get(&e.adapter_id) {
                if adapter.descriptor().platforms.iter().any(|p| p == platform) {
                    return Ok(Some(e.clone()));
                }
            }
        }
        Ok(None)
    }

    /// Set (or clear, with `None`) the user's default emulator for a platform.
    /// Persisted to config; consulted first by [`emulator_for_platform`].
    pub fn set_default_emulator(&self, platform: &str, emulator_id: Option<&str>) -> Result<()> {
        let mut cfg = self.load_config();
        match emulator_id {
            Some(id) => {
                cfg.default_emulators.insert(platform.to_string(), id.to_string());
            }
            None => {
                cfg.default_emulators.remove(platform);
            }
        }
        self.save_config(&cfg)
    }

    /// For each platform with at least one compatible installed emulator, the
    /// list of those emulators plus the user's current default. Platforms with
    /// no installed emulator are omitted (nothing to choose). Ordered by the
    /// canonical platform catalogue.
    pub async fn platform_emulator_options(&self) -> Result<Vec<PlatformEmulators>> {
        let installed = self.list_emulators().await?;
        let defaults = self.load_config().default_emulators;
        let mut out = Vec::new();
        for def in crate::platforms::PLATFORMS {
            let emulators: Vec<Emulator> = installed
                .iter()
                .filter(|e| {
                    self.adapters
                        .get(&e.adapter_id)
                        .map(|a| a.descriptor().platforms.iter().any(|p| p == def.id))
                        .unwrap_or(false)
                })
                .cloned()
                .collect();
            if emulators.is_empty() {
                continue;
            }
            out.push(PlatformEmulators {
                platform: def.id.to_string(),
                name: def.name.to_string(),
                hint_adapter_id: def.adapter_hint.to_string(),
                default_emulator_id: defaults.get(def.id).cloned(),
                emulators,
            });
        }
        Ok(out)
    }

    /// Surface the update-ownership rule for the UI. Arcadia never updates an
    /// externally-installed emulator; it shows the command instead.
    pub fn update_hint(emulator: &Emulator) -> Option<String> {
        match InstallSource::from_str(&emulator.install_source) {
            InstallSource::Pacman => Some("sudo pacman -Syu".to_string()),
            InstallSource::Yay => Some("yay -Syu".to_string()),
            InstallSource::Flatpak => Some("flatpak update".to_string()),
            InstallSource::AppImage | InstallSource::Manual => None,
            InstallSource::ArcadiaManaged => None,
        }
    }
}
