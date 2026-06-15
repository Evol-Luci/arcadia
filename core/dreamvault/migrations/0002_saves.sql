-- Arcadia / DreamVault — Save Management (v0.5)
-- Tracks backups of a game's save data. Risk-5 safety: a restore always
-- snapshots the current live saves first (kind = 'pre-restore'), so an
-- overwrite can never lose data — last-writer-wins is structurally impossible.

PRAGMA foreign_keys = ON;

CREATE TABLE save_backups (
    id          TEXT PRIMARY KEY NOT NULL,
    game_id     TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,            -- 'manual' | 'pre-restore'
    path        TEXT NOT NULL,            -- backup directory (holds files/ + manifest.json)
    file_count  INTEGER NOT NULL DEFAULT 0,
    byte_size   INTEGER NOT NULL DEFAULT 0,
    note        TEXT,
    created_at  TEXT NOT NULL
);

CREATE INDEX idx_save_backups_game ON save_backups(game_id, created_at DESC);
