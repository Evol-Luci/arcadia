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

/// True when we are running inside a Flatpak sandbox. The runtime always
/// bind-mounts `/.flatpak-info` (read-only) into every sandbox and it never
/// exists on the host, so its presence is the canonical sandbox marker.
///
/// Why this matters: Arcadia discovers and launches *host-installed* emulators,
/// but a sandboxed process sees only the runtime's `/app:/usr` — host binaries,
/// `pacman`, and `flatpak` are all invisible. When sandboxed we tunnel those
/// calls through `flatpak-spawn --host` (granted by the manifest's
/// `--talk-name=org.freedesktop.Flatpak`).
pub fn in_flatpak_sandbox() -> bool {
    Path::new("/.flatpak-info").exists()
}

/// Build a `Command` for a host tool. Inside a Flatpak sandbox the program lives
/// on the host, so we prefix `flatpak-spawn --host`; on a normal install this is
/// a plain passthrough.
fn host_command(program: &str) -> Command {
    if in_flatpak_sandbox() {
        let mut c = Command::new("flatpak-spawn");
        c.arg("--host").arg(program);
        c
    } else {
        Command::new(program)
    }
}

/// Resolve an executable name to an absolute path, honouring the host when
/// sandboxed. On the host this is the plain `$PATH` walk; inside a sandbox the
/// host's `$PATH` and binaries are invisible, so we ask the host shell to
/// resolve the name (`command -v`). The name is passed as a positional argument
/// rather than interpolated, so an odd name can't break out of the shell word.
async fn resolve_executable(name: &str) -> Option<PathBuf> {
    if in_flatpak_sandbox() {
        let out = Command::new("flatpak-spawn")
            .args(["--host", "sh", "-c", "command -v \"$1\"", "sh"])
            .arg(name)
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (!path.is_empty()).then(|| PathBuf::from(path))
    } else {
        which(name)
    }
}

/// The pacman package that owns `path`, if any. This is how we determine both
/// install source and version *without executing the emulator* — running an
/// unknown GUI binary with `--version` is unsafe (it can pop dialogs, launch
/// the app, or inhibit the screensaver). Returns the package name on success.
pub async fn pacman_owner(path: &Path) -> Option<String> {
    let out = host_command("pacman").arg("-Qoq").arg(path).output().await.ok()?;
    if !out.status.success() {
        return None;
    }
    let name = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!name.is_empty()).then_some(name)
}

/// Version of an installed pacman package, parsed from `pacman -Q <pkg>`
/// ("pkgname 1.2.3-1" → "1.2.3-1").
pub async fn pacman_version(pkg: &str) -> Option<String> {
    let out = host_command("pacman").arg("-Q").arg(pkg).output().await.ok()?;
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
    host_command("flatpak")
        .args(["info", app_id])
        .output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Version of an installed Flatpak from `flatpak info` metadata, if it exposes
/// a `Version:` field. Also side-effect-free.
pub async fn flatpak_version(app_id: &str) -> Option<String> {
    let out = host_command("flatpak").args(["info", app_id]).output().await.ok()?;
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
        if let Some(path) = resolve_executable(name).await {
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
    let sandboxed = in_flatpak_sandbox();

    // When sandboxed, the emulator runs on the host via `flatpak-spawn --host`,
    // so per-game env vars must be forwarded as `--env=` flags *before* the
    // program (setting them on the flatpak-spawn process would not reach the host
    // child). On the host we set them on the child directly.
    let mut cmd = if sandboxed {
        let mut c = Command::new("flatpak-spawn");
        c.arg("--host");
        for (key, value) in &ctx.env {
            c.arg(format!("--env={key}={value}"));
        }
        if let Some(app_id) = &ctx.flatpak_app_id {
            // The emulator is itself a Flatpak: we still can't `flatpak run` from
            // inside our own sandbox, so run the host's flatpak CLI.
            c.arg("flatpak").arg("run").arg(app_id);
        } else {
            // Trust detection for the host path: we cannot `stat` it from inside
            // the sandbox, where the host filesystem root is not mapped.
            c.arg(&ctx.executable_path);
        }
        c
    } else if let Some(app_id) = &ctx.flatpak_app_id {
        let mut c = Command::new("flatpak");
        c.arg("run").arg(app_id);
        for (key, value) in &ctx.env {
            c.env(key, value);
        }
        c
    } else {
        let exe = Path::new(&ctx.executable_path);
        if !exe.exists() {
            return Err(AdapterError::ExecutableMissing(ctx.executable_path.clone()));
        }
        let mut c = Command::new(exe);
        for (key, value) in &ctx.env {
            c.env(key, value);
        }
        c
    };

    cmd.args(&ctx.extra_args);
    cmd.args(game_args);
    Ok(cmd)
}
