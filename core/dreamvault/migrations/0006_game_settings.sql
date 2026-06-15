-- Per-game launch settings. Some titles only render or run correctly with
-- specific emulator flags, environment variables (e.g. GPU offload, vsync), or
-- a particular libretro core. We persist those per game and inject them at
-- launch via LaunchContext. A game with no special needs simply has no row.
--
-- `extra_args` is a JSON array of strings; `env_vars` is a JSON object of
-- key→value pairs; both default to empty so a row can carry just a core
-- override. `core_override` is a libretro core base name (no `_libretro.so`
-- suffix) and only affects the RetroArch adapter — standalone adapters ignore
-- it. Deleting the game cascades the row away.
CREATE TABLE game_settings (
    game_id       TEXT PRIMARY KEY REFERENCES games(id) ON DELETE CASCADE,
    extra_args    TEXT NOT NULL DEFAULT '[]',
    env_vars      TEXT NOT NULL DEFAULT '{}',
    core_override TEXT,
    updated_at    TEXT NOT NULL
);
