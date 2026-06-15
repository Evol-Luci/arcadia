//! Controller Center (v1.0).
//!
//! Arcadia stores named button-mapping *profiles*; it does not intercept input
//! at runtime — the emulator and the OS own that. These profiles document a
//! user's preferred layout and drive the big-picture UI's on-screen hints. The
//! live controller view in the shell reads connected pads through the browser
//! Gamepad API; the engine's job here is just durable storage of the profiles.
//!
//! TODO(controller-bridge): these profiles are NOT yet passed to the emulator at
//! launch — the launch path (`stats.rs` → `LaunchContext` → `adapters`) sends
//! nothing about controllers, so the emulator falls back to its own input
//! autoconfig. When we wire profiles into launch, the hard part is index
//! translation: profiles are in W3C Standard Gamepad terms (browser Gamepad
//! API), but emulators want raw, device- and SDL-version-specific joystick
//! indices (e.g. Mupen64Plus `button(7)`/`axis(2+)`/`hat(0 Up)` in
//! `[Input-SDL-Control1]`). Plan: per-adapter `apply_controller` that writes a
//! *scratch* input config and points the emulator at it via its config-dir flag
//! (Mupen `--configdir`, RetroArch `--appendconfig`, …) so the user's own config
//! is never clobbered. Full write-up in `Design_Discussions_00.md` →
//! "Future Work: Passing Controller Information to the Emulator at Launch".
//!
//! Related precedent: a narrower launch-time controller fix already ships — the
//! standalone adapter injects `SDL_JOYSTICK_HIDAPI=0` for SDL-input emulators to
//! dodge an SDL-HIDAPI vs xpadneo conflict that left Bluetooth Xbox pads dead in
//! Mupen64Plus (see `adapters/standalone.rs` `apply_sdl_hidapi_workaround` and
//! the "Troubleshooting Log" in `Design_Discussions_00.md`). It is gated by
//! [`crate::config::HidapiWorkaround`] — auto-detecting an xpadneo-bound pad by
//! default so it never disturbs other controllers — and uses the same
//! `LaunchContext.env` channel the full controller bridge will build on.

use crate::config::{AppConfig, ControllerConfig, ControllerProfile};
use crate::error::{EngineError, Result};
use crate::{new_id, Engine};

impl Engine {
    pub fn controller_config(&self) -> ControllerConfig {
        self.load_config().controller
    }

    /// Create or update a mapping profile. A blank `id` mints a new one and
    /// returns it; an existing id updates in place.
    pub fn save_controller_profile(&self, mut profile: ControllerProfile) -> Result<ControllerProfile> {
        if profile.name.trim().is_empty() {
            return Err(EngineError::Invalid("controller profile needs a name".into()));
        }
        let mut cfg: AppConfig = self.load_config();
        if profile.id.trim().is_empty() {
            profile.id = new_id();
        }
        match cfg.controller.profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile.clone(),
            None => cfg.controller.profiles.push(profile.clone()),
        }
        self.save_config(&cfg)?;
        Ok(profile)
    }

    pub fn delete_controller_profile(&self, id: &str) -> Result<()> {
        let mut cfg: AppConfig = self.load_config();
        cfg.controller.profiles.retain(|p| p.id != id);
        if cfg.controller.active_profile.as_deref() == Some(id) {
            cfg.controller.active_profile = None;
        }
        self.save_config(&cfg)
    }

    /// Set the active profile (or clear it with `None`). Rejects unknown ids.
    pub fn set_active_controller_profile(&self, id: Option<&str>) -> Result<()> {
        let mut cfg: AppConfig = self.load_config();
        if let Some(id) = id {
            if !cfg.controller.profiles.iter().any(|p| p.id == id) {
                return Err(EngineError::NotFound(format!("controller profile {id}")));
            }
        }
        cfg.controller.active_profile = id.map(String::from);
        self.save_config(&cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn engine_with_tempdirs() -> (Engine, tempfile::TempDir) {
        // Point config at a temp dir so the test never touches the real config.
        let tmp = tempfile::tempdir().unwrap();
        let pool = crate::db::connect_in_memory().await.unwrap();
        let mut engine = Engine::with_pool(pool).await.unwrap();
        engine.paths.config_dir = tmp.path().to_path_buf();
        (engine, tmp)
    }

    #[tokio::test]
    async fn profile_crud_roundtrip() {
        let (engine, _tmp) = engine_with_tempdirs().await;
        let mut prof = ControllerProfile {
            id: String::new(),
            name: "Couch".into(),
            bindings: Default::default(),
        };
        prof.bindings.insert("confirm".into(), "A".into());
        let saved = engine.save_controller_profile(prof).unwrap();
        assert!(!saved.id.is_empty());

        engine.set_active_controller_profile(Some(&saved.id)).unwrap();
        let cfg = engine.controller_config();
        assert_eq!(cfg.profiles.len(), 1);
        assert_eq!(cfg.active_profile.as_deref(), Some(saved.id.as_str()));

        engine.delete_controller_profile(&saved.id).unwrap();
        let cfg = engine.controller_config();
        assert!(cfg.profiles.is_empty());
        assert!(cfg.active_profile.is_none());
    }
}
