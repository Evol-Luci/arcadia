//! RetroArch adapter (libretro path). Covers many 2D/retro platforms via cores.
//!
//! RetroArch only boots content unattended when it can pick a core. Handing it
//! bare content (`retroarch <rom>`) drops the user into the menu on a fresh
//! setup, because there is no content/core association yet. So we resolve a
//! libretro core for the game's platform and launch `retroarch -L <core>
//! <content>`. When no core is installed we fall back to content-only — no
//! regression, RetroArch still does whatever it would have done before.

use super::detect::{self};
use super::{
    AdapterDescriptor, Capabilities, EmulatorAdapter, EmulatorInstall, LaunchContext, LaunchHandle,
    ScanContext,
};
use crate::error::AdapterError;
use crate::save_states::{discover_save_states, SaveState};
use crate::saves::retroarch_save_dirs;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

pub struct RetroArchAdapter;

impl RetroArchAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl Default for RetroArchAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl EmulatorAdapter for RetroArchAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            id: "retroarch".to_string(),
            display_name: "RetroArch".to_string(),
            platforms: ADVERTISED_PLATFORMS.iter().map(|s| s.to_string()).collect(),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: true, // RetroAchievements
                screenshots: true,
            },
        }
    }

    async fn detect(&self) -> Result<Vec<EmulatorInstall>, AdapterError> {
        Ok(detect::detect_emulator(
            "retroarch",
            "RetroArch",
            &["retroarch"],
            Some("org.libretro.RetroArch"),
        )
        .await)
    }

    // libretro cores load zipped content natively; never pre-extract for it.
    fn handles_archives(&self) -> bool {
        true
    }

    // RetroArch boots into a numbered slot with `--entryslot N` (handled in
    // `launch`), so every game it runs can be recalled.
    fn supports_launch_state(&self) -> bool {
        true
    }

    async fn launch(&self, ctx: &LaunchContext) -> Result<LaunchHandle, AdapterError> {
        // A per-game core override wins over the platform default when its
        // `.so` is actually installed; a stale/missing override silently defers
        // to the normal platform resolution rather than failing the launch.
        let resolved = ctx
            .core_override
            .as_deref()
            .and_then(|base| resolve_named_core(base, ctx.flatpak_app_id.as_deref()))
            .or_else(|| resolve_core(&ctx.platform, ctx.flatpak_app_id.as_deref()));

        // Resolve a core for the platform and boot it directly. Without a core
        // we fall back to bare content (RetroArch's own association, if any).
        let mut args = match resolved {
            Some(core) => {
                tracing::info!(platform = %ctx.platform, core = %core, "retroarch: launching with core");
                vec!["-L".to_string(), core, ctx.rom_path.clone()]
            }
            None => {
                tracing::debug!(platform = %ctx.platform, "retroarch: no core found, launching content only");
                vec![ctx.rom_path.clone()]
            }
        };

        // Boot straight into a savestate slot when asked. RetroArch keys the load
        // by slot number (`--entryslot N`) against the content's own state files,
        // which is exactly how our discovery found them. The auto-save slot (-1)
        // has no stable entryslot number, so we skip it and let the user resume it
        // from the in-emulator menu.
        if let Some(load) = &ctx.load_state {
            if load.slot >= 0 {
                tracing::info!(slot = load.slot, "retroarch: launching into savestate slot");
                args.push("-e".to_string());
                args.push(load.slot.to_string());
            }
        }
        let mut cmd = detect::build_command(ctx, args)?;
        let child = cmd
            .spawn()
            .map_err(|e| AdapterError::Launch(format!("retroarch: {e}")))?;
        let pid = child.id().unwrap_or(0);
        Ok(LaunchHandle { pid, child })
    }

    async fn scan_screenshots(&self, _ctx: &ScanContext) -> Result<Vec<String>, AdapterError> {
        // RetroArch writes screenshots to a configurable dir; reading that
        // config is a v0.5 task. Report none for now (capability stays true so
        // the UI can show "screenshots: supported, none indexed yet").
        Ok(Vec::new())
    }

    /// RetroArch keys savestates by the ROM's basename in its `states/` dir and
    /// writes a sibling `<file>.png` thumbnail when configured — exactly what
    /// [`discover_save_states`] pairs up.
    async fn scan_save_states(&self, ctx: &ScanContext) -> Result<Vec<SaveState>, AdapterError> {
        let dirs = retroarch_save_dirs(ctx.flatpak_app_id.as_deref());
        let Some(states_dir) = dirs
            .iter()
            .find(|d| d.file_name().map(|n| n == "states").unwrap_or(false))
        else {
            return Ok(Vec::new());
        };
        let stem = Path::new(&ctx.rom_path)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if stem.is_empty() {
            return Ok(Vec::new());
        }
        Ok(discover_save_states(states_dir, stem))
    }
}

/// Platforms RetroArch advertises, i.e. every platform with at least one core in
/// [`core_candidates`]. The descriptor and the core map must agree, which the
/// `every_advertised_platform_has_core_candidates` test enforces.
const ADVERTISED_PLATFORMS: &[&str] = &[
    "nes", "snes", "n64", "gb", "gbc", "gba", "nds", "genesis", "ps1", "sms", "gamegear",
    "sega32x", "segacd", "saturn", "pcengine", "virtualboy", "atari2600", "atari5200",
    "atari7800", "lynx", "jaguar", "wonderswan", "ngp", "colecovision", "c64", "amiga", "msx",
    "dreamcast",
];

/// Preferred libretro core base names per platform, most-preferred first (no
/// `_libretro.so` suffix). These are the widely-packaged cores in the libretro
/// buildbot / distro `libretro-*` packages; the first one actually present on
/// disk wins.
fn core_candidates(platform: &str) -> &'static [&'static str] {
    match platform {
        "nes" => &["mesen", "nestopia", "fceumm", "quicknes"],
        "snes" => &["snes9x", "bsnes", "bsnes_mercury_balanced"],
        "n64" => &["mupen64plus_next", "parallel_n64"],
        "gb" | "gbc" => &["gambatte", "sameboy", "gearboy"],
        "gba" => &["mgba", "vbam", "vba_next"],
        "nds" => &["melonds", "desmume"],
        "genesis" => &["genesis_plus_gx", "picodrive"],
        "ps1" => &["swanstation", "pcsx_rearmed", "beetle_psx", "mednafen_psx"],
        "sms" => &["genesis_plus_gx", "picodrive", "smsplus"],
        "gamegear" => &["genesis_plus_gx", "gearsystem"],
        "sega32x" => &["picodrive"],
        "segacd" => &["genesis_plus_gx", "picodrive"],
        "saturn" => &["mednafen_saturn", "kronos", "yabasanshiro"],
        "pcengine" => &["mednafen_pce", "mednafen_pce_fast", "mednafen_supergrafx"],
        "virtualboy" => &["mednafen_vb", "beetle_vb"],
        "atari2600" => &["stella", "stella2014"],
        "atari5200" => &["a5200", "atari800"],
        "atari7800" => &["prosystem"],
        "lynx" => &["handy", "mednafen_lynx", "beetle_lynx"],
        "jaguar" => &["virtualjaguar"],
        "wonderswan" => &["mednafen_wswan", "beetle_wswan"],
        "ngp" => &["mednafen_ngp", "beetle_ngp"],
        "colecovision" => &["bluemsx", "gearcoleco"],
        "c64" => &["vice_x64", "vice_x64sc"],
        "amiga" => &["puae", "puae2021"],
        "msx" => &["bluemsx", "fmsx"],
        "dreamcast" => &["flycast"],
        _ => &[],
    }
}

/// Directories to search for native (non-Flatpak) libretro cores. `$PATH`-style
/// override first, then the common distro and per-user locations.
fn native_core_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(env) = std::env::var_os("LIBRETRO_DIRECTORY") {
        dirs.push(PathBuf::from(env));
    }
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join(".config/retroarch/cores"));
        dirs.push(home.join(".local/share/libretro/cores"));
    }
    dirs.push(PathBuf::from("/usr/lib/libretro"));
    dirs.push(PathBuf::from("/usr/lib64/libretro"));
    dirs.push(PathBuf::from("/usr/local/lib/libretro"));
    dirs
}

/// Resolve an absolute (or sandbox-relative) core path to pass to `-L`.
///
/// Native installs: search the known core directories for the first candidate
/// that exists on disk and return its absolute path.
///
/// Flatpak: the Flatpak's cores live on the host under
/// `~/.var/app/<id>/config/retroarch/cores/`, but `-L` is interpreted *inside*
/// the sandbox where that same directory is mounted at `~/.config/retroarch/
/// cores/`. So we probe the host path to see what's installed but return the
/// in-sandbox path, which is what RetroArch will actually resolve.
fn resolve_core(platform: &str, flatpak_app_id: Option<&str>) -> Option<String> {
    let candidates = core_candidates(platform);
    if candidates.is_empty() {
        return None;
    }

    if let Some(app_id) = flatpak_app_id {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let host_dir = home
            .join(".var/app")
            .join(app_id)
            .join("config/retroarch/cores");
        for base in candidates {
            let file = format!("{base}_libretro.so");
            if host_dir.join(&file).is_file() {
                // Inside the sandbox $HOME is the real home, but ~/.config is
                // redirected to the host's ~/.var/app/<id>/config — so this
                // string is exactly what RetroArch resolves in the sandbox.
                let in_sandbox = home.join(".config/retroarch/cores").join(&file);
                return Some(in_sandbox.to_string_lossy().to_string());
            }
        }
        return None;
    }

    first_core_in_dirs(candidates, &native_core_dirs())
}

/// Resolve a single named core (a per-game override) the same way as the
/// platform resolver, but for one explicit base name. Returns `None` when that
/// core's `.so` isn't installed, so the caller can fall back to the platform
/// default rather than booting into the menu.
fn resolve_named_core(base: &str, flatpak_app_id: Option<&str>) -> Option<String> {
    let candidates = [base];
    if let Some(app_id) = flatpak_app_id {
        let home = std::env::var_os("HOME").map(PathBuf::from)?;
        let host_dir = home
            .join(".var/app")
            .join(app_id)
            .join("config/retroarch/cores");
        let file = format!("{base}_libretro.so");
        if host_dir.join(&file).is_file() {
            let in_sandbox = home.join(".config/retroarch/cores").join(&file);
            return Some(in_sandbox.to_string_lossy().to_string());
        }
        return None;
    }
    first_core_in_dirs(&candidates, &native_core_dirs())
}

/// Pure resolver: first `<candidate>_libretro.so` that exists across `dirs`.
fn first_core_in_dirs(candidates: &[&str], dirs: &[PathBuf]) -> Option<String> {
    for base in candidates {
        let file = format!("{base}_libretro.so");
        for dir in dirs {
            let path = dir.join(&file);
            if path.is_file() {
                return Some(path.to_string_lossy().to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advertised_platform_has_core_candidates() {
        // The adapter advertises these platforms; each must map to at least one
        // core or launch would silently drop to menu mode for it.
        for p in ADVERTISED_PLATFORMS {
            assert!(!core_candidates(p).is_empty(), "no core mapped for {p}");
        }
        // And it must not advertise platforms it can't actually boot.
        assert!(core_candidates("ps3").is_empty());
        assert!(!ADVERTISED_PLATFORMS.contains(&"ps3"));
    }

    #[test]
    fn resolves_core_from_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        std::fs::write(dir.join("snes9x_libretro.so"), b"\x7fELF").unwrap();
        let dirs = vec![dir];

        let core = first_core_in_dirs(core_candidates("snes"), &dirs).expect("core resolved");
        assert!(core.ends_with("snes9x_libretro.so"));

        // A platform whose core isn't in the search dirs => None (content-only).
        assert!(first_core_in_dirs(core_candidates("n64"), &dirs).is_none());
    }

    #[test]
    fn picks_first_available_candidate_by_preference() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().to_path_buf();
        // Only the second-choice NES core is present; it should still resolve.
        std::fs::write(dir.join("fceumm_libretro.so"), b"\x7fELF").unwrap();
        let core = first_core_in_dirs(core_candidates("nes"), &[dir]).expect("core resolved");
        assert!(core.ends_with("fceumm_libretro.so"));
    }
}
