// Arcadia · Dreamshell — Tauri entry point.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod gamepad;
mod sdl_probe;

use commands::AppState;
use dreamvault::{ArcadiaPaths, Engine};
use tauri::Manager;
use tracing_subscriber::prelude::*;

fn main() {
    // WebKitGTK's DMABUF renderer crashes on many Wayland compositors
    // (Hyprland / wlroots / NVIDIA), surfacing as "Error 71 (Protocol error)
    // dispatching to Wayland display". Disabling it is the standard fix and is
    // harmless on X11/other compositors. Respect an explicit user override.
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    // Logs go to $XDG_STATE_HOME/arcadia/arcadia.log (rotated daily) as well as
    // stdout. The non-blocking writer's guard must outlive the program, so it's
    // bound for the whole of `main`. Path discovery failing here is non-fatal —
    // we fall back to stdout-only so the app still starts.
    let _log_guard = init_tracing();

    // Bootstrap the engine before the UI: open the DB, run migrations, seed
    // platforms, ensure a default profile. If this fails the app can't run.
    let engine = tauri::async_runtime::block_on(async {
        Engine::bootstrap().await.expect("failed to bootstrap DreamVault engine")
    });

    tracing::info!(db = ?engine.paths.database_path(), "DreamVault engine ready");

    tauri::Builder::default()
        .manage(AppState { engine })
        .setup(|app| {
            // Allow the asset protocol to serve cached cover art. The static
            // `assetScope` glob in tauri.conf.json does not match paths through
            // a dotdir like `~/.cache`, so grant the artwork directory (and its
            // subfolders) explicitly at runtime.
            let artwork = app.state::<AppState>().engine.paths.artwork_dir();
            let _ = app.asset_protocol_scope().allow_directory(&artwork, true);
            // Imported screenshots live under the cache dir too; grant them so
            // the gallery can render through the asset protocol.
            let shots = app.state::<AppState>().engine.paths.screenshots_dir();
            let _ = app.asset_protocol_scope().allow_directory(&shots, true);

            // RetroArch writes savestate thumbnails (`<rom>.stateN.png`) into its
            // own states directory, which lives outside Arcadia's dirs. Grant the
            // native and Flatpak locations (read-only) so the Recall State grid
            // can render those previews through the asset protocol. The dirs may
            // not exist yet; granting a missing path just adds it to the scope.
            if let Some(home) = std::env::var_os("HOME").map(std::path::PathBuf::from) {
                let scope = app.asset_protocol_scope();
                let _ = scope.allow_directory(home.join(".config/retroarch/states"), true);
                let _ = scope.allow_directory(
                    home.join(".var/app/org.libretro.RetroArch/config/retroarch/states"),
                    true,
                );
            }

            // Forward engine "play session ended" notifications to the webview
            // so the UI refreshes playtime/stats when an emulator exits, rather
            // than polling. The session is attributed asynchronously, often
            // minutes after launch_game returns.
            let mut rx = app.state::<AppState>().engine.subscribe_sessions();
            let handle = app.handle();
            tauri::async_runtime::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(ev) => {
                            let _ = handle.emit_all("arcadia://session-ended", ev);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // Forward background-job progress (library scan, box-art fetch) to
            // the webview so the sidebar can show a live progress bar instead of
            // a spinner that looks frozen on large libraries.
            let mut prog_rx = app.state::<AppState>().engine.subscribe_progress();
            let prog_handle = app.handle();
            tauri::async_runtime::spawn(async move {
                loop {
                    match prog_rx.recv().await {
                        Ok(ev) => {
                            let _ = prog_handle.emit_all("arcadia://progress", ev);
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });

            // WebKitGTK doesn't reliably expose controllers to the webview, so
            // read them ourselves and push state to `navigator.getGamepads()`.
            gamepad::spawn(app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_profiles,
            commands::default_profile,
            commands::create_profile,
            commands::detect_emulators,
            commands::list_emulators,
            commands::adapters,
            commands::emulator_update_hint,
            commands::platform_emulator_options,
            commands::set_default_emulator,
            commands::list_rom_sources,
            commands::add_rom_source,
            commands::remove_rom_source,
            commands::scan_library,
            commands::enrich_metadata,
            commands::screenscraper_credentials,
            commands::set_screenscraper_credentials,
            commands::list_games,
            commands::get_game,
            commands::set_favorite,
            commands::set_custom_title,
            commands::set_game_emulator,
            commands::get_game_settings,
            commands::set_game_settings,
            commands::set_game_cover,
            commands::suggest_covers,
            commands::refetch_cover,
            commands::launch_game,
            commands::list_discs,
            commands::launch_game_disc,
            commands::launch_into_state,
            commands::remove_game,
            commands::backup_game_saves,
            commands::list_save_backups,
            commands::restore_save_backup,
            commands::delete_save_backup,
            commands::list_save_states,
            commands::game_supports_launch_state,
            commands::game_hotkeys,
            commands::scan_screenshots,
            commands::list_screenshots,
            commands::recent_screenshots,
            commands::delete_screenshot,
            commands::retroachievements_credentials,
            commands::set_retroachievements_credentials,
            commands::retroachievements_configured,
            commands::search_ra_games,
            commands::link_ra_game,
            commands::unlink_ra_game,
            commands::get_ra_link,
            commands::refresh_achievements,
            commands::list_achievements,
            commands::ra_user_summary,
            commands::list_ra_linked_games,
            commands::controller_config,
            commands::save_controller_profile,
            commands::delete_controller_profile,
            commands::set_active_controller_profile,
            commands::hidapi_status,
            commands::set_hidapi_workaround,
            commands::console_pad,
            commands::system_controller_profiles,
            commands::save_system_controller_profile,
            commands::delete_system_controller_profile,
            commands::assign_system_controller_profile,
            commands::preview_system_controller_profile,
            commands::apply_system_controller_profile,
            commands::sync_config,
            commands::set_sync_config,
            commands::sync_status,
            commands::sync_push,
            commands::sync_pull,
            commands::list_plugins,
            commands::plugins_dir,
            commands::set_plugin_enabled,
            commands::run_plugin_transform,
            commands::library_stats,
            commands::list_collections,
            commands::create_collection,
            commands::collection_summaries,
            commands::remove_collection,
            commands::collection_games,
            commands::game_collections,
            commands::add_game_to_collection,
            commands::remove_game_from_collection,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Arcadia");
}

/// Initialise tracing to both stdout and a daily-rotated file under the XDG
/// state dir. Returns the non-blocking writer's guard (kept alive by the
/// caller); `None` if the state dir couldn't be prepared, in which case only
/// the stdout layer is installed.
fn init_tracing() -> Option<tracing_appender::non_blocking::WorkerGuard> {
    let env_filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,dreamvault=debug".into())
    };
    let stdout_layer = tracing_subscriber::fmt::layer().with_writer(std::io::stdout);

    match ArcadiaPaths::discover() {
        Ok(paths) if paths.ensure().is_ok() => {
            let appender = tracing_appender::rolling::daily(&paths.state_dir, "arcadia.log");
            let (file_writer, guard) = tracing_appender::non_blocking(appender);
            tracing_subscriber::registry()
                .with(env_filter())
                .with(stdout_layer)
                .with(
                    tracing_subscriber::fmt::layer()
                        .with_ansi(false)
                        .with_writer(file_writer),
                )
                .init();
            tracing::info!(log_dir = ?paths.state_dir, "logging to file + stdout");
            Some(guard)
        }
        _ => {
            tracing_subscriber::registry()
                .with(env_filter())
                .with(stdout_layer)
                .init();
            tracing::warn!("could not resolve state dir; logging to stdout only");
            None
        }
    }
}
