//! Savestate discovery (v0.5, "Recall State").
//!
//! Distinct from [`crate::saves`], which treats a game's save data as opaque
//! blobs to back up. Here we *recognise* RetroArch savestates as first-class,
//! structured slots — slot number, last-modified time, and the thumbnail
//! RetroArch writes next to each state — so the UI can present a slot grid the
//! user can see at a glance.
//!
//! Per the design rule, this only ever *reads* the emulator's own files: it
//! parses filenames (and, for RetroArch, pairs each `.state` with its sibling
//! `.png`). It never parses or rewrites the state format. Each adapter owns its
//! discovery (see `EmulatorAdapter::scan_save_states`); the engine resolves the
//! game's adapter plus any disc key it needs and dispatches. This module holds
//! the shared `SaveState` shape and two reusable discovery primitives:
//! [`discover_save_states`] (RetroArch, with thumbnails) and [`discover_slots`]
//! (the generic, single-key, thumbnail-less scan the standalone adapters use).

use crate::error::{EngineError, Result};
use crate::saves::flatpak_app_id_of;
use crate::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

/// One savestate slot found on disk. `slot` is the parsed slot index: `-1` for
/// RetroArch's rolling auto-save, `0` for the base `.state`, and `N` for a
/// numbered `.stateN`. `thumbnail` is the sibling preview PNG when RetroArch was
/// configured to write one (`savestate_thumbnail_enable`), giving the UI a free
/// per-slot screenshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaveState {
    pub slot: i64,
    pub label: String,
    pub path: String,
    pub thumbnail: Option<String>,
    pub byte_size: i64,
    pub modified_at: Option<String>,
}

/// Map a savestate filename's suffix (everything after `<stem>.state`) to a slot
/// index, or `None` if it isn't a recognised state file. RetroArch names the
/// base slot `<stem>.state`, numbered slots `<stem>.state1`, `.state2`, …, and
/// the auto-save `<stem>.state.auto`.
fn slot_from_suffix(suffix: &str) -> Option<i64> {
    match suffix {
        "" => Some(0),
        ".auto" => Some(-1),
        _ => suffix.parse::<i64>().ok().filter(|n| *n >= 0),
    }
}

/// Human label for a slot index.
fn slot_label(slot: i64) -> String {
    match slot {
        -1 => "Auto-save".to_string(),
        n => format!("Slot {n}"),
    }
}

/// Size and last-modified (RFC 3339) for a state file, best-effort.
fn file_meta(path: &Path) -> (i64, Option<String>) {
    match std::fs::metadata(path) {
        Ok(m) => {
            let modified = m
                .modified()
                .ok()
                .map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            (m.len() as i64, modified)
        }
        Err(_) => (0, None),
    }
}

/// Discover every savestate for the ROM whose basename (without extension) is
/// `stem`, inside `states_dir`. Pure over the filesystem and order-stable
/// (sorted by slot, auto-save first) so it is fully tempdir-testable. Each
/// `<stem>.stateN` is paired with its `<stem>.stateN.png` thumbnail if present.
pub(crate) fn discover_save_states(states_dir: &Path, stem: &str) -> Vec<SaveState> {
    let prefix = format!("{stem}.state");
    // state filename -> its .png thumbnail path.
    let mut thumbs: HashMap<String, String> = HashMap::new();
    // (slot, state path, state filename) collected before we resolve thumbnails,
    // since a thumbnail may be read before or after its state in dir order.
    let mut states: Vec<(i64, std::path::PathBuf, String)> = Vec::new();

    let Ok(rd) = std::fs::read_dir(states_dir) else {
        return Vec::new();
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with(&prefix) {
            continue;
        }
        // A `.png` sibling is the thumbnail for the state file it shadows; key it
        // by that state's filename so we can attach it below.
        if let Some(state_name) = name.strip_suffix(".png") {
            thumbs.insert(state_name.to_string(), path.to_string_lossy().to_string());
            continue;
        }
        if let Some(slot) = slot_from_suffix(&name[prefix.len()..]) {
            states.push((slot, path.clone(), name.to_string()));
        }
    }

    let mut out: Vec<SaveState> = states
        .into_iter()
        .map(|(slot, path, name)| {
            let (byte_size, modified_at) = file_meta(&path);
            SaveState {
                slot,
                label: slot_label(slot),
                thumbnail: thumbs.get(&name).cloned(),
                path: path.to_string_lossy().to_string(),
                byte_size,
                modified_at,
            }
        })
        .collect();
    out.sort_by_key(|s| s.slot);
    out
}

/// Generic slot discovery for adapters that key states by a single string (ROM
/// stem, disc serial, or game id) with no sibling thumbnails. `classify` maps a
/// directory entry's filename to a slot index, or `None` to skip it — each
/// emulator's naming quirk lives entirely in that closure. Pure over the
/// filesystem and order-stable (sorted by slot), so every adapter's rule is
/// unit-testable in a tempdir. Returns empty if `dir` is missing/unreadable.
pub(crate) fn discover_slots(dir: &Path, classify: impl Fn(&str) -> Option<i64>) -> Vec<SaveState> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<SaveState> = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(slot) = classify(name) else {
            continue;
        };
        let (byte_size, modified_at) = file_meta(&path);
        out.push(SaveState {
            slot,
            label: slot_label(slot),
            thumbnail: None,
            path: path.to_string_lossy().to_string(),
            byte_size,
            modified_at,
        });
    }
    out.sort_by_key(|s| s.slot);
    out
}

/// Learn an emulator's per-game file key by observing what it wrote this
/// session. `classify` maps a directory entry's filename to the key it's built
/// on (the base before the slot/extension), or `None` to ignore it. Only files
/// modified at or after `since` — the session start — are considered, so a
/// pre-existing file from another game is never misattributed; of those, the
/// most-recently-written one wins (the user's latest save). Pure over the
/// filesystem so it's unit-testable in a tempdir. `None` if `dir` is
/// missing/unreadable or nothing matching was written this session.
pub(crate) fn learn_key_in(
    dir: &Path,
    classify: impl Fn(&str) -> Option<String>,
    since: std::time::SystemTime,
) -> Option<String> {
    let rd = std::fs::read_dir(dir).ok()?;
    let mut best: Option<(std::time::SystemTime, String)> = None;
    for entry in rd.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(key) = classify(name) else {
            continue;
        };
        let Ok(modified) = entry.metadata().and_then(|m| m.modified()) else {
            continue;
        };
        if modified < since {
            continue;
        }
        if best.as_ref().is_none_or(|(t, _)| modified >= *t) {
            best = Some((modified, key));
        }
    }
    best.map(|(_, key)| key)
}

/// Resolve the disc serial / game id that serial-keyed emulators use to name
/// savestates. A few platforms expose a cheap, reliable on-disk identifier we can
/// read today: PS1/PS2 (`SLUS-20946` / `SCUS-94163`, parsed from `SYSTEM.CNF` in
/// cooked `.iso`, raw `.bin`, or a `.cue`'s first track) and GameCube/Wii (the
/// 6-char header game id). PS1 `.chd` images stay unreadable until a CHD reader
/// lands, so DuckStation discovery is empty for those. Returns `None` otherwise.
fn resolve_disc_key(platform: &str, rom_path: &str) -> Option<String> {
    let path = Path::new(rom_path);
    match platform {
        "ps1" | "ps2" => crate::iso9660::disc_serial(path),
        "gamecube" | "wii" => crate::iso9660::nintendo_game_id(path),
        _ => None,
    }
}

impl Engine {
    /// List a game's savestates as structured slots, best-effort. Resolves the
    /// game's emulator → adapter, builds a scan context (Flatpak redirect + any
    /// disc key the emulator names states by), and dispatches to the adapter's
    /// own discovery. Returns empty for emulators that don't index states, when
    /// no adapter is resolved, or when the game has no states yet.
    pub async fn list_save_states(&self, game_id: &str) -> Result<Vec<SaveState>> {
        let game = self
            .get_game(game_id)
            .await?
            .ok_or_else(|| EngineError::NotFound(format!("game {game_id}")))?;
        let emulator = match &game.emulator_id {
            Some(id) => self.get_emulator(id).await?,
            None => self.emulator_for_platform(&game.platform).await?,
        };
        let Some(emulator) = emulator else {
            return Ok(Vec::new());
        };
        let Some(adapter) = self.adapters.get(&emulator.adapter_id) else {
            return Ok(Vec::new());
        };
        // Prefer a key learned from a prior session (the only source for
        // database-keyed emulators like Mupen64Plus); otherwise resolve one from
        // the ROM (disc serial / game id). One of these is `None` for most
        // games, which simply yields no savestates.
        // The disc key is always derived from the *original* ROM (its serial, or a
        // key learned from a prior session) — unchanged behaviour for DiscKey
        // emulators (PCSX2, DuckStation, Dolphin, Mupen).
        let disc_key = match self.learned_state_key(game_id).await? {
            Some(key) => Some(key),
            None => resolve_disc_key(&game.platform, &game.rom_path),
        };
        // For stem-/RomDir-keyed emulators, states for an *archived* ROM don't live
        // beside the `.zip`/`.7z` on the source — the emulator ran the file we
        // extracted into the cache, so it wrote states there, named after the
        // extracted file's inner stem (which can differ from the archive's name).
        // Point discovery at that already-extracted file when one exists; otherwise
        // (raw ROM, or never launched) keep the original path. This doesn't affect
        // DiscKey emulators: none key their state dir off rom_path.
        let scan_rom = crate::archive::existing_extraction(std::path::Path::new(&game.rom_path))
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| game.rom_path.clone());
        let ctx = crate::adapters::ScanContext {
            rom_path: scan_rom,
            platform: game.platform.clone(),
            flatpak_app_id: flatpak_app_id_of(&emulator),
            disc_key,
        };
        Ok(adapter.scan_save_states(&ctx).await.unwrap_or_default())
    }

    /// Whether booting straight into a savestate is possible for this game — i.e.
    /// the emulator that would launch it exposes a startup-load flag. Resolves the
    /// emulator exactly as launch does (explicit override → platform default), so
    /// the answer matches what `launch_game_into_state` would actually do. Drives
    /// the Recall State grid: `true` → clickable cards; `false` → the discovered
    /// states render read-only (e.g. Snes9x, which has no CLI savestate-load).
    /// Returns `false` rather than erroring when no emulator/adapter resolves.
    pub async fn game_supports_launch_state(&self, game_id: &str) -> Result<bool> {
        let Some(game) = self.get_game(game_id).await? else {
            return Ok(false);
        };
        let emulator = match &game.emulator_id {
            Some(id) => self.get_emulator(id).await?,
            None => self.emulator_for_platform(&game.platform).await?,
        };
        let Some(emulator) = emulator else {
            return Ok(false);
        };
        let Some(adapter) = self.adapters.get(&emulator.adapter_id) else {
            return Ok(false);
        };
        Ok(adapter.supports_launch_state())
    }

    /// The savestate key most recently learned for this game by observing the
    /// emulator's own writes (see `EmulatorAdapter::learn_state_key`). `None`
    /// when nothing has been learned yet — e.g. before the first save+exit of a
    /// database-keyed emulator like Mupen64Plus.
    pub(crate) async fn learned_state_key(&self, game_id: &str) -> Result<Option<String>> {
        let row: Option<(String,)> =
            sqlx::query_as("SELECT state_key FROM state_keys WHERE game_id = ?")
                .bind(game_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(k,)| k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_and_classifies_slots_with_thumbnails() {
        let tmp = tempfile::tempdir().unwrap();
        let states = tmp.path().join("states");
        std::fs::create_dir_all(&states).unwrap();

        // Base slot, two numbered slots, the rolling auto-save.
        std::fs::write(states.join("Super Mario 64.state"), b"s0").unwrap();
        std::fs::write(states.join("Super Mario 64.state1"), b"s1").unwrap();
        std::fs::write(states.join("Super Mario 64.state2"), b"s2").unwrap();
        std::fs::write(states.join("Super Mario 64.state.auto"), b"auto").unwrap();
        // Thumbnails for two of them.
        std::fs::write(states.join("Super Mario 64.state.png"), b"png0").unwrap();
        std::fs::write(states.join("Super Mario 64.state1.png"), b"png1").unwrap();
        // A different game's state must NOT be picked up.
        std::fs::write(states.join("Zelda.state"), b"other").unwrap();
        // A non-state file with the prefix shape but unparseable suffix is skipped.
        std::fs::write(states.join("Super Mario 64.state.bak"), b"bak").unwrap();

        let found = discover_save_states(&states, "Super Mario 64");
        let slots: Vec<i64> = found.iter().map(|s| s.slot).collect();
        // Auto-save (-1) sorts first, then 0, 1, 2. The .bak is excluded.
        assert_eq!(slots, vec![-1, 0, 1, 2]);

        let auto = &found[0];
        assert_eq!(auto.label, "Auto-save");
        assert!(auto.thumbnail.is_none());

        let base = &found[1];
        assert_eq!(base.label, "Slot 0");
        assert!(base.thumbnail.as_ref().unwrap().ends_with("Super Mario 64.state.png"));

        let one = &found[2];
        assert!(one.thumbnail.as_ref().unwrap().ends_with("Super Mario 64.state1.png"));

        let two = &found[3];
        assert!(two.thumbnail.is_none());
    }

    #[test]
    fn empty_or_missing_dir_yields_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        // Directory that doesn't exist.
        assert!(discover_save_states(&tmp.path().join("nope"), "Game").is_empty());
        // Existing but empty.
        let states = tmp.path().join("states");
        std::fs::create_dir_all(&states).unwrap();
        assert!(discover_save_states(&states, "Game").is_empty());
    }

    fn set_mtime(path: &Path, t: std::time::SystemTime) {
        let f = std::fs::File::options().write(true).open(path).unwrap();
        f.set_modified(t).unwrap();
    }

    #[test]
    fn learns_key_from_session_writes_and_respects_the_window() {
        use std::time::{Duration, SystemTime};
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let base = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);

        // An older save (a prior game's file) and a newer one written "this
        // session". `classify` strips `.st0` to recover the base name.
        std::fs::write(dir.join("Old-1111.st0"), b"old").unwrap();
        set_mtime(&dir.join("Old-1111.st0"), base);
        std::fs::write(dir.join("New-2222.st0"), b"new").unwrap();
        set_mtime(&dir.join("New-2222.st0"), base + Duration::from_secs(10));
        // A non-state sibling is ignored entirely.
        std::fs::write(dir.join("notes.txt"), b"x").unwrap();
        set_mtime(&dir.join("notes.txt"), base + Duration::from_secs(10));

        let classify = |name: &str| name.strip_suffix(".st0").map(str::to_string);

        // Session started after the old file but before the new one: only the
        // new write is attributed.
        assert_eq!(
            learn_key_in(dir, classify, base + Duration::from_secs(5)).as_deref(),
            Some("New-2222")
        );
        // Session started before both: the most recently written wins.
        assert_eq!(
            learn_key_in(dir, classify, base - Duration::from_secs(5)).as_deref(),
            Some("New-2222")
        );
        // Session started after both: nothing was written this session.
        assert!(learn_key_in(dir, classify, base + Duration::from_secs(60)).is_none());
        // Missing directory yields nothing.
        assert!(learn_key_in(&dir.join("nope"), classify, base).is_none());
    }

    #[test]
    fn thumbnail_only_does_not_create_a_phantom_slot() {
        // A stray .png with no matching state file must not appear as a slot.
        let tmp = tempfile::tempdir().unwrap();
        let states = tmp.path().join("states");
        std::fs::create_dir_all(&states).unwrap();
        std::fs::write(states.join("Game.state3.png"), b"png").unwrap();
        assert!(discover_save_states(&states, "Game").is_empty());
    }
}
