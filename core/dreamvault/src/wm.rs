//! Best-effort window placement for the launch path.
//!
//! On Wayland a launching application *cannot* position another app's window —
//! that is the compositor's job, and there is no protocol for "open this client
//! on monitor X". Hyprland (which Arcadia targets) opens new windows on the
//! *focused* monitor; with focus-follows-mouse enabled that may not be the
//! monitor Dreamshell is on, so an emulator can appear on the "wrong" screen.
//!
//! As a courtesy, when we detect Hyprland we capture the focused monitor at
//! launch and then ask `hyprctl` to relocate the freshly-spawned emulator
//! window back onto it. Every step fails silently: this never blocks or affects
//! launching, and is a complete no-op on other compositors. It only matches the
//! window by the process pid, so it covers native emulators (e.g. Mupen64Plus);
//! Flatpak launches run under a different pid and are simply skipped.

use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;

/// Are we running under Hyprland?
pub fn hyprland() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

async fn hyprctl_json(subcommand: &str) -> Option<Value> {
    let out = Command::new("hyprctl")
        .arg(subcommand)
        .arg("-j")
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Workspace id of the currently focused monitor — i.e. where Dreamshell is at
/// the moment of launch, before the emulator window maps.
pub async fn focused_workspace() -> Option<i64> {
    let monitors = hyprctl_json("monitors").await?;
    monitors
        .as_array()?
        .iter()
        .find(|m| m.get("focused").and_then(Value::as_bool).unwrap_or(false))
        .and_then(|m| m.get("activeWorkspace"))
        .and_then(|w| w.get("id"))
        .and_then(Value::as_i64)
}

/// Spawn a detached task that waits for the emulator window (matched by `pid`)
/// to appear and silently moves it to `workspace` (which lives on the monitor
/// Dreamshell launched from). Bounded poll; gives up quietly after a few
/// seconds if the window never shows or its pid can't be matched.
pub fn relocate_to_workspace(pid: u32, workspace: i64) {
    tokio::spawn(async move {
        for _ in 0..20 {
            tokio::time::sleep(Duration::from_millis(150)).await;

            let Some(clients) = hyprctl_json("clients").await else {
                continue;
            };
            let Some(addr) = clients.as_array().and_then(|arr| {
                arr.iter()
                    .find(|c| c.get("pid").and_then(Value::as_i64) == Some(pid as i64))
                    .and_then(|c| c.get("address"))
                    .and_then(Value::as_str)
                    .map(String::from)
            }) else {
                continue;
            };

            // `movetoworkspacesilent` relocates a specific window without
            // stealing focus or switching the visible workspace.
            let _ = Command::new("hyprctl")
                .arg("dispatch")
                .arg("movetoworkspacesilent")
                .arg(format!("{workspace},address:{addr}"))
                .output()
                .await;
            tracing::debug!(pid, workspace, "relocated emulator window to launch monitor");
            return;
        }
        tracing::debug!(pid, "emulator window not found for relocation; left to compositor");
    });
}
