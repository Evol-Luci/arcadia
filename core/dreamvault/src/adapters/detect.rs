//! Shared detection helpers: locate executables, query install source, build
//! launch commands. These respect existing Linux tooling rather than replacing
//! it — we ask `pacman` / `flatpak` what they own and never assume.

use crate::adapters::{EmulatorInstall, LaunchContext};
use crate::error::AdapterError;
use crate::models::InstallSource;
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Find an executable by name on `$PATH`. Returns the first match.
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if is_executable(&candidate) {
            return Some(candidate);
        }
    }
    None
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    match std::fs::metadata(p) {
        Ok(m) => m.is_file() && (m.permissions().mode() & 0o111 != 0),
        Err(_) => false,
    }
}

/// The pacman package that owns `path`, if any. This is how we determine both
/// install source and version *without executing the emulator* — running an
/// unknown GUI binary with `--version` is unsafe (it can pop dialogs, launch
/// the app, or inhibit the screensaver). Returns the package name on success.
pub async fn pacman_owner(path: &Path) -> Option<String> {
    let out = Command::new("pacman").arg("-Qoq").arg(path).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Version of an installed pacman package, parsed from `pacman -Q <pkg>`
/// ("pkgname 1.2.3-1" → "1.2.3-1").
pub async fn pacman_version(pkg: &str) -> Option<String> {
    let out = Command::new("pacman").arg("-Q").arg(pkg).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .nth(1)
        .map(|s| s.to_string())
}

/// Is a given Flatpak application installed? (`flatpak info` reads metadata; it
/// does not run the app, so this is side-effect-free.)
pub async fn flatpak_installed(app_id: &str) -> bool {
    Command::new("flatpak")
        .args(["info", app_id])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Version of an installed Flatpak from `flatpak info` metadata, if it exposes
/// a `Version:` field. Also side-effect-free.
pub async fn flatpak_version(app_id: &str) -> Option<String> {
    let out = Command::new("flatpak").args(["info", app_id]).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines()
        .find_map(|l| l.trim().strip_prefix("Version:").map(|v| v.trim().to_string()))
        .filter(|s| !s.is_empty())
}

/// Detect an emulator across native (PATH) and Flatpak installs.
///
/// `bin_names` are candidate executable names tried in order; `flatpak_app_id`
/// is the Flatpak application id (if the emulator ships as a Flatpak).
pub async fn detect_emulator(
    adapter_id: &str,
    display_name: &str,
    bin_names: &[&str],
    flatpak_app_id: Option<&str>,
) -> Vec<EmulatorInstall> {
    let mut found = Vec::new();

    for name in bin_names {
        if let Some(path) = which(name) {
            // Determine source + version from the package manager, never by
            // executing the binary.
            let (source, version) = match pacman_owner(&path).await {
                Some(pkg) => (InstallSource::Pacman, pacman_version(&pkg).await),
                None => (InstallSource::Manual, None),
            };
            found.push(EmulatorInstall {
                adapter_id: adapter_id.to_string(),
                name: display_name.to_string(),
                version,
                install_source: source,
                executable_path: path.display().to_string(),
                flatpak_app_id: None,
            });
            break; // first native hit is enough
        }
    }

    if let Some(app_id) = flatpak_app_id {
        if flatpak_installed(app_id).await {
            found.push(EmulatorInstall {
                adapter_id: adapter_id.to_string(),
                name: format!("{display_name} (Flatpak)"),
                version: flatpak_version(app_id).await,
                install_source: InstallSource::Flatpak,
                executable_path: format!("flatpak run {app_id}"),
                flatpak_app_id: Some(app_id.to_string()),
            });
        }
    }

    found
}

/// Build a `tokio::process::Command` for a launch, transparently handling the
/// Flatpak case (`flatpak run <app> <args>`) vs a native binary.
///
/// Note: no `--` separator before the game args. `flatpak run` stops parsing its
/// own options at the app id and forwards everything after it (including
/// single-dash flags like `-cart`) to the sandboxed app — and it forwards a
/// literal `--` too, which strict parsers like BlastEm reject ("Unrecognized
/// switch --"). SDL-based emulators silently ignored it, which masked the bug.
pub fn build_command(ctx: &LaunchContext, game_args: Vec<String>) -> Result<Command, AdapterError> {
    let mut cmd = if let Some(app_id) = &ctx.flatpak_app_id {
        let mut c = Command::new("flatpak");
        c.arg("run").arg(app_id);
        c
    } else {
        let exe = Path::new(&ctx.executable_path);
        if !exe.exists() {
            return Err(AdapterError::ExecutableMissing(ctx.executable_path.clone()));
        }
        Command::new(exe)
    };

    for (key, value) in &ctx.env {
        cmd.env(key, value);
    }
    cmd.args(&ctx.extra_args);
    cmd.args(game_args);
    Ok(cmd)
}
