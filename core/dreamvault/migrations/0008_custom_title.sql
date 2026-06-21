-- User-supplied display name override for a game. NULL/empty means "use the
-- scanned `title`". Survives rescans (see insert_game's ON CONFLICT guard).
ALTER TABLE games ADD COLUMN custom_title TEXT;
