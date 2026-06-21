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

use crate::config::{
    AppConfig, ControllerConfig, ControllerProfile, HidapiWorkaround, SystemControllerProfile,
};
use crate::console_pads::{self, ConsolePad};
use crate::controller_write;
use crate::error::{EngineError, Result};
use crate::{new_id, Engine};
use serde::Serialize;

/// A dry-run of materializing a system profile: which inputs would be written and
/// which the chosen emulator can't map blind (the user is pointed at the core's
/// own remap UI for those). Lets the UI show the outcome before any file changes.
#[derive(Debug, Clone, Serialize)]
pub struct MaterializePreview {
    /// `console_input_id` -> resolved native config key, for inputs we can write.
    pub encoded: std::collections::BTreeMap<String, String>,
    /// Console input ids with no stable mapping for this emulator.
    pub unencoded: Vec<String>,
}

/// Result of writing a system profile into an emulator's config.
#[derive(Debug, Clone, Serialize)]
pub struct ApplyOutcome {
    /// Absolute path of the config file that was edited.
    pub config_path: String,
    /// Absolute path of the backup taken before editing.
    pub backup_path: String,
    /// Number of bindings written.
    pub written: usize,
    /// Console input ids the emulator couldn't map blind (left for its remap UI).
    pub unencoded: Vec<String>,
    /// Display name of the emulator whose config was written (for UI messaging).
    pub emulator: String,
}

/// Live state of the SDL-HIDAPI vs xpadneo workaround, for the Controller UI.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct HidapiStatus {
    /// The user's configured policy.
    pub policy: HidapiWorkaround,
    /// Whether an xpadneo-bound controller is connected right now.
    pub xpadneo_present: bool,
    /// Whether the workaround would actually be applied to an SDL-input emulator
    /// launched at this moment, given the policy and detection.
    pub effective: bool,
}

impl Engine {
    pub fn controller_config(&self) -> ControllerConfig {
        self.load_config().controller
    }

    /// Set the policy for the SDL-HIDAPI vs xpadneo launch workaround.
    pub fn set_hidapi_workaround(&self, policy: HidapiWorkaround) -> Result<()> {
        let mut cfg: AppConfig = self.load_config();
        cfg.controller.sdl_hidapi_workaround = policy;
        self.save_config(&cfg)
    }

    /// Report the workaround's current policy and whether it would fire now, so
    /// the UI can show "Auto — active" vs "Auto — idle" honestly.
    pub fn hidapi_status(&self) -> HidapiStatus {
        let policy = self.load_config().controller.sdl_hidapi_workaround;
        let xpadneo_present = crate::adapters::xpadneo_controller_present();
        let effective = match policy {
            HidapiWorkaround::Off => false,
            HidapiWorkaround::Force => true,
            HidapiWorkaround::Auto => xpadneo_present,
        };
        HidapiStatus { policy, xpadneo_present, effective }
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

    // --- Per-system controller profiles (input-setup module) ----------------

    /// The target input layout for a system, or `None` if not yet catalogued.
    pub fn console_pad(&self, system: &str) -> Option<&'static ConsolePad> {
        console_pads::pad_for(system)
    }

    pub fn system_controller_profiles(&self) -> Vec<SystemControllerProfile> {
        self.load_config().controller.system_profiles
    }

    /// Create or update a per-system mapping profile. A blank `id` mints a new
    /// one. Validates that `system` is catalogued and that every binding key is a
    /// real input on that console — a profile can only ever hold writable keys.
    pub fn save_system_controller_profile(
        &self,
        mut profile: SystemControllerProfile,
    ) -> Result<SystemControllerProfile> {
        if profile.name.trim().is_empty() {
            return Err(EngineError::Invalid("system controller profile needs a name".into()));
        }
        let pad = console_pads::pad_for(&profile.system)
            .ok_or_else(|| EngineError::Invalid(format!("unknown system {}", profile.system)))?;
        if let Some(bad) = profile.bindings.keys().find(|k| !pad.has_input(k)) {
            return Err(EngineError::Invalid(format!(
                "input {bad} is not valid for system {}",
                profile.system
            )));
        }
        let mut cfg: AppConfig = self.load_config();
        if profile.id.trim().is_empty() {
            profile.id = new_id();
        }
        match cfg.controller.system_profiles.iter_mut().find(|p| p.id == profile.id) {
            Some(existing) => *existing = profile.clone(),
            None => cfg.controller.system_profiles.push(profile.clone()),
        }
        self.save_config(&cfg)?;
        Ok(profile)
    }

    pub fn delete_system_controller_profile(&self, id: &str) -> Result<()> {
        let mut cfg: AppConfig = self.load_config();
        cfg.controller.system_profiles.retain(|p| p.id != id);
        cfg.controller.system_assignments.retain(|_, pid| pid != id);
        self.save_config(&cfg)
    }

    /// Assign a profile as the active mapping for its system (or clear the
    /// system's assignment with `None`). Rejects unknown profile ids.
    pub fn assign_system_controller_profile(
        &self,
        system: &str,
        profile_id: Option<&str>,
    ) -> Result<()> {
        let mut cfg: AppConfig = self.load_config();
        match profile_id {
            Some(id) => {
                let prof = cfg
                    .controller
                    .system_profiles
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| EngineError::NotFound(format!("system controller profile {id}")))?;
                if prof.system != system {
                    return Err(EngineError::Invalid(format!(
                        "profile {id} targets {}, not {system}",
                        prof.system
                    )));
                }
                cfg.controller.system_assignments.insert(system.to_string(), id.to_string());
            }
            None => {
                cfg.controller.system_assignments.remove(system);
            }
        }
        self.save_config(&cfg)
    }

    /// Dry-run materializing a profile for RetroArch: report which inputs would be
    /// written and which RetroArch can't map blind. Read-only — touches no files.
    pub fn preview_system_controller_profile(&self, id: &str) -> Result<MaterializePreview> {
        let cfg = self.load_config();
        let profile = cfg
            .controller
            .system_profiles
            .iter()
            .find(|p| p.id == id)
            .ok_or_else(|| EngineError::NotFound(format!("system controller profile {id}")))?;
        let (_assignments, unencoded) =
            controller_write::materialize_retroarch(1, &profile.bindings);
        // console_input_id -> resolved config key, for the UI's preview.
        let mut encoded = std::collections::BTreeMap::new();
        for (input_id, desc) in &profile.bindings {
            if let Some(w) = controller_write::parse_w3c(desc) {
                if let Some(a) = controller_write::encode_retroarch(1, input_id, w) {
                    encoded.insert(input_id.clone(), a.key);
                }
            }
        }
        Ok(MaterializePreview { encoded, unencoded })
    }

    /// Materialize the profile assigned to `system` into its emulator's config,
    /// surgically (one line per binding, everything else byte-preserved). Takes a
    /// timestamped `.bak` first and writes atomically (temp file + rename).
    ///
    /// Refuses while any emulator is running — the emulators rewrite their config
    /// on exit, which would clobber our edit. Three adapters are wired up:
    /// **RetroArch** (Tier A — normalized indices, no device probe),
    /// **Mupen64Plus** (Tier B — raw SDL-joystick indices resolved from the
    /// `pad_mapping` the caller probed from live SDL), and **Mednafen** (Tier B,
    /// GUID-embedded — `pad_guid`/config GUID plus a config-derived d-pad axis
    /// convention). Any other adapter for the system yields an error.
    pub async fn apply_system_controller_profile(
        &self,
        system: &str,
        pad_mapping: Option<&controller_write::PadMapping>,
        pad_guid: Option<&str>,
        pad_name: Option<&str>,
    ) -> Result<ApplyOutcome> {
        let cfg = self.load_config();
        let profile_id = cfg
            .controller
            .system_assignments
            .get(system)
            .ok_or_else(|| EngineError::Invalid(format!("no controller profile assigned to {system}")))?;
        let profile = cfg
            .controller
            .system_profiles
            .iter()
            .find(|p| &p.id == profile_id)
            .ok_or_else(|| EngineError::NotFound(format!("system controller profile {profile_id}")))?
            .clone();

        let emulator = self
            .emulator_for_platform(system)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("no installed emulator for {system}")))?;

        // The emulators persist config on exit — never edit while one runs.
        let open_sessions: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM play_sessions WHERE ended_at IS NULL")
                .fetch_one(self.pool())
                .await?;
        if open_sessions > 0 {
            return Err(EngineError::Invalid(
                "an emulator is running; close it before writing controller config".into(),
            ));
        }

        match emulator.adapter_id.as_str() {
            "retroarch" => {
                let path = retroarch_config_path(&emulator).ok_or_else(|| {
                    EngineError::Invalid("cannot resolve HOME for retroarch.cfg".into())
                })?;
                let original = std::fs::read_to_string(&path).map_err(|e| {
                    EngineError::Invalid(format!(
                        "retroarch.cfg not readable at {}: {e}",
                        path.display()
                    ))
                })?;
                let (assignments, unencoded) =
                    controller_write::materialize_retroarch(1, &profile.bindings);
                let mut text = original;
                for a in &assignments {
                    text = controller_write::set_flat_key(&text, &a.key, &a.value);
                }
                let backup = backup_then_write(&path, &text, "retroarch.cfg")?;
                Ok(ApplyOutcome {
                    config_path: path.display().to_string(),
                    backup_path: backup,
                    written: assignments.len(),
                    unencoded,
                    emulator: emulator.name.clone(),
                })
            }
            "mupen64plus" => {
                // Tier B: the raw joystick indices Mupen reads only agree with the
                // pad's *live SDL* view, which the caller probes and threads in.
                let mapping = pad_mapping.ok_or_else(|| {
                    EngineError::Invalid(
                        "no controller detected; connect the pad you want to map before writing"
                            .into(),
                    )
                })?;
                let path = mupen_config_path().ok_or_else(|| {
                    EngineError::Invalid("cannot resolve config dir for mupen64plus.cfg".into())
                })?;
                let original = std::fs::read_to_string(&path).map_err(|e| {
                    EngineError::Invalid(format!(
                        "mupen64plus.cfg not readable at {} (launch the emulator once to \
                         generate it): {e}",
                        path.display()
                    ))
                })?;
                let (assignments, unencoded) =
                    controller_write::materialize_mupen(&profile.bindings, mapping);
                let mut text = original;
                // Force fully-manual mode or Mupen auto-detects and ignores our keys.
                text = controller_write::set_ini_key_in_section(
                    &text,
                    MUPEN_SECTION,
                    "mode",
                    "0",
                );
                for a in &assignments {
                    text = controller_write::set_ini_key_in_section(
                        &text,
                        MUPEN_SECTION,
                        &a.key,
                        &a.value,
                    );
                }
                let backup = backup_then_write(&path, &text, "mupen64plus.cfg")?;
                Ok(ApplyOutcome {
                    config_path: path.display().to_string(),
                    backup_path: backup,
                    written: assignments.len(),
                    unencoded,
                    emulator: emulator.name.clone(),
                })
            }
            "mednafen" => {
                // Tier B with a twist: Mednafen embeds the device GUID in every
                // binding and flattens the d-pad to virtual axes. We read both the
                // GUID and the d-pad axis convention from Mednafen's own config
                // (ground truth) rather than synthesizing them — see the
                // controller_write module note on backend divergence.
                let mapping = pad_mapping.ok_or_else(|| {
                    EngineError::Invalid(
                        "no controller detected; connect the pad you want to map before writing"
                            .into(),
                    )
                })?;
                let path = mednafen_config_path().ok_or_else(|| {
                    EngineError::Invalid("cannot resolve HOME for mednafen.cfg".into())
                })?;
                let original = std::fs::read_to_string(&path).map_err(|e| {
                    EngineError::Invalid(format!(
                        "mednafen.cfg not readable at {} (launch the emulator once to \
                         generate it): {e}",
                        path.display()
                    ))
                })?;
                // Mednafen keys bindings by its OWN evdev-derived calculated ID
                // (NOT the SDL GUID), so ask Mednafen directly — authoritative and
                // backend-independent. Fall back to the ID recorded in the existing
                // config, then the live SDL GUID, only if the probe is unavailable.
                let guid = probe_mednafen_joystick_id(&emulator.executable_path)
                    .or_else(|| controller_write::mednafen_config_guid(&original, system))
                    .or_else(|| pad_guid.map(str::to_string))
                    .ok_or_else(|| {
                        EngineError::Invalid(format!(
                            "could not determine Mednafen's device ID for {system}: ensure the \
                             controller is connected (or bind one input in Mednafen's input \
                             config once), then retry"
                        ))
                    })?;
                let dpad_axes = controller_write::mednafen_config_dpad_axes(&original, system);
                let (assignments, unencoded) = controller_write::materialize_mednafen(
                    system,
                    &profile.bindings,
                    mapping,
                    &guid,
                    dpad_axes,
                );
                let mut text = original;
                for a in &assignments {
                    text = controller_write::set_mednafen_setting(&text, &a.key, &a.value);
                }
                let backup = backup_then_write(&path, &text, "mednafen.cfg")?;
                Ok(ApplyOutcome {
                    config_path: path.display().to_string(),
                    backup_path: backup,
                    written: assignments.len(),
                    unencoded,
                    emulator: emulator.name.clone(),
                })
            }
            "pcsx2" => {
                let path = pcsx2_config_path(&emulator).ok_or_else(|| {
                    EngineError::Invalid("cannot resolve config dir for PCSX2.ini".into())
                })?;
                apply_ps_named_profile(
                    &path,
                    "Pad1",
                    controller_write::PsDialect::Pcsx2,
                    &profile.bindings,
                    &emulator.name,
                    "PCSX2.ini",
                )
            }
            "duckstation" => {
                let path = duckstation_config_path(&emulator).ok_or_else(|| {
                    EngineError::Invalid("cannot resolve config dir for DuckStation settings.ini".into())
                })?;
                apply_ps_named_profile(
                    &path,
                    "Pad1",
                    controller_write::PsDialect::DuckStation,
                    &profile.bindings,
                    &emulator.name,
                    "settings.ini",
                )
            }
            "snes9x" => {
                // Tier B: snes9x reads raw SDL-joystick indices (resolved from the
                // live probe). Its d-pad is a flattened-hat special case — see the
                // controller_write module note — so a hat-probed direction is left
                // for snes9x's own input detect rather than synthesized.
                let mapping = pad_mapping.ok_or_else(|| {
                    EngineError::Invalid(
                        "no controller detected; connect the pad you want to map before writing"
                            .into(),
                    )
                })?;
                let path = snes9x_config_path(&emulator).ok_or_else(|| {
                    EngineError::Invalid("cannot resolve config dir for snes9x.conf".into())
                })?;
                let original = std::fs::read_to_string(&path).map_err(|e| {
                    EngineError::Invalid(format!(
                        "snes9x.conf not readable at {} (launch the emulator once to \
                         generate it): {e}",
                        path.display()
                    ))
                })?;
                let joy = controller_write::snes9x_joystick_number(&original, SNES9X_SECTION)
                    .unwrap_or(1);
                let (assignments, mut unencoded) =
                    controller_write::materialize_snes9x(&profile.bindings, mapping, joy);
                // Don't nag about a d-pad already bound in the config: snes9x can't
                // address the SDL hat (it flattens hats to virtual axes), but the
                // existing lines work and are left untouched.
                unencoded.retain(|id| {
                    !(id.starts_with("dpad_")
                        && controller_write::snes9x_has_binding(&original, SNES9X_SECTION, id))
                });
                let mut text = original;
                for a in &assignments {
                    text = controller_write::set_ini_key_in_section(
                        &text,
                        SNES9X_SECTION,
                        &a.key,
                        &a.value,
                    );
                }
                let backup = backup_then_write(&path, &text, "snes9x.conf")?;
                Ok(ApplyOutcome {
                    config_path: path.display().to_string(),
                    backup_path: backup,
                    written: assignments.len(),
                    unencoded,
                    emulator: emulator.name.clone(),
                })
            }
            "mgba" => {
                // Tier B: mGBA reads raw SDL-joystick indices into a
                // per-platform `[<platform>.input.SDLB]` section, written compactly
                // (no spaces around `=`) to match mGBA's own writer.
                let mapping = pad_mapping.ok_or_else(|| {
                    EngineError::Invalid(
                        "no controller detected; connect the pad you want to map before writing"
                            .into(),
                    )
                })?;
                let section = controller_write::mgba_section(system).ok_or_else(|| {
                    EngineError::Invalid(format!("mGBA has no input section for {system}"))
                })?;
                let path = mgba_config_path(&emulator).ok_or_else(|| {
                    EngineError::Invalid("cannot resolve config dir for mGBA config.ini".into())
                })?;
                let original = std::fs::read_to_string(&path).map_err(|e| {
                    EngineError::Invalid(format!(
                        "mGBA config.ini not readable at {} (launch the emulator once to \
                         generate it): {e}",
                        path.display()
                    ))
                })?;
                let (assignments, unencoded) =
                    controller_write::materialize_mgba(&profile.bindings, mapping);
                let mut text = original;
                for a in &assignments {
                    text = controller_write::set_ini_key_in_section_compact(
                        &text, &section, &a.key, &a.value,
                    );
                }
                // CRITICAL: mGBA's Qt frontend (mgba-qt, what Arcadia launches)
                // loads the generic SDLB section *first* and then OVERRIDES it with
                // a per-device `input-profile.<JoystickName>` section it loads last
                // (and re-saves on exit). If such a profile exists, an SDLB-only
                // write is silently overridden — the bug the user hit. Mirror the
                // same bindings into the live pad's name-keyed profile so they win.
                if let Some(name) = pad_name {
                    if let Some(profile_section) =
                        controller_write::mgba_profile_section(system, name)
                    {
                        for a in &assignments {
                            text = controller_write::set_ini_key_in_section_compact(
                                &text,
                                &profile_section,
                                &a.key,
                                &a.value,
                            );
                        }
                    }
                }
                let backup = backup_then_write(&path, &text, "config.ini")?;
                Ok(ApplyOutcome {
                    config_path: path.display().to_string(),
                    backup_path: backup,
                    written: assignments.len(),
                    unencoded,
                    emulator: emulator.name.clone(),
                })
            }
            other => Err(EngineError::Invalid(format!(
                "controller write isn't wired up for {other}; {system} launches with it"
            ))),
        }
    }
}

/// Resolve `mednafen.cfg` under `~/.mednafen` — a plain home dotdir, not XDG
/// config (matching the adapter's `HotkeyBase::Home` + `.mednafen/mednafen.cfg`).
fn mednafen_config_path() -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    Some(home.join(".mednafen").join("mednafen.cfg"))
}

/// Ask Mednafen for its own calculated joystick ID — the value it keys config
/// bindings by (evdev-derived, NOT the SDL GUID; see `parse_mednafen_joystick_id`).
///
/// Mednafen enumerates joysticks (printing `ID: 0x…`) *before* loading the ROM,
/// so we launch it with a bogus ROM path: it prints the ID, fails to open the
/// ROM, and exits on its own — no window, no settings written. `SDL_VIDEODRIVER`/
/// `SDL_AUDIODRIVER=dummy` guarantee silence even on a slow exit, and
/// `MEDNAFEN_ALLOWMULTI=1` bypasses the single-instance lockfile. The ID is
/// backend-independent, so we needn't mirror the launch SDL backend here. A
/// reader thread + timeout keep a hung child from blocking the write.
fn probe_mednafen_joystick_id(executable: &str) -> Option<String> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    let mut child = Command::new(executable)
        .arg("/nonexistent/arcadia-mednafen-probe")
        .env("SDL_VIDEODRIVER", "dummy")
        .env("SDL_AUDIODRIVER", "dummy")
        .env("MEDNAFEN_ALLOWMULTI", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let mut stdout = child.stdout.take()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buf = String::new();
        let _ = stdout.read_to_string(&mut buf);
        let _ = tx.send(buf);
    });
    let text = rx.recv_timeout(Duration::from_secs(6)).ok();
    let _ = child.kill();
    let _ = child.wait();

    controller_write::parse_mednafen_joystick_id(text.as_deref().unwrap_or(""))
}

/// The `mupen64plus.cfg` player-1 input section.
const MUPEN_SECTION: &str = "Input-SDL-Control1";

/// The `snes9x.conf` player-1 joypad section.
const SNES9X_SECTION: &str = "Joypad 0";

/// Back up `path` to a timestamped `.bak.<stamp>` (mirroring the Dolphin pattern),
/// then write `text` atomically (temp file + rename in the same dir, so the swap is
/// atomic on the same filesystem). Returns the backup path for the apply outcome.
fn backup_then_write(
    path: &std::path::Path,
    text: &str,
    fallback_name: &str,
) -> Result<String> {
    let stamp = chrono::Utc::now().format("%Y%m%d%H%M%S");
    let backup = path.with_file_name(format!(
        "{}.bak.{stamp}",
        path.file_name().and_then(|n| n.to_str()).unwrap_or(fallback_name)
    ));
    std::fs::copy(path, &backup)?;
    let tmp = path.with_extension("arcadia-tmp");
    std::fs::write(&tmp, text.as_bytes())?;
    std::fs::rename(&tmp, path)?;
    Ok(backup.display().to_string())
}

/// Shared writer for the SDL named-token PlayStation standalones (PCSX2,
/// DuckStation). Reads the `SDL-N` device index the emulator already uses (else
/// `SDL-0`), encodes the profile into the `[section]` block, and writes it back
/// with a timestamped backup. These bind by SDL standard-gamepad token names, so
/// no controller probe is needed — the mapping is device-independent.
fn apply_ps_named_profile(
    path: &std::path::Path,
    section: &str,
    dialect: controller_write::PsDialect,
    bindings: &std::collections::BTreeMap<String, String>,
    emulator_name: &str,
    fallback_name: &str,
) -> Result<ApplyOutcome> {
    let original = std::fs::read_to_string(path).map_err(|e| {
        EngineError::Invalid(format!(
            "{} not readable at {} (launch the emulator once to generate it): {e}",
            fallback_name,
            path.display()
        ))
    })?;
    let prefix =
        controller_write::ps_device_prefix(&original, section).unwrap_or_else(|| "SDL-0".to_string());
    let (assignments, unencoded) =
        controller_write::materialize_ps_named(bindings, dialect, &prefix);
    let mut text = original;
    for a in &assignments {
        text = controller_write::set_ini_key_in_section(&text, section, &a.key, &a.value);
    }
    let backup = backup_then_write(path, &text, fallback_name)?;
    Ok(ApplyOutcome {
        config_path: path.display().to_string(),
        backup_path: backup,
        written: assignments.len(),
        unencoded,
        emulator: emulator_name.to_string(),
    })
}

/// Resolve the `retroarch.cfg` Arcadia should write — the same file the hotkey
/// reader scans, native or Flatpak.
fn retroarch_config_path(emulator: &crate::models::Emulator) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    let is_flatpak = emulator.install_source.eq_ignore_ascii_case("flatpak")
        || emulator.executable_path.contains("flatpak run");
    Some(if is_flatpak {
        home.join(".var/app/org.libretro.RetroArch/config/retroarch/retroarch.cfg")
    } else {
        home.join(".config/retroarch/retroarch.cfg")
    })
}

/// Resolve `mupen64plus.cfg` under the XDG config dir (matching the adapter's
/// `HotkeyBase::XdgConfig` + `mupen64plus/mupen64plus.cfg`). Mupen64Plus is
/// native-only (no Flatpak app id), so there is no Flatpak branch.
fn mupen_config_path() -> Option<std::path::PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".config")))?;
    Some(base.join("mupen64plus").join("mupen64plus.cfg"))
}

/// Resolve PCSX2's `PCSX2.ini` (native `~/.config/PCSX2/inis/` or the Flatpak
/// per-app config tree).
fn pcsx2_config_path(emulator: &crate::models::Emulator) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    Some(if is_flatpak(emulator) {
        home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/inis/PCSX2.ini")
    } else {
        home.join(".config/PCSX2/inis/PCSX2.ini")
    })
}

/// Resolve DuckStation's `settings.ini` (native `~/.config/duckstation/` or the
/// Flatpak per-app config tree).
fn duckstation_config_path(emulator: &crate::models::Emulator) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    Some(if is_flatpak(emulator) {
        home.join(".var/app/org.duckstation.DuckStation/config/duckstation/settings.ini")
    } else {
        home.join(".config/duckstation/settings.ini")
    })
}

/// Resolve snes9x's `snes9x.conf` (native `~/.config/snes9x/` or the Flatpak
/// per-app config tree).
fn snes9x_config_path(emulator: &crate::models::Emulator) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    Some(if is_flatpak(emulator) {
        home.join(".var/app/com.snes9x.Snes9x/config/snes9x/snes9x.conf")
    } else {
        home.join(".config/snes9x/snes9x.conf")
    })
}

/// Resolve mGBA's `config.ini` (native `~/.config/mgba/` or the Flatpak per-app
/// config tree).
fn mgba_config_path(emulator: &crate::models::Emulator) -> Option<std::path::PathBuf> {
    let home = std::env::var_os("HOME").map(std::path::PathBuf::from)?;
    Some(if is_flatpak(emulator) {
        home.join(".var/app/io.mgba.mGBA/config/mgba/config.ini")
    } else {
        home.join(".config/mgba/config.ini")
    })
}

fn is_flatpak(emulator: &crate::models::Emulator) -> bool {
    emulator.install_source.eq_ignore_ascii_case("flatpak")
        || emulator.executable_path.contains("flatpak run")
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

    #[tokio::test]
    async fn system_profile_crud_validation_and_assignment() {
        let (engine, _tmp) = engine_with_tempdirs().await;

        // Unknown system is rejected.
        let bad = SystemControllerProfile {
            id: String::new(),
            name: "x".into(),
            system: "nonexistent".into(),
            bindings: Default::default(),
        };
        assert!(engine.save_system_controller_profile(bad).is_err());

        // Invalid binding key for the system is rejected.
        let mut bad_key = SystemControllerProfile {
            id: String::new(),
            name: "x".into(),
            system: "nes".into(),
            bindings: Default::default(),
        };
        bad_key.bindings.insert("c_up".into(), "btn:0".into()); // c_up isn't on NES
        assert!(engine.save_system_controller_profile(bad_key).is_err());

        // Valid profile saves and roundtrips.
        let mut prof = SystemControllerProfile {
            id: String::new(),
            name: "N64 pad".into(),
            system: "n64".into(),
            bindings: Default::default(),
        };
        prof.bindings.insert("a".into(), "btn:0".into());
        prof.bindings.insert("c_up".into(), "btn:3".into());
        let saved = engine.save_system_controller_profile(prof).unwrap();
        assert!(!saved.id.is_empty());
        assert_eq!(engine.system_controller_profiles().len(), 1);

        // Assignment to the matching system works; mismatched system is rejected.
        engine.assign_system_controller_profile("n64", Some(&saved.id)).unwrap();
        assert!(engine.assign_system_controller_profile("nes", Some(&saved.id)).is_err());

        // Preview separates encodable (a) from unencoded (c_up has no stable map).
        let preview = engine.preview_system_controller_profile(&saved.id).unwrap();
        assert!(preview.encoded.contains_key("a"));
        assert!(preview.unencoded.contains(&"c_up".to_string()));

        // Delete clears both the profile and its assignment.
        engine.delete_system_controller_profile(&saved.id).unwrap();
        let cfg = engine.controller_config();
        assert!(cfg.system_profiles.is_empty());
        assert!(cfg.system_assignments.is_empty());
    }
}
