//! User-owned configuration persisted as JSON under the config dir.
//!
//! Online metadata providers need credentials that belong to the *user*, not to
//! Arcadia: their own ScreenScraper account / dev key, used under their own rate
//! quota. We store them here (never bake keys into the binary) and the provider
//! degrades to a no-op when they're absent.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const CONFIG_FILE: &str = "config.json";

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AppConfig {
    #[serde(default)]
    pub screenscraper: ScreenScraperCredentials,
    #[serde(default)]
    pub retroachievements: RetroAchievementsCredentials,
    #[serde(default)]
    pub controller: ControllerConfig,
    #[serde(default)]
    pub sync: SyncConfig,
    #[serde(default)]
    pub launchbox: LaunchBoxConfig,
    /// Ids of community plugins the user has explicitly enabled. Plugins are
    /// disabled until opted-in (sandboxed, but opt-in is a clear trust gate).
    #[serde(default)]
    pub enabled_plugins: Vec<String>,
    /// User's preferred emulator per platform, keyed by platform id → emulator
    /// id. Consulted first when resolving which emulator launches a game; an
    /// absent or now-uninstalled entry falls back to the platform's adapter hint.
    #[serde(default)]
    pub default_emulators: std::collections::BTreeMap<String, String>,
}

/// RetroAchievements web-API credentials. The username plus a personal Web API
/// key (from the user's RA account settings) are required; we ship none, so the
/// Achievement Hub stays dark until the user provides their own.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RetroAchievementsCredentials {
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub api_key: String,
}

impl RetroAchievementsCredentials {
    pub fn is_configured(&self) -> bool {
        !self.username.trim().is_empty() && !self.api_key.trim().is_empty()
    }
}

/// User-defined controller button-mapping profiles. Arcadia stores the mapping;
/// the emulator still owns input at runtime — these document and drive the UI's
/// big-picture navigation hints, not a re-implementation of input handling.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControllerConfig {
    #[serde(default)]
    pub profiles: Vec<ControllerProfile>,
    /// Id of the active profile, if any.
    #[serde(default)]
    pub active_profile: Option<String>,
    /// Per-system controller mappings — the source the input-setup module
    /// materializes into each emulator's config. Distinct from `profiles`
    /// (app-navigation hints) in lifecycle and key space: these keys are
    /// constrained to a console's [`crate::console_pads::ConsolePad`] inputs.
    #[serde(default)]
    pub system_profiles: Vec<SystemControllerProfile>,
    /// Which system-profile is active for each system: platform id -> profile id.
    #[serde(default)]
    pub system_assignments: std::collections::BTreeMap<String, String>,
    /// Policy for the SDL-HIDAPI vs xpadneo launch workaround (SDL-input
    /// emulators only). Defaults to auto-detect so it fires only on the affected
    /// setup and stays out of the way of everyone else's working controllers.
    #[serde(default)]
    pub sdl_hidapi_workaround: HidapiWorkaround,
}

/// A per-console button mapping. Unlike [`ControllerProfile`] (logical app-nav
/// actions) the keys here are a console's canonical input ids
/// ([`crate::console_pads::ConsolePad`]) and the values are compact, device-
/// agnostic **W3C Standard Gamepad** descriptors captured from the live pad:
/// `"btn:N"` for a button index, `"axis:N+"` / `"axis:N-"` for an axis half.
/// The physical device's identity (SDL GUID/name) is read live at materialize
/// time, never stored here — it can change between sessions.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SystemControllerProfile {
    pub id: String,
    pub name: String,
    /// Platform id this profile targets (matches a `ConsolePad.system`).
    pub system: String,
    /// console_input_id -> W3C descriptor ("btn:0", "axis:1-").
    #[serde(default)]
    pub bindings: std::collections::BTreeMap<String, String>,
}

/// When to force SDL's evdev joystick backend (`SDL_JOYSTICK_HIDAPI=0`) for
/// SDL-input emulators. A Bluetooth Xbox pad on the xpadneo kernel driver is
/// enumerated by SDL's HIDAPI driver but delivers zero input through it; the
/// evdev backend restores a full event stream. Other controllers are usually
/// fine on HIDAPI (and some need it), so forcing it off unconditionally would be
/// a regression — hence the default is to detect the affected setup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HidapiWorkaround {
    /// Apply only when an xpadneo-driven controller is currently connected.
    #[default]
    Auto,
    /// Always apply for SDL-input emulators.
    Force,
    /// Never apply.
    Off,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControllerProfile {
    pub id: String,
    pub name: String,
    /// Logical action -> physical button label (e.g. "confirm" -> "A").
    #[serde(default)]
    pub bindings: std::collections::BTreeMap<String, String>,
}

/// Cloud-sync settings. Arcadia does not run its own sync daemon: the user
/// points it at a folder that Syncthing / Nextcloud / Dropbox already keeps in
/// sync, and Arcadia mirrors its save backups and config into it, conflict-aware
/// (Risk-5: never last-writer-wins).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Absolute path to the externally-synced folder, if configured.
    #[serde(default)]
    pub folder: Option<String>,
}

impl SyncConfig {
    pub fn is_configured(&self) -> bool {
        self.enabled && self.folder.as_deref().map(|f| !f.trim().is_empty()).unwrap_or(false)
    }
}

/// ScreenScraper API credentials. The API requires a developer id/password
/// (issued per-application by ScreenScraper); a user account is optional but
/// raises the per-request quota. All four are supplied by the user — we ship
/// none — so an out-of-the-box install simply skips ScreenScraper.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScreenScraperCredentials {
    #[serde(default)]
    pub dev_id: String,
    #[serde(default)]
    pub dev_password: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub user_password: String,
}

impl ScreenScraperCredentials {
    /// Usable only if the mandatory developer credentials are present.
    pub fn is_configured(&self) -> bool {
        !self.dev_id.trim().is_empty() && !self.dev_password.trim().is_empty()
    }
}

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

impl AppConfig {
    fn file_path(config_dir: &Path) -> PathBuf {
        config_dir.join(CONFIG_FILE)
    }

    /// Load config from `<config_dir>/config.json`. A missing or unparsable
    /// file yields defaults rather than an error — config is best-effort.
    pub fn load(config_dir: &Path) -> Self {
        let path = Self::file_path(config_dir);
        match std::fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                tracing::warn!(?path, error = %e, "config unparsable, using defaults");
                AppConfig::default()
            }),
            Err(_) => AppConfig::default(),
        }
    }

    /// Persist config to `<config_dir>/config.json`, creating the dir if needed.
    pub fn save(&self, config_dir: &Path) -> Result<()> {
        std::fs::create_dir_all(config_dir)?;
        let text = serde_json::to_string_pretty(self)?;
        std::fs::write(Self::file_path(config_dir), text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_yields_defaults() {
        let dir = std::env::temp_dir().join(format!("arcadia_cfg_missing_{}", std::process::id()));
        let cfg = AppConfig::load(&dir);
        assert!(!cfg.screenscraper.is_configured());
    }

    #[test]
    fn save_then_load_roundtrips_credentials() {
        let dir = std::env::temp_dir().join(format!("arcadia_cfg_rt_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut cfg = AppConfig::default();
        cfg.screenscraper.dev_id = "devid".into();
        cfg.screenscraper.dev_password = "devpw".into();
        cfg.screenscraper.user_id = "luci".into();
        cfg.save(&dir).unwrap();

        let loaded = AppConfig::load(&dir);
        assert!(loaded.screenscraper.is_configured());
        assert_eq!(loaded.screenscraper.dev_id, "devid");
        assert_eq!(loaded.screenscraper.user_id, "luci");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unconfigured_without_dev_credentials() {
        let mut c = ScreenScraperCredentials::default();
        c.user_id = "luci".into();
        assert!(!c.is_configured());
        c.dev_id = "d".into();
        c.dev_password = "p".into();
        assert!(c.is_configured());
    }

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
}
