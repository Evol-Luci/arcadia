-- Multi-disc games. A disc-based title spread across several images
-- (e.g. "Final Fantasy VII (Disc 1/2/3)") is collapsed into ONE `games` row so
-- the library shows a single card and saves/stats/achievements aggregate under
-- one game_id. Each physical image is recorded here, ordered by disc number.
--
-- The owning `games` row's `rom_path` points at disc 1 (so single-disc launch
-- paths and on-disk art lookup keep working); the full ordered set is materialised
-- into an .m3u playlist at launch so the emulator's own disc-swap menu can change
-- discs mid-game. A single-disc game simply has no rows here.
CREATE TABLE game_discs (
    game_id     TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
    disc_number INTEGER NOT NULL,
    rom_path    TEXT NOT NULL,
    label       TEXT,
    PRIMARY KEY (game_id, disc_number)
);

CREATE INDEX idx_game_discs_game ON game_discs(game_id);
