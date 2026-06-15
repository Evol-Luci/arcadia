-- Learned per-game savestate keys.
--
-- Most emulators name their per-game files by something we can derive from the
-- ROM ahead of time: the filename stem (Snes9x), or a disc serial / game id we
-- read out of the image (PCSX2, DuckStation, Dolphin). A few don't — notably
-- Mupen64Plus, which names states and battery saves `<GoodName>-<CRC>.stN`,
-- where both fields come from its bundled ROM database via an MD5 lookup, not
-- from the ROM header. We can't reconstruct that without reimplementing the
-- emulator's database match, which the design forbids.
--
-- Instead we *learn* it: after a play session that Arcadia launched, the adapter
-- inspects which of its own files the emulator wrote during the session window
-- and reports the key they're named by. We persist it here so later savestate
-- discovery can match the files. `state_key` is the literal base the emulator
-- chose (e.g. `Mario Kart 64 (U) [!]-3A67D998`); latest session wins, so the row
-- self-heals if the emulator ever renames. Deleting the game cascades the row.
CREATE TABLE state_keys (
    game_id    TEXT PRIMARY KEY REFERENCES games(id) ON DELETE CASCADE,
    adapter_id TEXT NOT NULL,
    state_key  TEXT NOT NULL,
    learned_at TEXT NOT NULL
);
