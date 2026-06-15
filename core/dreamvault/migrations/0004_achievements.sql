-- Achievement Hub (RetroAchievements). We cache a game's achievement set and
-- the user's unlock state locally so the UI works offline and we stay within
-- RA's rate limits. `ra_links` records which RA game a local game maps to.
CREATE TABLE IF NOT EXISTS ra_links (
    game_id     TEXT PRIMARY KEY REFERENCES games(id) ON DELETE CASCADE,
    ra_game_id  INTEGER NOT NULL,
    title       TEXT,
    icon_url    TEXT,
    linked_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS achievements (
    id          TEXT PRIMARY KEY,
    game_id     TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    ra_id       INTEGER NOT NULL,
    title       TEXT NOT NULL,
    description TEXT,
    points      INTEGER NOT NULL DEFAULT 0,
    badge_url   TEXT,
    unlocked    INTEGER NOT NULL DEFAULT 0,
    unlocked_at TEXT,
    display_order INTEGER NOT NULL DEFAULT 0,
    UNIQUE (game_id, ra_id)
);

CREATE INDEX IF NOT EXISTS idx_achievements_game ON achievements (game_id);
