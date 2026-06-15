//! Emulator adapter layer — the heart of the system.
//!
//! Per the build plan's correction: built-in / first-party adapters are
//! **native Rust** (this module). They are the only path that spawns
//! emulator processes and reads arbitrary save/ROM directories. WASM is
//! reserved for pure-data community plugins and is not in the MVP.
//!
//! Adapters are async, fallible, and capability-reporting. Capabilities are
//! *reported*, not assumed: an adapter that can't read achievements omits the
//! flag in its descriptor and the UI hides the feature.

mod retroarch;
mod standalone;

pub mod detect;

use crate::error::AdapterError;
use crate::models::InstallSource;
use crate::save_states::SaveState;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

pub use retroarch::RetroArchAdapter;
pub use standalone::StandaloneAdapter;

/// Static, declared capabilities of an adapter.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct Capabilities {
    pub savestates: bool,
    pub saves: bool,
    pub achievements: bool,
    pub screenshots: bool,
}

/// Static descriptor: id, supported platforms, capability flags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterDescriptor {
    pub id: String,
    pub display_name: String,
    pub platforms: Vec<String>,
    pub capabilities: Capabilities,
}

/// A concrete install of an emulator found on this machine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorInstall {
    pub adapter_id: String,
    pub name: String,
    pub version: Option<String>,
    pub install_source: InstallSource,
    /// Executable path, or the in-sandbox command for Flatpak installs.
    pub executable_path: String,
    /// Set when the install is a Flatpak; used to build `flatpak run` lines.
    pub flatpak_app_id: Option<String>,
}

/// A discovered savestate the user asked to boot straight into. Carries both the
/// slot number and the file path because emulators key the load differently:
/// RetroArch loads by slot number (`--entryslot N`), Mupen64Plus by file path
/// (`--savestate <file>`). Each adapter reads whichever it needs and ignores the
/// other.
#[derive(Debug, Clone)]
pub struct LoadState {
    pub slot: i64,
    pub path: String,
}

/// Everything an adapter needs to spawn a game.
#[derive(Debug, Clone)]
pub struct LaunchContext {
    pub executable_path: String,
    pub install_source: InstallSource,
    pub flatpak_app_id: Option<String>,
    pub rom_path: String,
    pub platform: String,
    /// Per-game extra CLI args, inserted before the adapter's own game args.
    pub extra_args: Vec<String>,
    /// Per-game environment variables applied to the spawned process.
    pub env: Vec<(String, String)>,
    /// Per-game libretro core override (base name, no `_libretro.so`). Only the
    /// RetroArch adapter honours it; other adapters ignore it.
    pub core_override: Option<String>,
    /// When set, boot directly into this discovered savestate instead of a cold
    /// start. Adapters that can't load a state at startup ignore it.
    pub load_state: Option<LoadState>,
    /// Resolved policy for the SDL-HIDAPI vs xpadneo workaround. Only the
    /// standalone adapter's SDL-input emulators consult it; everything else
    /// ignores it.
    pub sdl_hidapi_workaround: crate::config::HidapiWorkaround,
}

/// Handle to a spawned emulator. The session tracker owns the child and awaits
/// its exit to record playtime.
pub struct LaunchHandle {
    pub pid: u32,
    pub child: tokio::process::Child,
}

/// Context for filesystem scans (saves, savestates, screenshots, ...).
#[derive(Debug, Clone)]
pub struct ScanContext {
    pub rom_path: String,
    pub platform: String,
    /// Flatpak application id when the resolved emulator is a Flatpak install, so
    /// the adapter can redirect its save/state dirs to `~/.var/app/<id>/…`.
    pub flatpak_app_id: Option<String>,
    /// The game's disc serial or game id, when the engine could resolve one from
    /// the ROM (PS2 `SLUS-20946`, GameCube/Wii 6-char id, …). Adapters that key
    /// savestates by serial/id (PCSX2, DuckStation, Dolphin) need it; stem-keyed
    /// adapters (RetroArch, Snes9x) ignore it. `None` when unresolved.
    pub disc_key: Option<String>,
}

#[async_trait]
pub trait EmulatorAdapter: Send + Sync {
    /// Static descriptor: id, supported platforms, capability flags.
    fn descriptor(&self) -> AdapterDescriptor;

    /// Locate installs on this machine (paths, version, install source).
    async fn detect(&self) -> Result<Vec<EmulatorInstall>, AdapterError>;

    /// Spawn the emulator for a game; returns a handle for session tracking.
    async fn launch(&self, ctx: &LaunchContext) -> Result<LaunchHandle, AdapterError>;

    /// Whether this emulator can boot a `.zip` directly. Most can't, so the
    /// launcher transparently extracts the inner ROM first; the few that read
    /// archives natively (libretro frontends, MAME romsets) override this to
    /// `true` and are handed the `.zip` untouched.
    fn handles_archives(&self) -> bool {
        false
    }

    /// Whether this adapter can boot straight into a discovered savestate via a
    /// startup CLI flag (RetroArch `--entryslot`, Mupen64Plus/PCSX2/DuckStation
    /// `--savestate`/`-statefile`, Dolphin `-s`). The Recall State grid makes a
    /// slot card clickable only when this is `true`; otherwise it renders the
    /// discovered states read-only, because a click would silently cold-boot and
    /// ignore the state (the lie we avoid for Snes9x). Defaults to `false`.
    fn supports_launch_state(&self) -> bool {
        false
    }

    /// File-level scans. Default to empty: adapters opt in via capabilities.
    async fn scan_saves(&self, _ctx: &ScanContext) -> Result<Vec<String>, AdapterError> {
        Ok(Vec::new())
    }
    async fn scan_screenshots(&self, _ctx: &ScanContext) -> Result<Vec<String>, AdapterError> {
        Ok(Vec::new())
    }

    /// Discover the game's savestates as structured slots, reading (never
    /// rewriting) the emulator's own state files. Defaults to empty: an adapter
    /// opts in by knowing where its emulator stores states and how they're named.
    async fn scan_save_states(&self, _ctx: &ScanContext) -> Result<Vec<SaveState>, AdapterError> {
        Ok(Vec::new())
    }

    /// Learn the key this emulator names its per-game files by, by observing
    /// which of its own files were written during a just-ended play session
    /// (`since` is the session start). For most emulators the key is derivable
    /// from the ROM ahead of time, so they return `None`; the opt-in is for
    /// emulators like Mupen64Plus that name files from an internal database
    /// lookup we can't reproduce — there we read back the name the emulator
    /// actually chose. Returns the learned key to persist, or `None` when the
    /// emulator wrote nothing this session (or doesn't key by a learnable name).
    async fn learn_state_key(
        &self,
        _ctx: &ScanContext,
        _since: std::time::SystemTime,
    ) -> Option<String> {
        None
    }
}

/// The set of built-in adapters, keyed by id.
pub struct AdapterRegistry {
    adapters: HashMap<String, Arc<dyn EmulatorAdapter>>,
}

impl AdapterRegistry {
    /// Build the adapter set: one libretro frontend plus the standalone
    /// emulators, spanning 2D retro through current-gen emulation.
    pub fn builtin() -> Self {
        let list: Vec<Arc<dyn EmulatorAdapter>> = vec![
            Arc::new(RetroArchAdapter::new()),
            Arc::new(StandaloneAdapter::snes9x()),
            Arc::new(StandaloneAdapter::mgba()),
            Arc::new(StandaloneAdapter::mupen64plus()),
            Arc::new(StandaloneAdapter::dolphin()),
            Arc::new(StandaloneAdapter::pcsx2()),
            Arc::new(StandaloneAdapter::ppsspp()),
            Arc::new(StandaloneAdapter::rpcs3()),
            Arc::new(StandaloneAdapter::duckstation()),
            Arc::new(StandaloneAdapter::melonds()),
            Arc::new(StandaloneAdapter::flycast()),
            Arc::new(StandaloneAdapter::mame()),
            Arc::new(StandaloneAdapter::xemu()),
            Arc::new(StandaloneAdapter::cemu()),
            Arc::new(StandaloneAdapter::vita3k()),
            Arc::new(StandaloneAdapter::ryujinx()),
            Arc::new(StandaloneAdapter::lime3ds()),
            // Launch-only single-system cores (savestate discovery not yet wired).
            Arc::new(StandaloneAdapter::mednafen()),
            Arc::new(StandaloneAdapter::mesen()),
            Arc::new(StandaloneAdapter::stella()),
            Arc::new(StandaloneAdapter::blastem()),
            Arc::new(StandaloneAdapter::openmsx()),
            Arc::new(StandaloneAdapter::gearcoleco()),
            Arc::new(StandaloneAdapter::bigpemu()),
            Arc::new(StandaloneAdapter::vice()),
            Arc::new(StandaloneAdapter::atari800()),
        ];
        let mut adapters = HashMap::new();
        for a in list {
            adapters.insert(a.descriptor().id, a);
        }
        Self { adapters }
    }

    pub fn get(&self, id: &str) -> Option<Arc<dyn EmulatorAdapter>> {
        self.adapters.get(id).cloned()
    }

    pub fn all(&self) -> impl Iterator<Item = &Arc<dyn EmulatorAdapter>> {
        self.adapters.values()
    }

    pub fn descriptors(&self) -> Vec<AdapterDescriptor> {
        self.adapters.values().map(|a| a.descriptor()).collect()
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::PLATFORMS;

    #[test]
    fn every_platform_is_launchable_and_hints_resolve() {
        let reg = AdapterRegistry::builtin();
        let descriptors = reg.descriptors();
        for p in PLATFORMS {
            // The preferred adapter must actually exist in the registry.
            assert!(
                reg.get(p.adapter_hint).is_some(),
                "{}: adapter_hint '{}' is not a registered adapter",
                p.id,
                p.adapter_hint
            );
            // And some adapter must advertise the platform, or nothing could
            // ever boot it (the hint adapter usually does, but verify directly).
            assert!(
                descriptors.iter().any(|d| d.platforms.iter().any(|x| x == p.id)),
                "{}: no adapter advertises this platform",
                p.id
            );
        }
    }
}
