// Typed bindings over the Tauri IPC commands exposed by src-tauri/commands.rs.
import { invoke } from "@tauri-apps/api/tauri";
import { convertFileSrc } from "@tauri-apps/api/tauri";
import type {
  Achievement,
  AdapterDescriptor,
  Collection,
  CollectionSummary,
  ControllerConfig,
  ControllerProfile,
  Emulator,
  Game,
  GameDisc,
  GameQuery,
  GameSettings,
  LaunchResult,
  LibraryStats,
  PlatformEmulators,
  PluginInfo,
  Profile,
  RaGameCandidate,
  RaGameProgress,
  RaLink,
  RaUserSummary,
  RetroAchievementsCredentials,
  RomSource,
  SaveBackup,
  SaveState,
  ScanReport,
  Screenshot,
  ScreenScraperCredentials,
  SyncConfig,
  SyncReport,
  SyncStatus,
} from "./types";

export const api = {
  // Profiles
  listProfiles: () => invoke<Profile[]>("list_profiles"),
  defaultProfile: () => invoke<Profile>("default_profile"),
  createProfile: (name: string) => invoke<Profile>("create_profile", { name }),

  // Emulator registry
  detectEmulators: () => invoke<Emulator[]>("detect_emulators"),
  listEmulators: () => invoke<Emulator[]>("list_emulators"),
  adapters: () => invoke<AdapterDescriptor[]>("adapters"),
  emulatorUpdateHint: (emulatorId: string) =>
    invoke<string | null>("emulator_update_hint", { emulatorId }),
  platformEmulatorOptions: () =>
    invoke<PlatformEmulators[]>("platform_emulator_options"),
  setDefaultEmulator: (platform: string, emulatorId: string | null) =>
    invoke<void>("set_default_emulator", { platform, emulatorId }),

  // ROM sources + scanning
  listRomSources: (profileId: string) =>
    invoke<RomSource[]>("list_rom_sources", { profileId }),
  addRomSource: (profileId: string, path: string) =>
    invoke<RomSource>("add_rom_source", { profileId, path }),
  removeRomSource: (id: string) => invoke<void>("remove_rom_source", { id }),
  scanLibrary: (profileId: string) =>
    invoke<ScanReport>("scan_library", { profileId }),
  enrichMetadata: (profileId: string) =>
    invoke<number>("enrich_metadata", { profileId }),

  // Configuration
  screenScraperCredentials: () =>
    invoke<ScreenScraperCredentials>("screenscraper_credentials"),
  setScreenScraperCredentials: (credentials: ScreenScraperCredentials) =>
    invoke<void>("set_screenscraper_credentials", { credentials }),

  // Games
  listGames: (query: GameQuery) => invoke<Game[]>("list_games", { query }),
  getGame: (id: string) => invoke<Game | null>("get_game", { id }),
  setFavorite: (gameId: string, favorite: boolean) =>
    invoke<void>("set_favorite", { gameId, favorite }),
  setGameEmulator: (gameId: string, emulatorId: string | null) =>
    invoke<void>("set_game_emulator", { gameId, emulatorId }),
  getGameSettings: (gameId: string) =>
    invoke<GameSettings>("get_game_settings", { gameId }),
  setGameSettings: (settings: GameSettings) =>
    invoke<void>("set_game_settings", { settings }),
  setGameCover: (gameId: string, sourcePath: string) =>
    invoke<string>("set_game_cover", { gameId, sourcePath }),
  suggestCovers: (gameId: string, query: string | null) =>
    invoke<string[]>("suggest_covers", { gameId, query }),
  refetchCover: (gameId: string, query: string | null) =>
    invoke<boolean>("refetch_cover", { gameId, query }),
  launchGame: (gameId: string) =>
    invoke<LaunchResult>("launch_game", { gameId }),
  listDiscs: (gameId: string) => invoke<GameDisc[]>("list_discs", { gameId }),
  launchGameDisc: (gameId: string, discNumber: number) =>
    invoke<LaunchResult>("launch_game_disc", { gameId, discNumber }),
  launchIntoState: (gameId: string, slot: number) =>
    invoke<LaunchResult>("launch_into_state", { gameId, slot }),
  removeGame: (gameId: string) => invoke<void>("remove_game", { gameId }),

  // Save management
  backupGameSaves: (gameId: string, note: string | null) =>
    invoke<SaveBackup | null>("backup_game_saves", { gameId, note }),
  listSaveBackups: (gameId: string) =>
    invoke<SaveBackup[]>("list_save_backups", { gameId }),
  restoreSaveBackup: (backupId: string) =>
    invoke<SaveBackup | null>("restore_save_backup", { backupId }),
  deleteSaveBackup: (backupId: string) =>
    invoke<void>("delete_save_backup", { backupId }),
  listSaveStates: (gameId: string) =>
    invoke<SaveState[]>("list_save_states", { gameId }),
  gameSupportsLaunchState: (gameId: string) =>
    invoke<boolean>("game_supports_launch_state", { gameId }),

  // Screenshots
  scanScreenshots: (profileId: string) =>
    invoke<number>("scan_screenshots", { profileId }),
  listScreenshots: (gameId: string) =>
    invoke<Screenshot[]>("list_screenshots", { gameId }),
  recentScreenshots: (profileId: string, limit: number) =>
    invoke<Screenshot[]>("recent_screenshots", { profileId, limit }),
  deleteScreenshot: (id: string) => invoke<void>("delete_screenshot", { id }),

  // Achievements (RetroAchievements)
  retroachievementsCredentials: () =>
    invoke<RetroAchievementsCredentials>("retroachievements_credentials"),
  setRetroachievementsCredentials: (credentials: RetroAchievementsCredentials) =>
    invoke<void>("set_retroachievements_credentials", { credentials }),
  retroachievementsConfigured: () =>
    invoke<boolean>("retroachievements_configured"),
  searchRaGames: (platform: string, query: string) =>
    invoke<RaGameCandidate[]>("search_ra_games", { platform, query }),
  linkRaGame: (gameId: string, raGameId: number) =>
    invoke<number>("link_ra_game", { gameId, raGameId }),
  unlinkRaGame: (gameId: string) => invoke<void>("unlink_ra_game", { gameId }),
  getRaLink: (gameId: string) => invoke<RaLink | null>("get_ra_link", { gameId }),
  refreshAchievements: (gameId: string) =>
    invoke<number>("refresh_achievements", { gameId }),
  listAchievements: (gameId: string) =>
    invoke<Achievement[]>("list_achievements", { gameId }),
  raUserSummary: () => invoke<RaUserSummary>("ra_user_summary"),
  listRaLinkedGames: (profileId: string) =>
    invoke<RaGameProgress[]>("list_ra_linked_games", { profileId }),

  // Controller Center
  controllerConfig: () => invoke<ControllerConfig>("controller_config"),
  saveControllerProfile: (profile: ControllerProfile) =>
    invoke<ControllerProfile>("save_controller_profile", { profile }),
  deleteControllerProfile: (id: string) =>
    invoke<void>("delete_controller_profile", { id }),
  setActiveControllerProfile: (id: string | null) =>
    invoke<void>("set_active_controller_profile", { id }),

  // Cloud Sync
  syncConfig: () => invoke<SyncConfig>("sync_config"),
  setSyncConfig: (config: SyncConfig) =>
    invoke<void>("set_sync_config", { config }),
  syncStatus: () => invoke<SyncStatus>("sync_status"),
  syncPush: () => invoke<SyncReport>("sync_push"),
  syncPull: () => invoke<SyncReport>("sync_pull"),

  // Community plugins
  listPlugins: () => invoke<PluginInfo[]>("list_plugins"),
  pluginsDir: () => invoke<string>("plugins_dir"),
  setPluginEnabled: (pluginId: string, enabled: boolean) =>
    invoke<void>("set_plugin_enabled", { pluginId, enabled }),
  runPluginTransform: (pluginId: string, input: string) =>
    invoke<string>("run_plugin_transform", { pluginId, input }),

  // Stats + collections
  libraryStats: (profileId: string) =>
    invoke<LibraryStats>("library_stats", { profileId }),
  listCollections: (profileId: string) =>
    invoke<Collection[]>("list_collections", { profileId }),
  createCollection: (profileId: string, name: string) =>
    invoke<Collection>("create_collection", { profileId, name }),
  collectionSummaries: (profileId: string) =>
    invoke<CollectionSummary[]>("collection_summaries", { profileId }),
  removeCollection: (id: string) =>
    invoke<void>("remove_collection", { id }),
  collectionGames: (collectionId: string) =>
    invoke<Game[]>("collection_games", { collectionId }),
  gameCollections: (gameId: string) =>
    invoke<string[]>("game_collections", { gameId }),
  addGameToCollection: (collectionId: string, gameId: string) =>
    invoke<void>("add_game_to_collection", { collectionId, gameId }),
  removeGameFromCollection: (collectionId: string, gameId: string) =>
    invoke<void>("remove_game_from_collection", { collectionId, gameId }),
};

/// Resolve a local artwork path to a URL the webview can load (asset protocol).
/// `version` is an optional cache-buster: covers live at stable, deterministic
/// paths, so a corrected cover reuses the same URL and the webview would serve
/// the stale image. Bumping `version` on a cover change appends `?v=N`, which
/// the asset protocol ignores for resolution but the cache treats as distinct.
export function artworkUrl(path: string | null, version = 0): string | null {
  if (!path) return null;
  try {
    const url = convertFileSrc(path);
    return version > 0 ? `${url}?v=${version}` : url;
  } catch {
    return null;
  }
}
