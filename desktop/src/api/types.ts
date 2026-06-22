// Mirrors the serde-serialised types coming from the DreamVault engine.

export interface Profile {
  id: string;
  name: string;
  is_default: boolean;
  created_at: string;
}

export interface Emulator {
  id: string;
  name: string;
  version: string | null;
  install_source: string;
  owner: string;
  executable_path: string;
  adapter_id: string;
  supports_savestates: boolean;
  supports_saves: boolean;
  supports_achievements: boolean;
  supports_screenshots: boolean;
  detected_at: string;
}

export interface Capabilities {
  savestates: boolean;
  saves: boolean;
  achievements: boolean;
  screenshots: boolean;
}

export interface AdapterDescriptor {
  id: string;
  display_name: string;
  platforms: string[];
  capabilities: Capabilities;
}

/// Installed emulators able to launch one platform, plus the user's default.
export interface PlatformEmulators {
  platform: string;
  name: string;
  hint_adapter_id: string;
  default_emulator_id: string | null;
  emulators: Emulator[];
}

export interface Game {
  id: string;
  profile_id: string;
  title: string;
  custom_title: string | null;
  sort_title: string;
  platform: string;
  rom_path: string;
  file_size: number;
  emulator_id: string | null;
  cover_art: string | null;
  background_art: string | null;
  description: string | null;
  genre: string | null;
  developer: string | null;
  publisher: string | null;
  release_date: string | null;
  playtime_minutes: number;
  launch_count: number;
  favorite: boolean;
  last_played: string | null;
  added_at: string;
}

/// Per-game launch overrides injected into the emulator at launch. Empty fields
/// mean "no override": args/env are additive, core_override is RetroArch-only.
export interface GameSettings {
  game_id: string;
  extra_args: string[];
  env_vars: Record<string, string>;
  core_override: string | null;
}

/// One physical disc image of a multi-disc game, ordered by disc_number.
export interface GameDisc {
  game_id: string;
  disc_number: number;
  rom_path: string;
  label: string | null;
}

export interface RomSource {
  id: string;
  profile_id: string;
  path: string;
  added_at: string;
}

export interface Collection {
  id: string;
  profile_id: string;
  name: string;
  created_at: string;
}

export interface CollectionSummary {
  id: string;
  name: string;
  created_at: string;
  game_count: number;
}

export interface SaveBackup {
  id: string;
  game_id: string;
  kind: string;
  path: string;
  file_count: number;
  byte_size: number;
  note: string | null;
  created_at: string;
}

/// One savestate slot discovered on disk. slot: -1 = auto-save, 0 = base slot,
/// N = numbered slot. thumbnail is the sibling preview PNG, if the emulator
/// wrote one.
export interface SaveState {
  slot: number;
  label: string;
  path: string;
  thumbnail: string | null;
  byte_size: number;
  modified_at: string | null;
}

/// One canonical emulator hotkey action. Matches the Rust `HotkeyAction` enum
/// (serialized snake_case).
export type HotkeyAction =
  | "save_state"
  | "load_state"
  | "next_slot"
  | "prev_slot"
  | "screenshot"
  | "pause"
  | "fast_forward_hold"
  | "fast_forward_toggle"
  | "rewind"
  | "toggle_fullscreen"
  | "toggle_menu"
  | "exit"
  | "reset";

export type HotkeyDevice = "keyboard" | "controller";

/// One resolved emulator hotkey for display. binding is a normalized key string
/// ("F2", "Shift + F1"). is_default is true when sourced from the defaults table
/// rather than the user's own config.
export interface EmulatorHotkey {
  action: HotkeyAction;
  label: string;
  binding: string;
  is_default: boolean;
  device: HotkeyDevice;
}

export interface ScreenScraperCredentials {
  dev_id: string;
  dev_password: string;
  user_id: string;
  user_password: string;
}

export interface LaunchBoxConfig {
  enabled: boolean;
  last_refresh: number | null;
}

export interface Screenshot {
  id: string;
  game_id: string;
  path: string;
  source_path: string;
  byte_size: number;
  captured_at: string | null;
  created_at: string;
}

export interface RetroAchievementsCredentials {
  username: string;
  api_key: string;
}

export interface RaGameCandidate {
  ra_game_id: number;
  title: string;
}

export interface CoverCandidate {
  source: "libretro" | "launchbox";
  label: string;
  token: string;
}

export interface RaUserSummary {
  username: string;
  total_points: number;
  rank: number;
  recently_played_count: number;
}

export interface RaLink {
  game_id: string;
  ra_game_id: number;
  title: string | null;
  icon_url: string | null;
  linked_at: string;
}

export interface Achievement {
  id: string;
  game_id: string;
  ra_id: number;
  title: string;
  description: string | null;
  points: number;
  badge_url: string | null;
  unlocked: boolean;
  unlocked_at: string | null;
  display_order: number;
}

export interface RaGameProgress {
  game_id: string;
  title: string;
  cover_art: string | null;
  ra_title: string | null;
  icon_url: string | null;
  total: number;
  unlocked: number;
  points_earned: number;
  points_total: number;
}

export interface ControllerProfile {
  id: string;
  name: string;
  bindings: Record<string, string>;
}

export type HidapiWorkaround = "auto" | "force" | "off";

export interface ControllerConfig {
  profiles: ControllerProfile[];
  active_profile: string | null;
  system_profiles: SystemControllerProfile[];
  system_assignments: Record<string, string>;
  sdl_hidapi_workaround: HidapiWorkaround;
}

export type InputKind = "button" | "dpad" | "stick" | "trigger";

export interface ConsoleInput {
  id: string;
  label: string;
  kind: InputKind;
}

export interface ConsolePad {
  system: string;
  inputs: ConsoleInput[];
}

/** A per-console mapping. `bindings` keys are ConsolePad input ids; values are
 * compact W3C descriptors ("btn:0", "axis:1-"). */
export interface SystemControllerProfile {
  id: string;
  name: string;
  system: string;
  bindings: Record<string, string>;
}

/** Dry-run of materializing a profile into emulator config. */
export interface MaterializePreview {
  encoded: Record<string, string>;
  unencoded: string[];
}

/** Result of writing a profile into an emulator's config. */
export interface ApplyOutcome {
  config_path: string;
  backup_path: string;
  written: number;
  unencoded: string[];
  emulator: string;
}

export interface HidapiStatus {
  policy: HidapiWorkaround;
  xpadneo_present: boolean;
  effective: boolean;
}

export interface SyncConfig {
  enabled: boolean;
  folder: string | null;
}

export interface SyncStatus {
  configured: boolean;
  folder: string | null;
  folder_exists: boolean;
  local_backup_count: number;
  remote_backup_count: number;
}

export interface SyncReport {
  files_copied: number;
  conflicts: number;
}

export interface PluginManifest {
  id: string;
  name: string;
  version: string;
  kind: string;
  entry: string;
  description: string | null;
  author: string | null;
  capabilities: string[];
}

export interface PluginInfo {
  manifest: PluginManifest;
  enabled: boolean;
  valid: boolean;
  error: string | null;
  dir: string;
}

export interface ScanReport {
  scanned_files: number;
  added: number;
  skipped_existing: number;
  unrecognized: number;
}

export interface GamePlaytime {
  id: string;
  title: string;
  platform: string;
  cover_art: string | null;
  playtime_minutes: number;
  launch_count: number;
  last_played: string | null;
}

export interface PlatformUsage {
  platform: string;
  game_count: number;
  playtime_minutes: number;
}

export interface LibraryStats {
  total_games: number;
  total_playtime_minutes: number;
  total_launches: number;
  platform_count: number;
  favorite_count: number;
  most_played: GamePlaytime[];
  platform_usage: PlatformUsage[];
  recently_played: GamePlaytime[];
}

export interface LaunchResult {
  session_id: string;
  pid: number;
  emulator_id: string;
  emulator_name: string;
}

export interface GameQuery {
  profile_id: string;
  platform?: string | null;
  favorites_only?: boolean | null;
  search?: string | null;
  sort?: string | null;
  limit?: number | null;
}
