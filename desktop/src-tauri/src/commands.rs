//! Tauri IPC surface. Thin wrappers that translate between the frontend and
//! the DreamVault engine. All commands are async and return `Result<T, String>`
//! so the React side gets a clean error string to render.

use dreamvault::achievements::{RaGameCandidate, RaGameProgress, RaUserSummary};
use dreamvault::adapters::AdapterDescriptor;
use dreamvault::config::{
    ControllerConfig, ControllerProfile, HidapiWorkaround, LaunchBoxConfig,
    RetroAchievementsCredentials, ScreenScraperCredentials, SyncConfig, SystemControllerProfile,
};
use dreamvault::console_pads::ConsolePad;
use dreamvault::controller::{ApplyOutcome, HidapiStatus, MaterializePreview};
use dreamvault::library::{GameQuery, ScanReport};
use dreamvault::models::{
    Achievement, Collection, CollectionSummary, Emulator, Game, GameDisc, GameSettings,
    LibraryStats, Profile, RaLink, RomSource, SaveBackup, Screenshot,
};
use dreamvault::plugins::PluginInfo;
use dreamvault::registry::PlatformEmulators;
use dreamvault::hotkeys::EmulatorHotkey;
use dreamvault::save_states::SaveState;
use dreamvault::stats::LaunchResult;
use dreamvault::sync::{SyncReport, SyncStatus};
use dreamvault::Engine;
use tauri::State;

/// Managed engine handle.
pub struct AppState {
    pub engine: Engine,
}

type CmdResult<T> = Result<T, String>;

fn map<T>(r: dreamvault::Result<T>) -> CmdResult<T> {
    r.map_err(|e| e.to_string())
}

// ---- Profiles -------------------------------------------------------------

#[tauri::command]
pub async fn list_profiles(state: State<'_, AppState>) -> CmdResult<Vec<Profile>> {
    map(state.engine.list_profiles().await)
}

#[tauri::command]
pub async fn default_profile(state: State<'_, AppState>) -> CmdResult<Profile> {
    map(state.engine.ensure_default_profile().await)
}

#[tauri::command]
pub async fn create_profile(state: State<'_, AppState>, name: String) -> CmdResult<Profile> {
    map(state.engine.create_profile(&name).await)
}

// ---- Emulator registry ----------------------------------------------------

#[tauri::command]
pub async fn detect_emulators(state: State<'_, AppState>) -> CmdResult<Vec<Emulator>> {
    map(state.engine.detect_emulators().await)
}

#[tauri::command]
pub async fn list_emulators(state: State<'_, AppState>) -> CmdResult<Vec<Emulator>> {
    map(state.engine.list_emulators().await)
}

#[tauri::command]
pub fn adapters(state: State<'_, AppState>) -> Vec<AdapterDescriptor> {
    state.engine.adapter_descriptors()
}

/// The update command to *show* the user for an externally-managed emulator,
/// honouring the install-ownership rule (we inform, we don't run it silently).
#[tauri::command]
pub async fn emulator_update_hint(
    state: State<'_, AppState>,
    emulator_id: String,
) -> CmdResult<Option<String>> {
    let e = map(state.engine.get_emulator(&emulator_id).await)?;
    Ok(e.and_then(|e| Engine::update_hint(&e)))
}

/// Per-platform emulator choices for the Settings "Default Emulators" panel:
/// each platform with a compatible installed emulator, its candidates, and the
/// user's current default.
#[tauri::command]
pub async fn platform_emulator_options(
    state: State<'_, AppState>,
) -> CmdResult<Vec<PlatformEmulators>> {
    map(state.engine.platform_emulator_options().await)
}

/// Pin (or clear, with `null`) the default emulator used to launch a platform.
#[tauri::command]
pub fn set_default_emulator(
    state: State<'_, AppState>,
    platform: String,
    emulator_id: Option<String>,
) -> CmdResult<()> {
    map(state
        .engine
        .set_default_emulator(&platform, emulator_id.as_deref()))
}

// ---- ROM sources + scanning ----------------------------------------------

#[tauri::command]
pub async fn list_rom_sources(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<Vec<RomSource>> {
    map(state.engine.list_rom_sources(&profile_id).await)
}

#[tauri::command]
pub async fn add_rom_source(
    state: State<'_, AppState>,
    profile_id: String,
    path: String,
) -> CmdResult<RomSource> {
    map(state.engine.add_rom_source(&profile_id, &path).await)
}

#[tauri::command]
pub async fn remove_rom_source(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    map(state.engine.remove_rom_source(&id).await)
}

#[tauri::command]
pub async fn scan_library(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<ScanReport> {
    let report = map(state.engine.scan_library(&profile_id).await)?;
    // Best-effort local artwork pass after a scan; ignore its count here.
    let _ = state.engine.enrich_metadata_local(&profile_id).await;
    Ok(report)
}

#[tauri::command]
pub async fn enrich_metadata(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<usize> {
    map(state.engine.enrich_metadata_online(&profile_id).await)
}

// ---- Configuration (user-owned credentials) -------------------------------

/// The user's saved ScreenScraper credentials, for pre-filling the Settings
/// form. Returned verbatim (local single-user app; the keys are the user's own).
#[tauri::command]
pub fn screenscraper_credentials(state: State<'_, AppState>) -> ScreenScraperCredentials {
    state.engine.load_config().screenscraper
}

/// Persist the user's ScreenScraper credentials.
#[tauri::command]
pub fn set_screenscraper_credentials(
    state: State<'_, AppState>,
    credentials: ScreenScraperCredentials,
) -> CmdResult<()> {
    let mut cfg = state.engine.load_config();
    cfg.screenscraper = credentials;
    map(state.engine.save_config(&cfg))
}

/// The user's LaunchBox cover settings (enable flag + last refresh time).
#[tauri::command]
pub fn launchbox_config(state: State<'_, AppState>) -> LaunchBoxConfig {
    state.engine.load_config().launchbox
}

/// Toggle the LaunchBox cover source on/off. Does not download anything.
#[tauri::command]
pub fn set_launchbox_enabled(state: State<'_, AppState>, enabled: bool) -> CmdResult<()> {
    let mut cfg = state.engine.load_config();
    cfg.launchbox.enabled = enabled;
    map(state.engine.save_config(&cfg))
}

/// Download the LaunchBox metadata dump and rebuild the cover index. Heavy
/// (hundreds of MB). Returns the number of indexed entries.
#[tauri::command]
pub async fn refresh_launchbox_index(state: State<'_, AppState>) -> CmdResult<usize> {
    map(state.engine.refresh_launchbox_index().await)
}

// ---- Games ----------------------------------------------------------------

#[tauri::command]
pub async fn list_games(state: State<'_, AppState>, query: GameQuery) -> CmdResult<Vec<Game>> {
    map(state.engine.list_games(&query).await)
}

#[tauri::command]
pub async fn get_game(state: State<'_, AppState>, id: String) -> CmdResult<Option<Game>> {
    map(state.engine.get_game(&id).await)
}

#[tauri::command]
pub async fn set_favorite(
    state: State<'_, AppState>,
    game_id: String,
    favorite: bool,
) -> CmdResult<()> {
    map(state.engine.set_favorite(&game_id, favorite).await)
}

#[tauri::command]
pub async fn set_custom_title(
    state: State<'_, AppState>,
    game_id: String,
    title: Option<String>,
) -> CmdResult<()> {
    map(state
        .engine
        .set_custom_title(&game_id, title.as_deref())
        .await)
}

#[tauri::command]
pub async fn set_game_emulator(
    state: State<'_, AppState>,
    game_id: String,
    emulator_id: Option<String>,
) -> CmdResult<()> {
    map(state
        .engine
        .set_game_emulator(&game_id, emulator_id.as_deref())
        .await)
}

#[tauri::command]
pub async fn get_game_settings(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<GameSettings> {
    map(state.engine.get_game_settings(&game_id).await)
}

#[tauri::command]
pub async fn set_game_settings(
    state: State<'_, AppState>,
    settings: GameSettings,
) -> CmdResult<()> {
    map(state.engine.set_game_settings(&settings).await)
}

#[tauri::command]
pub async fn launch_game(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<LaunchResult> {
    map(state.engine.launch_game(&game_id).await)
}

/// The ordered disc list for a game; empty for single-disc games. The details
/// page uses this to show a disc picker for multi-disc sets.
#[tauri::command]
pub async fn list_discs(state: State<'_, AppState>, game_id: String) -> CmdResult<Vec<GameDisc>> {
    map(state.engine.list_discs(&game_id).await)
}

/// Launch one specific disc of a multi-disc game (bypasses the auto `.m3u`).
#[tauri::command]
pub async fn launch_game_disc(
    state: State<'_, AppState>,
    game_id: String,
    disc_number: i64,
) -> CmdResult<LaunchResult> {
    map(state.engine.launch_game_disc(&game_id, disc_number).await)
}

/// Launch a game booting straight into one of its discovered savestate slots.
/// The Recall State grid invokes this when the user clicks a slot card.
#[tauri::command]
pub async fn launch_into_state(
    state: State<'_, AppState>,
    game_id: String,
    slot: i64,
) -> CmdResult<LaunchResult> {
    map(state.engine.launch_game_into_state(&game_id, slot).await)
}

/// Override a game's box art with a local image file the user picks.
#[tauri::command]
pub async fn set_game_cover(
    state: State<'_, AppState>,
    game_id: String,
    source_path: String,
) -> CmdResult<String> {
    map(state.engine.set_game_cover(&game_id, &source_path).await)
}

/// Closest libretro box-art names for a game, ranked, for the user to choose.
#[tauri::command]
pub async fn suggest_covers(
    state: State<'_, AppState>,
    game_id: String,
    query: Option<String>,
) -> CmdResult<Vec<String>> {
    map(state.engine.suggest_covers(&game_id, query.as_deref()).await)
}

/// Re-fetch one game's box art from libretro, optionally with a corrected name.
#[tauri::command]
pub async fn refetch_cover(
    state: State<'_, AppState>,
    game_id: String,
    query: Option<String>,
) -> CmdResult<bool> {
    map(state.engine.refetch_cover(&game_id, query.as_deref()).await)
}

// ---- Stats + collections --------------------------------------------------

#[tauri::command]
pub async fn library_stats(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<LibraryStats> {
    map(state.engine.library_stats(&profile_id).await)
}

#[tauri::command]
pub async fn list_collections(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<Vec<Collection>> {
    map(state.engine.list_collections(&profile_id).await)
}

#[tauri::command]
pub async fn create_collection(
    state: State<'_, AppState>,
    profile_id: String,
    name: String,
) -> CmdResult<Collection> {
    map(state.engine.create_collection(&profile_id, &name).await)
}

#[tauri::command]
pub async fn collection_summaries(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<Vec<CollectionSummary>> {
    map(state.engine.list_collection_summaries(&profile_id).await)
}

#[tauri::command]
pub async fn remove_collection(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    map(state.engine.remove_collection(&id).await)
}

#[tauri::command]
pub async fn collection_games(
    state: State<'_, AppState>,
    collection_id: String,
) -> CmdResult<Vec<Game>> {
    map(state.engine.collection_games(&collection_id).await)
}

#[tauri::command]
pub async fn game_collections(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<Vec<String>> {
    map(state.engine.game_collections(&game_id).await)
}

#[tauri::command]
pub async fn add_game_to_collection(
    state: State<'_, AppState>,
    collection_id: String,
    game_id: String,
) -> CmdResult<()> {
    map(state
        .engine
        .add_game_to_collection(&collection_id, &game_id)
        .await)
}

#[tauri::command]
pub async fn remove_game_from_collection(
    state: State<'_, AppState>,
    collection_id: String,
    game_id: String,
) -> CmdResult<()> {
    map(state
        .engine
        .remove_game_from_collection(&collection_id, &game_id)
        .await)
}

#[tauri::command]
pub async fn remove_game(state: State<'_, AppState>, game_id: String) -> CmdResult<()> {
    map(state.engine.remove_game(&game_id).await)
}

// ---- Save management ------------------------------------------------------

/// Back up a game's current saves. Returns `None` if the game has none yet.
#[tauri::command]
pub async fn backup_game_saves(
    state: State<'_, AppState>,
    game_id: String,
    note: Option<String>,
) -> CmdResult<Option<SaveBackup>> {
    map(state.engine.backup_game_saves(&game_id, note.as_deref()).await)
}

#[tauri::command]
pub async fn list_save_backups(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<Vec<SaveBackup>> {
    map(state.engine.list_save_backups(&game_id).await)
}

/// Restore a backup over the live saves. The current saves are snapshotted
/// first (Risk-5); the returned backup, if any, is that pre-restore snapshot.
#[tauri::command]
pub async fn restore_save_backup(
    state: State<'_, AppState>,
    backup_id: String,
) -> CmdResult<Option<SaveBackup>> {
    map(state.engine.restore_save_backup(&backup_id).await)
}

#[tauri::command]
pub async fn delete_save_backup(state: State<'_, AppState>, backup_id: String) -> CmdResult<()> {
    map(state.engine.delete_save_backup(&backup_id).await)
}

/// A game's savestates as structured slots (slot index, thumbnail, mtime), for
/// the "Recall State" grid. Empty for adapters we don't map or games with none.
#[tauri::command]
pub async fn list_save_states(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<Vec<SaveState>> {
    map(state.engine.list_save_states(&game_id).await)
}

/// Whether this game's emulator can boot straight into a savestate. The Recall
/// State grid gates its cards on this: `true` → clickable; `false` → read-only
/// (the emulator has no CLI savestate-load, so a click would cold-boot).
#[tauri::command]
pub async fn game_supports_launch_state(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<bool> {
    map(state.engine.game_supports_launch_state(&game_id).await)
}

/// This game's emulator hotkeys (save/load state, slots, screenshot, pause,
/// fast-forward, fullscreen, menu, exit) as a read-only label→binding list,
/// overlaid on the emulator's documented defaults. Empty for emulators we don't
/// map, which the UI hides.
#[tauri::command]
pub async fn game_hotkeys(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<Vec<EmulatorHotkey>> {
    map(state.engine.game_hotkeys(&game_id).await)
}

// ---- Screenshots ----------------------------------------------------------

#[tauri::command]
pub async fn scan_screenshots(state: State<'_, AppState>, profile_id: String) -> CmdResult<usize> {
    map(state.engine.scan_screenshots(&profile_id).await)
}

#[tauri::command]
pub async fn list_screenshots(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<Vec<Screenshot>> {
    map(state.engine.list_screenshots(&game_id).await)
}

#[tauri::command]
pub async fn recent_screenshots(
    state: State<'_, AppState>,
    profile_id: String,
    limit: i64,
) -> CmdResult<Vec<Screenshot>> {
    map(state.engine.recent_screenshots(&profile_id, limit).await)
}

#[tauri::command]
pub async fn delete_screenshot(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    map(state.engine.delete_screenshot(&id).await)
}

// ---- Achievements (RetroAchievements) -------------------------------------

#[tauri::command]
pub fn retroachievements_credentials(state: State<'_, AppState>) -> RetroAchievementsCredentials {
    state.engine.load_config().retroachievements
}

#[tauri::command]
pub fn set_retroachievements_credentials(
    state: State<'_, AppState>,
    credentials: RetroAchievementsCredentials,
) -> CmdResult<()> {
    let mut cfg = state.engine.load_config();
    cfg.retroachievements = credentials;
    map(state.engine.save_config(&cfg))
}

#[tauri::command]
pub fn retroachievements_configured(state: State<'_, AppState>) -> bool {
    state.engine.retroachievements_configured()
}

#[tauri::command]
pub async fn search_ra_games(
    state: State<'_, AppState>,
    platform: String,
    query: String,
) -> CmdResult<Vec<RaGameCandidate>> {
    map(state.engine.search_ra_games(&platform, &query).await)
}

#[tauri::command]
pub async fn link_ra_game(
    state: State<'_, AppState>,
    game_id: String,
    ra_game_id: i64,
) -> CmdResult<usize> {
    map(state.engine.link_ra_game(&game_id, ra_game_id).await)
}

#[tauri::command]
pub async fn unlink_ra_game(state: State<'_, AppState>, game_id: String) -> CmdResult<()> {
    map(state.engine.unlink_ra_game(&game_id).await)
}

#[tauri::command]
pub async fn get_ra_link(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<Option<RaLink>> {
    map(state.engine.get_ra_link(&game_id).await)
}

#[tauri::command]
pub async fn refresh_achievements(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<usize> {
    map(state.engine.refresh_achievements(&game_id).await)
}

#[tauri::command]
pub async fn list_achievements(
    state: State<'_, AppState>,
    game_id: String,
) -> CmdResult<Vec<Achievement>> {
    map(state.engine.list_achievements(&game_id).await)
}

#[tauri::command]
pub async fn ra_user_summary(state: State<'_, AppState>) -> CmdResult<RaUserSummary> {
    map(state.engine.ra_user_summary().await)
}

#[tauri::command]
pub async fn list_ra_linked_games(
    state: State<'_, AppState>,
    profile_id: String,
) -> CmdResult<Vec<RaGameProgress>> {
    map(state.engine.list_ra_linked_games(&profile_id).await)
}

// ---- Controller Center ----------------------------------------------------

#[tauri::command]
pub fn controller_config(state: State<'_, AppState>) -> ControllerConfig {
    state.engine.controller_config()
}

#[tauri::command]
pub fn save_controller_profile(
    state: State<'_, AppState>,
    profile: ControllerProfile,
) -> CmdResult<ControllerProfile> {
    map(state.engine.save_controller_profile(profile))
}

#[tauri::command]
pub fn delete_controller_profile(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    map(state.engine.delete_controller_profile(&id))
}

#[tauri::command]
pub fn set_active_controller_profile(
    state: State<'_, AppState>,
    id: Option<String>,
) -> CmdResult<()> {
    map(state.engine.set_active_controller_profile(id.as_deref()))
}

#[tauri::command]
pub fn hidapi_status(state: State<'_, AppState>) -> HidapiStatus {
    state.engine.hidapi_status()
}

#[tauri::command]
pub fn set_hidapi_workaround(
    state: State<'_, AppState>,
    policy: HidapiWorkaround,
) -> CmdResult<()> {
    map(state.engine.set_hidapi_workaround(policy))
}

// ---- Per-system controller profiles (input-setup module) ------------------

/// The target input layout for a system, or `null` if not yet catalogued.
#[tauri::command]
pub fn console_pad(system: String) -> Option<ConsolePad> {
    dreamvault::console_pads::pad_for(&system).copied()
}

#[tauri::command]
pub fn system_controller_profiles(state: State<'_, AppState>) -> Vec<SystemControllerProfile> {
    state.engine.system_controller_profiles()
}

#[tauri::command]
pub fn save_system_controller_profile(
    state: State<'_, AppState>,
    profile: SystemControllerProfile,
) -> CmdResult<SystemControllerProfile> {
    map(state.engine.save_system_controller_profile(profile))
}

#[tauri::command]
pub fn delete_system_controller_profile(state: State<'_, AppState>, id: String) -> CmdResult<()> {
    map(state.engine.delete_system_controller_profile(&id))
}

#[tauri::command]
pub fn assign_system_controller_profile(
    state: State<'_, AppState>,
    system: String,
    profile_id: Option<String>,
) -> CmdResult<()> {
    map(state
        .engine
        .assign_system_controller_profile(&system, profile_id.as_deref()))
}

#[tauri::command]
pub fn preview_system_controller_profile(
    state: State<'_, AppState>,
    id: String,
) -> CmdResult<MaterializePreview> {
    map(state.engine.preview_system_controller_profile(&id))
}

#[tauri::command]
pub async fn apply_system_controller_profile(
    state: State<'_, AppState>,
    system: String,
) -> CmdResult<ApplyOutcome> {
    // Tier-B writers (Mupen64Plus, Mednafen) need the pad's raw SDL-joystick
    // element indices — and, for Mednafen, its joystick GUID — which only the
    // same libSDL2 the emulator uses can answer. Probe here in the Tauri layer
    // (where the SDL FFI lives) and hand the resolved mapping + GUID to the pure
    // engine. Tier-A (RetroArch) ignores both. A `None` probe (no controller /
    // SDL unavailable) is passed through so the engine can report a clean
    // "connect the pad" error for Tier-B rather than guessing.
    let probed = crate::sdl_probe::probe_first_controller();
    let mapping = probed.as_ref().map(|p| &p.mapping);
    let guid = probed.as_ref().and_then(|p| p.guid.as_deref());
    // mGBA's Qt frontend keys its per-device input-profile (which overrides the
    // generic SDLB section it loads first) by the raw joystick *name*.
    let name = probed.as_ref().and_then(|p| p.name.as_deref());
    map(state
        .engine
        .apply_system_controller_profile(&system, mapping, guid, name)
        .await)
}

// ---- Cloud Sync -----------------------------------------------------------

#[tauri::command]
pub fn sync_config(state: State<'_, AppState>) -> SyncConfig {
    state.engine.load_config().sync
}

#[tauri::command]
pub fn set_sync_config(state: State<'_, AppState>, config: SyncConfig) -> CmdResult<()> {
    let mut cfg = state.engine.load_config();
    cfg.sync = config;
    map(state.engine.save_config(&cfg))
}

#[tauri::command]
pub fn sync_status(state: State<'_, AppState>) -> SyncStatus {
    state.engine.sync_status()
}

#[tauri::command]
pub fn sync_push(state: State<'_, AppState>) -> CmdResult<SyncReport> {
    map(state.engine.sync_push())
}

#[tauri::command]
pub fn sync_pull(state: State<'_, AppState>) -> CmdResult<SyncReport> {
    map(state.engine.sync_pull())
}

// ---- Community plugins (sandboxed WASM) -----------------------------------

#[tauri::command]
pub fn list_plugins(state: State<'_, AppState>) -> Vec<PluginInfo> {
    state.engine.list_plugins()
}

#[tauri::command]
pub fn plugins_dir(state: State<'_, AppState>) -> String {
    state.engine.plugins_dir().to_string_lossy().to_string()
}

#[tauri::command]
pub fn set_plugin_enabled(
    state: State<'_, AppState>,
    plugin_id: String,
    enabled: bool,
) -> CmdResult<()> {
    map(state.engine.set_plugin_enabled(&plugin_id, enabled))
}

#[tauri::command]
pub fn run_plugin_transform(
    state: State<'_, AppState>,
    plugin_id: String,
    input: String,
) -> CmdResult<String> {
    map(state.engine.run_plugin_transform(&plugin_id, &input))
}
