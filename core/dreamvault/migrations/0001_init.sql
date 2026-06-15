-- Arcadia / DreamVault core schema (v0.1)
-- SQLite. All timestamps are RFC3339 TEXT (UTC). IDs are UUID TEXT.

PRAGMA foreign_keys = ON;

-- Workspace profiles scope the library (Retro / Handheld / Deck / Desktop, ...).
CREATE TABLE profiles (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL UNIQUE,
    is_default  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);

-- Canonical platforms (snes, n64, ps2, ...). Seeded by the library engine.
CREATE TABLE platforms (
    id            TEXT PRIMARY KEY NOT NULL,  -- slug, e.g. "snes"
    name          TEXT NOT NULL,              -- display, e.g. "Super Nintendo"
    manufacturer  TEXT,
    generation    INTEGER
);

-- Installed emulators discovered on this machine.
CREATE TABLE emulators (
    id                    TEXT PRIMARY KEY NOT NULL,
    name                  TEXT NOT NULL,
    version               TEXT,
    install_source        TEXT NOT NULL,      -- Pacman|Yay|Flatpak|AppImage|Manual|ArcadiaManaged
    owner                 TEXT NOT NULL,      -- "external" | "arcadia"
    executable_path       TEXT NOT NULL,
    adapter_id            TEXT NOT NULL,      -- which adapter drives this emulator
    supports_savestates   INTEGER NOT NULL DEFAULT 0,
    supports_saves        INTEGER NOT NULL DEFAULT 0,
    supports_achievements INTEGER NOT NULL DEFAULT 0,
    supports_screenshots  INTEGER NOT NULL DEFAULT 0,
    detected_at           TEXT NOT NULL,
    UNIQUE(adapter_id, executable_path)
);

-- ROM library. Most rows belong to a profile.
CREATE TABLE games (
    id               TEXT PRIMARY KEY NOT NULL,
    profile_id       TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    title            TEXT NOT NULL,
    sort_title       TEXT NOT NULL,
    platform         TEXT NOT NULL REFERENCES platforms(id),
    rom_path         TEXT NOT NULL,
    file_size        INTEGER NOT NULL DEFAULT 0,

    -- preferred emulator override; NULL = let the engine pick by platform
    emulator_id      TEXT REFERENCES emulators(id) ON DELETE SET NULL,

    -- metadata (filled by the metadata engine; nullable)
    cover_art        TEXT,
    background_art   TEXT,
    description      TEXT,
    genre            TEXT,
    developer        TEXT,
    publisher        TEXT,
    release_date     TEXT,

    -- statistics (denormalised for fast list rendering)
    playtime_minutes INTEGER NOT NULL DEFAULT 0,
    launch_count     INTEGER NOT NULL DEFAULT 0,
    favorite         INTEGER NOT NULL DEFAULT 0,
    last_played      TEXT,

    added_at         TEXT NOT NULL,
    UNIQUE(profile_id, rom_path)
);

-- Hot query paths: "10,000+ games without slowdown" comes from these indexes.
CREATE INDEX idx_games_profile      ON games(profile_id);
CREATE INDEX idx_games_sort_title   ON games(profile_id, sort_title);
CREATE INDEX idx_games_platform     ON games(profile_id, platform);
CREATE INDEX idx_games_last_played  ON games(profile_id, last_played);
CREATE INDEX idx_games_favorite     ON games(profile_id, favorite);

-- One row per play session. Playtime is aggregated onto games.playtime_minutes.
CREATE TABLE play_sessions (
    id              TEXT PRIMARY KEY NOT NULL,
    game_id         TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    emulator_id     TEXT REFERENCES emulators(id) ON DELETE SET NULL,
    started_at      TEXT NOT NULL,
    ended_at        TEXT,
    duration_minutes INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_sessions_game    ON play_sessions(game_id);
CREATE INDEX idx_sessions_started ON play_sessions(started_at);

-- Collections (manual groupings). v0.5 surfaces these in the UI; schema lands now.
CREATE TABLE collections (
    id          TEXT PRIMARY KEY NOT NULL,
    profile_id  TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    created_at  TEXT NOT NULL,
    UNIQUE(profile_id, name)
);

CREATE TABLE collection_games (
    collection_id TEXT NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    game_id       TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    PRIMARY KEY (collection_id, game_id)
);

-- ROM folders the user pointed us at, scoped per profile. Read in place; never relocated.
CREATE TABLE rom_sources (
    id          TEXT PRIMARY KEY NOT NULL,
    profile_id  TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    path        TEXT NOT NULL,
    added_at    TEXT NOT NULL,
    UNIQUE(profile_id, path)
);
