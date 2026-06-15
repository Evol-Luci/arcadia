-- Screenshots (v0.5). Indexed captures the emulator already wrote to disk; we
-- import a copy into Arcadia's cache (so the webview can load them through the
-- asset protocol) and remember the source so re-scans are idempotent.
CREATE TABLE IF NOT EXISTS screenshots (
    id          TEXT PRIMARY KEY,
    game_id     TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    path        TEXT NOT NULL,        -- imported copy under the cache dir
    source_path TEXT NOT NULL,        -- where the emulator wrote it
    byte_size   INTEGER NOT NULL DEFAULT 0,
    captured_at TEXT,                 -- file mtime, RFC3339, best-effort
    created_at  TEXT NOT NULL,
    UNIQUE (game_id, source_path)
);

CREATE INDEX IF NOT EXISTS idx_screenshots_game ON screenshots (game_id);
