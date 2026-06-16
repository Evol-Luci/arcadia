use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;

/// How an emulator got onto the machine. Drives the update-ownership rule:
/// if Arcadia didn't install it, Arcadia never updates it — only informs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InstallSource {
    Pacman,
    Yay,
    Flatpak,
    AppImage,
    Manual,
    ArcadiaManaged,
}

impl InstallSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            InstallSource::Pacman => "Pacman",
            InstallSource::Yay => "Yay",
            InstallSource::Flatpak => "Flatpak",
            InstallSource::AppImage => "AppImage",
            InstallSource::Manual => "Manual",
            InstallSource::ArcadiaManaged => "ArcadiaManaged",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "Pacman" => InstallSource::Pacman,
            "Yay" => InstallSource::Yay,
            "Flatpak" => InstallSource::Flatpak,
            "AppImage" => InstallSource::AppImage,
            "ArcadiaManaged" => InstallSource::ArcadiaManaged,
            _ => InstallSource::Manual,
        }
    }

    /// Who owns updates. Only ArcadiaManaged is owned by us.
    pub fn owner(&self) -> &'static str {
        match self {
            InstallSource::ArcadiaManaged => "arcadia",
            _ => "external",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Platform {
    pub id: String,
    pub name: String,
    pub manufacturer: Option<String>,
    pub generation: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Emulator {
    pub id: String,
    pub name: String,
    pub version: Option<String>,
    pub install_source: String,
    pub owner: String,
    pub executable_path: String,
    pub adapter_id: String,
    pub supports_savestates: bool,
    pub supports_saves: bool,
    pub supports_achievements: bool,
    pub supports_screenshots: bool,
    pub detected_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Game {
    pub id: String,
    pub profile_id: String,
    pub title: String,
    pub sort_title: String,
    pub platform: String,
    pub rom_path: String,
    pub file_size: i64,
    pub emulator_id: Option<String>,

    pub cover_art: Option<String>,
    pub background_art: Option<String>,
    pub description: Option<String>,
    pub genre: Option<String>,
    pub developer: Option<String>,
    pub publisher: Option<String>,
    pub release_date: Option<String>,

    pub playtime_minutes: i64,
    pub launch_count: i64,
    pub favorite: bool,
    pub last_played: Option<DateTime<Utc>>,

    pub added_at: DateTime<Utc>,
}

/// Per-game launch overrides injected into the emulator process at launch.
/// Empty/absent fields mean "no override": `extra_args` and `env_vars` are
/// additive on top of whatever the adapter already passes, and `core_override`
/// only affects the RetroArch adapter. Persisted in `game_settings`, keyed by
/// game id; a game with no special needs simply has no row (see
/// [`Engine::get_game_settings`], which returns the empty default in that case).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GameSettings {
    pub game_id: String,
    pub extra_args: Vec<String>,
    pub env_vars: std::collections::BTreeMap<String, String>,
    pub core_override: Option<String>,
}

/// One physical disc image belonging to a multi-disc [`Game`]. Disc-based
/// titles spread across several images are collapsed into a single game; this
/// records each image, ordered by `disc_number` (1-based).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GameDisc {
    pub game_id: String,
    pub disc_number: i64,
    pub rom_path: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlaySession {
    pub id: String,
    pub game_id: String,
    pub emulator_id: Option<String>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub duration_minutes: i64,
}

/// Emitted when a play session ends (the emulator process exits) and playtime
/// has been attributed to the game. The Tauri layer forwards this to the
/// webview so the UI can refresh stats on exit instead of polling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionEnded {
    pub session_id: String,
    pub game_id: String,
    pub duration_minutes: i64,
}

/// Progress for a long-running background job (library scan, box-art fetch).
/// The engine emits these as work proceeds; the Tauri layer forwards them to the
/// webview so the sidebar can show live progress instead of a frozen spinner.
/// `total == 0` means the work is indeterminate (e.g. a filesystem walk whose
/// size isn't known up front); `done == total && total > 0` (or `finished`)
/// means the phase is complete.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressEvent {
    pub profile_id: String,
    /// Machine-readable phase: "scan" | "artwork" | "metadata".
    pub kind: String,
    /// Human-readable label for the sidebar, e.g. "Fetching box art".
    pub label: String,
    pub done: usize,
    pub total: usize,
    /// True on the terminal event for this job, so the UI can clear the bar.
    pub finished: bool,
}

/// A point-in-time backup of a game's save data. `kind` is "manual" for a
/// user-requested backup or "pre-restore" for the automatic snapshot taken
/// before a restore overwrites the live saves (Risk-5: never lose data).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct SaveBackup {
    pub id: String,
    pub game_id: String,
    pub kind: String,
    pub path: String,
    pub file_count: i64,
    pub byte_size: i64,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// An indexed screenshot: a copy of an image the emulator wrote, stored in the
/// cache so the webview can render it. `source_path` is the emulator's original.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Screenshot {
    pub id: String,
    pub game_id: String,
    pub path: String,
    pub source_path: String,
    pub byte_size: i64,
    pub captured_at: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// One RetroAchievements achievement plus the user's unlock state, cached
/// locally so the Achievement Hub renders offline.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Achievement {
    pub id: String,
    pub game_id: String,
    pub ra_id: i64,
    pub title: String,
    pub description: Option<String>,
    pub points: i64,
    pub badge_url: Option<String>,
    pub unlocked: bool,
    pub unlocked_at: Option<String>,
    pub display_order: i64,
}

/// Which RetroAchievements game a local game is linked to.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RaLink {
    pub game_id: String,
    pub ra_game_id: i64,
    pub title: Option<String>,
    pub icon_url: Option<String>,
    pub linked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Collection {
    pub id: String,
    pub profile_id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

/// A collection plus its current game count, for grid rendering without an
/// N+1 query per card.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct CollectionSummary {
    pub id: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub game_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct RomSource {
    pub id: String,
    pub profile_id: String,
    pub path: String,
    pub added_at: DateTime<Utc>,
}

/// Aggregated dashboard numbers for the Statistics view ("Year in Review,
/// always available").
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LibraryStats {
    pub total_games: i64,
    pub total_playtime_minutes: i64,
    pub total_launches: i64,
    pub platform_count: i64,
    pub favorite_count: i64,
    pub most_played: Vec<GamePlaytime>,
    pub platform_usage: Vec<PlatformUsage>,
    pub recently_played: Vec<GamePlaytime>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct GamePlaytime {
    pub id: String,
    pub title: String,
    pub platform: String,
    pub cover_art: Option<String>,
    pub playtime_minutes: i64,
    pub launch_count: i64,
    pub last_played: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct PlatformUsage {
    pub platform: String,
    pub game_count: i64,
    pub playtime_minutes: i64,
}
