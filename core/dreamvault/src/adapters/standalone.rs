//! Generic adapter for standalone emulators. Each instance is parameterised by
//! a `Spec` describing the binary names, Flatpak id, capabilities, and how to
//! turn a ROM path into command-line arguments. Ships SNES9x, mGBA,
//! Mupen64Plus, Dolphin, PCSX2, PPSSPP, RPCS3, DuckStation, melonDS, Flycast,
//! MAME, xemu, Cemu, Vita3K, Ryujinx, and Lime3DS, plus a tail of launch-only
//! single-system cores (Mesen, Stella, BlastEm, openMSX, Gearcoleco, BigPEmu,
//! VICE, Atari800). Mednafen sits between: launch-only for boot, but its
//! savestates are discovered read-only (`~/.mednafen/mcs/<stem>.<md5>.mc<slot>`).

use super::detect::{self};
use super::{
    AdapterDescriptor, Capabilities, EmulatorAdapter, EmulatorInstall, LaunchContext, LaunchHandle,
    ScanContext,
};
use crate::error::AdapterError;
use crate::hotkeys::{
    ini_section_pairs, normalize_key, overlay_defaults, sdl_keysym_name, sdl_scancode_name,
    EmulatorHotkey, HotkeyAction,
};
use crate::save_states::{discover_slots, SaveState};
use async_trait::async_trait;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where an emulator keeps its savestate files, relative to a base it shares
/// with its other config/data. The Flatpak redirect (`~/.var/app/<id>/{config,
/// data}`) is applied by [`SaveStateScheme::dir`].
enum StateLocation {
    /// Under the XDG config root (`~/.config`, or `…/config` in a Flatpak).
    Config(&'static str),
    /// Under the XDG data root (`~/.local/share`, or `…/data` in a Flatpak).
    Data(&'static str),
    /// A per-game subdirectory under the XDG data root, named by the ROM stem:
    /// `<data>/<sub>/<stem>/` (BlastEm: `data/blastem/<rom name>/slot_N.state`).
    DataPerGame(&'static str),
    /// In the same directory as the ROM (Snes9x's default, no folder set).
    RomDir,
    /// Directly under `$HOME`, for emulators with their own dotdir base rather
    /// than an XDG root (Mednafen's `~/.mednafen`). Not Flatpak-redirected.
    Home(&'static str),
}

/// What string the emulator keys its savestate filenames by.
enum KeyKind {
    /// The ROM's basename without extension (Snes9x).
    Stem,
    /// The disc serial / game id the engine resolved into `ScanContext::disc_key`
    /// (PCSX2 PS2 serial, Dolphin GameCube/Wii game id) — or, for Mupen64Plus, a
    /// key *learned* from a prior session (see [`SaveStateScheme::learn`]) and
    /// threaded through the same field. Discovery yields nothing when the key is
    /// unresolved.
    DiscKey,
}

/// How one emulator stores savestates: where the files live, what they're keyed
/// by, and a parser turning a directory entry's filename into a slot index (or
/// `None` to skip it). Per the design rule, discovery only ever *reads* these
/// files — it classifies names, never the state payload.
struct SaveStateScheme {
    location: StateLocation,
    key: KeyKind,
    /// `(filename, resolved_key) -> slot`. `-1` is the auto/resume slot.
    parse: fn(&str, &str) -> Option<i64>,
    /// A function mapping one of the emulator's own filenames back to the key
    /// it's built on, so the engine can *learn* the key after a session by
    /// reading back what the emulator wrote. Two reasons an emulator needs this:
    ///   1. its key can't be derived from the ROM at all (Mupen64Plus names files
    ///      from a database `GoodName-CRC` we can't reconstruct); or
    ///   2. its key normally *is* derivable (a disc serial/id) but the ROM we hold
    ///      is unreadable as a disc — a `.zip`/`.7z` we launch from a temp
    ///      extraction, or a `.chd` — so reading back the emulator's writes is the
    ///      only way to recover it.
    /// A learned key takes precedence over `resolve_disc_key` (see
    /// `Engine::list_save_states`). `None` only for stem-keyed schemes, whose key
    /// is always available from the ROM's own filename.
    learn: Option<fn(&str) -> Option<String>>,
}

impl SaveStateScheme {
    /// Resolve the directory to scan, applying the Flatpak redirect for
    /// `Config`/`Data` locations. `RomDir` is the ROM's own folder regardless of
    /// install source.
    fn dir(&self, flatpak_app_id: Option<&str>, rom_path: &str) -> Option<PathBuf> {
        match &self.location {
            StateLocation::RomDir => Path::new(rom_path).parent().map(Path::to_path_buf),
            StateLocation::Config(sub) => Some(xdg_base(flatpak_app_id, true)?.join(sub)),
            StateLocation::Data(sub) => Some(xdg_base(flatpak_app_id, false)?.join(sub)),
            StateLocation::DataPerGame(sub) => {
                let stem = Path::new(rom_path).file_stem()?.to_str()?;
                Some(xdg_base(flatpak_app_id, false)?.join(sub).join(stem))
            }
            StateLocation::Home(sub) => {
                Some(PathBuf::from(std::env::var_os("HOME")?).join(sub))
            }
        }
    }
}

/// The XDG config (`config = true`) or data (`config = false`) base directory,
/// redirected into the Flatpak sandbox layout when `flatpak_app_id` is set.
fn xdg_base(flatpak_app_id: Option<&str>, config: bool) -> Option<PathBuf> {
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    Some(match (flatpak_app_id, config) {
        (Some(app), true) => home.join(".var/app").join(app).join("config"),
        (Some(app), false) => home.join(".var/app").join(app).join("data"),
        (None, true) => home.join(".config"),
        (None, false) => home.join(".local/share"),
    })
}

/// True when at least one HID controller is currently bound to the `xpadneo`
/// kernel driver — the specific setup whose SDL-HIDAPI joystick path delivers no
/// input. Bound devices show up as entries named `BUS:VID:PID.N` under the
/// driver's sysfs dir; the driver's own control files (`bind`, `unbind`,
/// `uevent`, `module`, …) never contain a colon, so that's the discriminator.
/// Absent sysfs (non-Linux, sandbox) reads as "not present" and the workaround
/// stays off.
pub(crate) fn xpadneo_controller_present() -> bool {
    const DRIVER_DIRS: &[&str] = &[
        "/sys/bus/hid/drivers/xpadneo",
        "/sys/bus/hid/drivers/hid-xpadneo",
    ];
    DRIVER_DIRS.iter().any(|dir| {
        std::fs::read_dir(dir)
            .map(|entries| {
                entries
                    .flatten()
                    .any(|e| e.file_name().to_string_lossy().contains(':'))
            })
            .unwrap_or(false)
    })
}

// ---- Per-emulator savestate filename parsers ------------------------------
// Each is pure (filename + resolved key -> slot) so it's unit-testable against
// real captured filenames without touching the filesystem.

/// PCSX2: `<SERIAL> (<CRC>).<slot>.p2s`, e.g. `SLUS-20415 (2999BCF9).resume.p2s`
/// (auto) or `… .00.p2s` … `.09.p2s`. The CRC isn't known ahead of time, so we
/// anchor on the serial and read the slot token after the final `.`.
fn parse_pcsx2(name: &str, serial: &str) -> Option<i64> {
    let rest = name.strip_suffix(".p2s")?;
    if !rest.starts_with(&format!("{serial} (")) {
        return None;
    }
    match rest.rsplit('.').next()? {
        "resume" => Some(-1),
        n => n.parse().ok(),
    }
}

/// DuckStation: `<SERIAL>_resume.sav` (auto) or `<SERIAL>_<n>.sav` (numbered).
fn parse_duckstation(name: &str, serial: &str) -> Option<i64> {
    let slot = name.strip_suffix(".sav")?.strip_prefix(&format!("{serial}_"))?;
    match slot {
        "resume" => Some(-1),
        n => n.parse().ok(),
    }
}

/// Snes9x: `<stem>.NNN`, a zero-padded three-digit slot (`000`..`009` by
/// default). The `.srm` battery save and other siblings are length-3-but-not or
/// not-all-digits and are skipped.
fn parse_snes9x(name: &str, stem: &str) -> Option<i64> {
    let suffix = name.strip_prefix(&format!("{stem}."))?;
    (suffix.len() == 3 && suffix.bytes().all(|b| b.is_ascii_digit()))
        .then(|| suffix.parse().ok())
        .flatten()
}

/// mGBA: `<stem>.ssN` savestate slots written beside the ROM by default (slot 1
/// through 9, plus the quick-save `.ss0`). The `.sav` battery save and `.png`
/// screenshot siblings don't match the `.ss<digits>` shape and are skipped.
fn parse_mgba(name: &str, stem: &str) -> Option<i64> {
    let slot = name.strip_prefix(&format!("{stem}.ss"))?;
    (!slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

/// Flycast: `<stem>.state` for the default slot 0, `<stem>_<N>.state` for slot
/// N (1..9). Confirmed from a real file (`Jet Grind Radio (USA).state`) plus the
/// binary's `_%d` + `.state` path format. The `.state.net` autosave/netplay
/// variant ends in `.net`, so it fails the `.state` suffix check and is skipped.
fn parse_flycast(name: &str, stem: &str) -> Option<i64> {
    let rest = name.strip_suffix(".state")?;
    if rest == stem {
        return Some(0);
    }
    let slot = rest.strip_prefix(&format!("{stem}_"))?;
    (!slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

/// melonDS: `<stem>.ml<slot>` written beside the ROM (when no Savestate path is
/// configured), e.g. `0028 - Kirby - Canvas Curse (USA).ml1` (slots 1..8). Keyed
/// by stem; because NDS dumps are usually zipped, the "ROM dir" discovery scans is
/// the *extraction cache* and the stem is the extracted file's inner name (see
/// `Engine::list_save_states`). The `.dsv` battery save doesn't match `.ml<N>`.
fn parse_melonds(name: &str, stem: &str) -> Option<i64> {
    let slot = name.strip_prefix(&format!("{stem}.ml"))?;
    (!slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

/// BlastEm: states live in a per-game folder `data/blastem/<rom name>/`, holding
/// `slot_<N>.state` (numbered slots, 0..9) and `quicksave.state` (the F5 quick
/// slot, mapped to the auto/resume slot `-1`). Confirmed from real files
/// (`slot_0.state`, `quicksave.state`). The folder is the discovery dir, so the
/// stem isn't needed to match within it.
fn parse_blastem(name: &str, _stem: &str) -> Option<i64> {
    let base = name.strip_suffix(".state")?;
    if base == "quicksave" {
        return Some(-1);
    }
    let slot = base.strip_prefix("slot_")?;
    (!slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

/// Mednafen: `<stem>.<md5>.mc<slot>` in `~/.mednafen/mcs/`, e.g.
/// `Hook (Europe) (Proto).7a07e3e59f51b620d8c51bef468d4df5.mc0` (slots 0..9).
/// The 32-hex ROM MD5 sits between the stem and the slot token, so — like
/// `parse_pcsx2` with its CRC — we anchor on the stem prefix and read the
/// `mc<digits>` slot from the final `.`-segment, ignoring the hash in between.
fn parse_mednafen(name: &str, stem: &str) -> Option<i64> {
    let rest = name.strip_prefix(&format!("{stem}."))?;
    let slot = rest.rsplit('.').next()?.strip_prefix("mc")?;
    (!slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

/// Mesen: `<stem>_<slot>.mss` in `~/.config/Mesen2/SaveStates/` (slots 1..10),
/// e.g. `Addams Family, The (USA)_1.mss`. Fixed config dir keyed by the ROM
/// stem, so it survives our zip→temp extraction (the extracted file keeps the
/// archive's basename). The non-numeric auto-save sibling is skipped.
fn parse_mesen(name: &str, stem: &str) -> Option<i64> {
    let slot = name.strip_suffix(".mss")?.strip_prefix(&format!("{stem}_"))?;
    (!slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

/// Dolphin: `<GAMEID>.s01`..`.s10` in `StateSaves/`, keyed by the 6-char disc id.
fn parse_dolphin(name: &str, game_id: &str) -> Option<i64> {
    let slot = name.strip_prefix(&format!("{game_id}.s"))?;
    (!slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

// ---- Learn-the-key classifiers for the serial/id-keyed emulators ----------
// These let discovery work even when we can't read the disc serial/id from the
// ROM ahead of time — most importantly for games launched from a `.zip`/`.7z`
// (we extract to a temp dir, so `resolve_disc_key` can't read the original
// archive) or `.chd`. After a session the engine reads back the emulator's own
// writes and learns the key, exactly like Mupen64Plus. Each is the inverse of
// the matching `parse_*`: given a filename the emulator wrote, recover the key
// it's built on. Pure (filename -> key) so they're unit-testable.

/// PCSX2: `<SERIAL> (<CRC>).<slot>.p2s` -> `<SERIAL>` (the part before ` (`).
fn learn_pcsx2_serial(name: &str) -> Option<String> {
    let rest = name.strip_suffix(".p2s")?; // skips the `.p2s.backup` sibling
    let serial = rest.split_once(" (")?.0;
    (!serial.is_empty()).then(|| serial.to_string())
}

/// DuckStation: `<SERIAL>_resume.sav` / `<SERIAL>_<n>.sav` -> `<SERIAL>` (before
/// the final `_`; PS1 serials never contain one).
fn learn_duckstation_serial(name: &str) -> Option<String> {
    let rest = name.strip_suffix(".sav")?;
    let serial = rest.rsplit_once('_')?.0;
    (!serial.is_empty()).then(|| serial.to_string())
}

/// Dolphin: `<GAMEID>.sNN` -> `<GAMEID>` (the id before the `.s<digits>` slot).
fn learn_dolphin_game_id(name: &str) -> Option<String> {
    let idx = name.rfind(".s")?;
    let (id, rest) = name.split_at(idx);
    let digits = &rest[".s".len()..];
    (!id.is_empty() && !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .then(|| id.to_string())
}

/// Mupen64Plus savestate filename extensions, longest-suffix listed where it
/// matters. States are `.stN`; the rest are battery saves that share the same
/// `<GoodName>-<CRC>` base, so observing any of them teaches us the base.
const MUPEN_SAVE_EXTS: &[&str] = &[".eep", ".mpk", ".sra", ".fla"];

/// Mupen64Plus: `<base>.stN` in the save dir, keyed by the learned base
/// `GoodName-CRC` (e.g. `Mario Kart 64 (U) [!]-3A67D998.st0`). The base is
/// supplied via `ScanContext::disc_key` from the learned-key store, not derived
/// from the ROM. Only `.stN` files are savestate slots; battery saves sharing
/// the base are not.
fn parse_mupen(name: &str, base: &str) -> Option<i64> {
    let slot = name.strip_prefix(&format!("{base}.st"))?;
    (!slot.is_empty() && slot.bytes().all(|b| b.is_ascii_digit()))
        .then(|| slot.parse().ok())
        .flatten()
}

/// Map one Mupen64Plus filename back to the `<GoodName>-<CRC>` base it's built
/// on, so a session's writes teach us the key. Recognises savestates
/// (`<base>.stN`) and the battery saves that share the base
/// ([`MUPEN_SAVE_EXTS`]); `None` for anything else in the directory.
fn learn_mupen_base(name: &str) -> Option<String> {
    // `<base>.stN` — strip the slot digits, then the `.st`.
    if let Some(idx) = name.rfind(".st") {
        let (base, slot) = name.split_at(idx);
        let digits = &slot[".st".len()..];
        if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
            return Some(base.to_string());
        }
    }
    // `<base>.eep` / `.mpk` / `.sra` / `.fla`.
    for ext in MUPEN_SAVE_EXTS {
        if let Some(base) = name.strip_suffix(ext) {
            return Some(base.to_string());
        }
    }
    None
}

/// Standalone emulators with a working "boot straight into a savestate" CLI
/// flag. [`StandaloneAdapter::apply_load_state`] must have a match arm for each,
/// and [`EmulatorAdapter::supports_launch_state`] reports exactly this set so
/// the UI only makes Recall State cards clickable when a click can be honoured.
/// Snes9x is deliberately absent: it has no CLI savestate-load (snes9x#437), and
/// no other standalone SNES emulator does either (verified Mesen 2.1.1, bsnes,
/// ares), so its cards render read-only rather than silently cold-booting.
const LAUNCH_STATE_IDS: &[&str] =
    &["mupen64plus", "pcsx2", "duckstation", "dolphin", "mgba"];

/// Where a "boot into this state" flag sits relative to the positional ROM.
/// Not uniform across emulators: most take options *before* the ROM, but
/// Dolphin's launch arg is `-e <rom>` (which consumes the next token), so its
/// state flag must come *after* the ROM.
enum LoadStatePlacement {
    /// Insert immediately before the final (ROM) arg — Mupen, PCSX2, DuckStation.
    BeforeRom,
    /// Append after the ROM — Dolphin.
    AfterRom,
}

// ---- Hotkey discovery -----------------------------------------------------
// Read-only: each scheme points at the emulator's own config file, parses its
// `action -> key` bindings, and overlays them on a documented defaults table so
// the panel is useful even when the user never rebound anything.

/// Which root the hotkey config path is relative to. Most emulators live under
/// the XDG config root (Flatpak-redirected); Mednafen keeps its config in a plain
/// `~/.mednafen` dotdir, so it resolves against `$HOME` directly.
enum HotkeyBase {
    /// `~/.config` (or the Flatpak `~/.var/app/<id>/config` redirect).
    XdgConfig,
    /// `$HOME` directly (e.g. Mednafen's `~/.mednafen/mednafen.cfg`).
    Home,
}

/// How one emulator stores its keyboard hotkeys: the base root + relative config
/// path, the documented defaults to fall back on, and a pure parser turning the
/// file's text into a canonical `action -> binding` map.
struct HotkeyScheme {
    base: HotkeyBase,
    /// Config file path relative to `base`.
    rel_path: &'static str,
    defaults: &'static [(HotkeyAction, &'static str)],
    parse: fn(&str) -> HashMap<HotkeyAction, String>,
}

impl HotkeyScheme {
    /// Resolve the absolute config file path, applying the Flatpak redirect for
    /// XDG-config schemes. `None` when `$HOME` is unset.
    fn path(&self, flatpak_app_id: Option<&str>) -> Option<PathBuf> {
        let base = match self.base {
            HotkeyBase::XdgConfig => xdg_base(flatpak_app_id, true)?,
            HotkeyBase::Home => PathBuf::from(std::env::var_os("HOME")?),
        };
        Some(base.join(self.rel_path))
    }
}

/// PCSX2/DuckStation value form `Keyboard/<Key>`, combos joined with ` & `, e.g.
/// `Keyboard/Alt & Keyboard/Return`. Drop the `Keyboard/` device prefix from each
/// token, normalize, and re-join with ` + `. Empty when no token survives.
fn parse_device_combo(raw: &str) -> String {
    raw.split('&')
        .filter_map(|tok| {
            let key = tok.trim().rsplit('/').next()?.trim();
            let n = normalize_key(key);
            (!n.is_empty()).then_some(n)
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Dolphin value form: backtick-wrapped keys, combos joined with `&`, e.g.
/// `` `Alt`&`Return` `` or `` `F5` ``. Strip backticks, split on `&`, normalize,
/// re-join with ` + `.
fn parse_backtick_combo(raw: &str) -> String {
    raw.replace('`', "")
        .split('&')
        .filter_map(|tok| {
            let n = normalize_key(tok.trim());
            (!n.is_empty()).then_some(n)
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

/// Scan a `[section]` of an INI config, matching each `key -> action` in
/// `keymap` and normalizing its value with `value`. Shared by every `[Hotkeys]`
/// emulator; only the key map and value grammar differ.
fn map_ini_hotkeys(
    text: &str,
    section: &str,
    keymap: &[(&str, HotkeyAction)],
    value: fn(&str) -> String,
) -> HashMap<HotkeyAction, String> {
    let mut out = HashMap::new();
    for (k, v) in ini_section_pairs(text, section) {
        if let Some((_, action)) = keymap.iter().find(|(name, _)| *name == k) {
            let binding = value(&v);
            if !binding.is_empty() {
                out.insert(*action, binding);
            }
        }
    }
    out
}

const PCSX2_HOTKEY_MAP: &[(&str, HotkeyAction)] = &[
    ("SaveStateToSlot", HotkeyAction::SaveState),
    ("LoadStateFromSlot", HotkeyAction::LoadState),
    ("NextSaveStateSlot", HotkeyAction::NextSlot),
    ("PreviousSaveStateSlot", HotkeyAction::PrevSlot),
    ("Screenshot", HotkeyAction::Screenshot),
    ("TogglePause", HotkeyAction::Pause),
    ("HoldTurbo", HotkeyAction::FastForwardHold),
    ("ToggleTurbo", HotkeyAction::FastForwardToggle),
    ("ToggleFullscreen", HotkeyAction::ToggleFullscreen),
    ("OpenPauseMenu", HotkeyAction::ToggleMenu),
];

fn parse_pcsx2_hotkeys(text: &str) -> HashMap<HotkeyAction, String> {
    map_ini_hotkeys(text, "Hotkeys", PCSX2_HOTKEY_MAP, parse_device_combo)
}

const DUCKSTATION_HOTKEY_MAP: &[(&str, HotkeyAction)] = &[
    ("SaveSelectedSaveState", HotkeyAction::SaveState),
    ("LoadSelectedSaveState", HotkeyAction::LoadState),
    ("SelectNextSaveStateSlot", HotkeyAction::NextSlot),
    ("SelectPreviousSaveStateSlot", HotkeyAction::PrevSlot),
    ("Screenshot", HotkeyAction::Screenshot),
    ("TogglePause", HotkeyAction::Pause),
    ("FastForward", HotkeyAction::FastForwardHold),
    ("ToggleFullscreen", HotkeyAction::ToggleFullscreen),
    ("OpenPauseMenu", HotkeyAction::ToggleMenu),
];

fn parse_duckstation_hotkeys(text: &str) -> HashMap<HotkeyAction, String> {
    map_ini_hotkeys(text, "Hotkeys", DUCKSTATION_HOTKEY_MAP, parse_device_combo)
}

const DOLPHIN_HOTKEY_MAP: &[(&str, HotkeyAction)] = &[
    ("Save State/Save to Selected Slot", HotkeyAction::SaveState),
    ("Load State/Load from Selected Slot", HotkeyAction::LoadState),
    ("General/Take Screenshot", HotkeyAction::Screenshot),
    ("General/Toggle Pause", HotkeyAction::Pause),
    ("General/Toggle Fullscreen", HotkeyAction::ToggleFullscreen),
    ("General/Stop", HotkeyAction::Exit),
];

fn parse_dolphin_hotkeys(text: &str) -> HashMap<HotkeyAction, String> {
    map_ini_hotkeys(text, "Hotkeys", DOLPHIN_HOTKEY_MAP, parse_backtick_combo)
}

const MUPEN_HOTKEY_MAP: &[(&str, HotkeyAction)] = &[
    ("Kbd Mapping Save State", HotkeyAction::SaveState),
    ("Kbd Mapping Load State", HotkeyAction::LoadState),
    ("Kbd Mapping Increment Slot", HotkeyAction::NextSlot),
    ("Kbd Mapping Screenshot", HotkeyAction::Screenshot),
    ("Kbd Mapping Pause", HotkeyAction::Pause),
    ("Kbd Mapping Fast Forward", HotkeyAction::FastForwardHold),
    ("Kbd Mapping Reset", HotkeyAction::Reset),
    ("Kbd Mapping Stop", HotkeyAction::Exit),
    ("Kbd Mapping Fullscreen", HotkeyAction::ToggleFullscreen),
];

/// Mupen64Plus `[CoreEvents]`: each value is an SDL *keysym* integer (sometimes
/// quoted). Translate via [`sdl_keysym_name`]; `0`/unknown yields empty so the
/// action falls back to its documented default.
fn parse_mupen_keysym(raw: &str) -> String {
    match raw.trim().trim_matches('"').parse::<u32>() {
        Ok(n) => sdl_keysym_name(n),
        Err(_) => String::new(),
    }
}

fn parse_mupen_hotkeys(text: &str) -> HashMap<HotkeyAction, String> {
    map_ini_hotkeys(text, "CoreEvents", MUPEN_HOTKEY_MAP, parse_mupen_keysym)
}

const MEDNAFEN_HOTKEY_MAP: &[(&str, HotkeyAction)] = &[
    ("command.save_state", HotkeyAction::SaveState),
    ("command.load_state", HotkeyAction::LoadState),
    ("command.state_slot_inc", HotkeyAction::NextSlot),
    ("command.state_slot_dec", HotkeyAction::PrevSlot),
    ("command.take_snapshot", HotkeyAction::Screenshot),
    ("command.pause", HotkeyAction::Pause),
    ("command.fast_forward", HotkeyAction::FastForwardHold),
    ("command.reset", HotkeyAction::Reset),
    ("command.exit", HotkeyAction::Exit),
];

/// Mednafen's `mednafen.cfg` is flat whitespace-delimited lines, not INI
/// sections: `command.<action> keyboard 0x0 <SDL scancode>`. Match the action,
/// require the `keyboard` device, and translate the trailing scancode via
/// [`sdl_scancode_name`].
fn parse_mednafen_hotkeys(text: &str) -> HashMap<HotkeyAction, String> {
    let mut out = HashMap::new();
    for line in text.lines() {
        let mut toks = line.split_whitespace();
        let Some(key) = toks.next() else { continue };
        let Some((_, action)) = MEDNAFEN_HOTKEY_MAP.iter().find(|(name, _)| *name == key) else {
            continue;
        };
        let rest: Vec<&str> = toks.collect();
        if rest.first() != Some(&"keyboard") {
            continue;
        }
        if let Some(code) = rest.last().and_then(|c| c.parse::<u32>().ok()) {
            let binding = sdl_scancode_name(code);
            if !binding.is_empty() {
                out.insert(*action, binding);
            }
        }
    }
    out
}

/// A GTK accelerator string (`<Shift><Control>F1`, `<Control>s`, `F5`) as written
/// by snes9x-gtk's `[Shortcuts]`. Pull out the leading `<Mod>` tokens, normalize
/// the trailing key, and re-join with ` + `. `Unset`/empty → empty (unbound).
fn parse_gtk_accel(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("unset") {
        return String::new();
    }
    let mut mods = Vec::new();
    let mut rest = raw;
    while let Some(open) = rest.find('<') {
        let Some(close_rel) = rest[open..].find('>') else { break };
        let name = &rest[open + 1..open + close_rel];
        let label = match name.to_ascii_lowercase().as_str() {
            "shift" => "Shift",
            "control" | "ctrl" | "primary" => "Ctrl",
            "alt" | "mod1" => "Alt",
            "super" => "Super",
            _ => "",
        };
        if !label.is_empty() {
            mods.push(label.to_string());
        }
        rest = &rest[open + close_rel + 1..];
    }
    let key = normalize_key(rest.trim());
    if key.is_empty() {
        return String::new();
    }
    mods.push(key);
    mods.join(" + ")
}

const SNES9X_HOTKEY_MAP: &[(&str, HotkeyAction)] = &[
    ("GTK_state_save_current", HotkeyAction::SaveState),
    ("GTK_state_load_current", HotkeyAction::LoadState),
    ("GTK_state_increment", HotkeyAction::NextSlot),
    ("GTK_state_decrement", HotkeyAction::PrevSlot),
    ("Screenshot", HotkeyAction::Screenshot),
    ("GTK_pause", HotkeyAction::Pause),
    ("GTK_fullscreen", HotkeyAction::ToggleFullscreen),
    ("GTK_rewind", HotkeyAction::Rewind),
    ("GTK_quit", HotkeyAction::Exit),
];

/// snes9x-gtk `[Shortcuts]`: GTK accelerator strings. snes9x ships these all
/// `Unset` (no compiled defaults), so this is an *override-only* reader — it
/// surfaces whatever the user has bound and the panel stays hidden until they
/// bind something.
fn parse_snes9x_hotkeys(text: &str) -> HashMap<HotkeyAction, String> {
    map_ini_hotkeys(text, "Shortcuts", SNES9X_HOTKEY_MAP, parse_gtk_accel)
}

/// For emulators whose config stores overrides in a form we don't translate yet
/// (e.g. mGBA's Qt keycodes): read nothing, leaning entirely on the defaults
/// table. The config existing or not makes no difference to the result.
fn no_hotkey_overrides(_text: &str) -> HashMap<HotkeyAction, String> {
    HashMap::new()
}

/// Static description of one standalone emulator.
struct Spec {
    id: &'static str,
    display_name: &'static str,
    platforms: &'static [&'static str],
    bin_names: &'static [&'static str],
    flatpak_app_id: Option<&'static str>,
    capabilities: Capabilities,
    /// Build the per-game arguments (everything after the executable).
    args: fn(rom_path: &str) -> Vec<String>,
    /// How (and whether) this emulator's savestates are discovered. `None` for
    /// emulators we can't yet index reliably — e.g. Mupen64Plus keys states by
    /// the ROM's internal name + CRC, which needs N64-header parsing we don't do.
    save_states: Option<SaveStateScheme>,
    /// How (and whether) this emulator's keyboard hotkeys are read. `None` leaves
    /// the panel hidden (the default-empty `scan_hotkeys`).
    hotkeys: Option<HotkeyScheme>,
}

pub struct StandaloneAdapter {
    spec: Spec,
}

impl StandaloneAdapter {
    fn new(spec: Spec) -> Self {
        Self { spec }
    }

    pub fn snes9x() -> Self {
        Self::new(Spec {
            id: "snes9x",
            display_name: "Snes9x",
            platforms: &["snes"],
            bin_names: &["snes9x-gtk", "snes9x"],
            flatpak_app_id: Some("com.snes9x.Snes9x"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            // Snes9x writes states next to the ROM by default (no folder set),
            // named `<stem>.000`..`.009`.
            save_states: Some(SaveStateScheme {
                location: StateLocation::RomDir,
                key: KeyKind::Stem,
                parse: parse_snes9x,
                learn: None,
            }),
            // `[Shortcuts]` GTK accelerators in `snes9x/snes9x.conf`. snes9x ships
            // them all `Unset` (no compiled defaults), so this is override-only
            // with an empty defaults table — hidden until the user binds keys.
            hotkeys: Some(HotkeyScheme {
                base: HotkeyBase::XdgConfig,
                rel_path: "snes9x/snes9x.conf",
                defaults: &[],
                parse: parse_snes9x_hotkeys,
            }),
        })
    }

    pub fn mgba() -> Self {
        Self::new(Spec {
            id: "mgba",
            display_name: "mGBA",
            platforms: &["gb", "gbc", "gba"],
            bin_names: &["mgba-qt", "mgba", "mgba-sdl"],
            flatpak_app_id: Some("io.mgba.mGBA"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            // mGBA defaults to writing savestates beside the ROM, which breaks for
            // our zip libraries (ROMs are extracted to an ephemeral /tmp dir that's
            // cleaned on exit, taking the state with it). We pin `savestatePath`
            // (and `savegamePath`) in mGBA's config to fixed XDG dirs so states
            // survive and stay discoverable; this scheme reads that fixed dir.
            // Files are named `<stem>.ss0`..`.ss9`.
            save_states: Some(SaveStateScheme {
                location: StateLocation::Data("mgba/savestates"),
                key: KeyKind::Stem,
                parse: parse_mgba,
                learn: None,
            }),
            // mGBA's `[shortcutKey]` is empty by default and stores overrides as
            // Qt keycodes we don't translate yet, so lean on the documented
            // slot-1 save/load defaults.
            hotkeys: Some(HotkeyScheme {
                base: HotkeyBase::XdgConfig,
                rel_path: "mgba/config.ini",
                defaults: crate::hotkeys::MGBA_DEFAULTS,
                parse: no_hotkey_overrides,
            }),
        })
    }

    pub fn mupen64plus() -> Self {
        Self::new(Spec {
            id: "mupen64plus",
            display_name: "Mupen64Plus",
            platforms: &["n64"],
            bin_names: &["mupen64plus"],
            flatpak_app_id: None,
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            // Mupen64Plus names states (and battery saves) `<GoodName>-<CRC>.stN`,
            // where both fields come from its bundled ROM database
            // (mupen64plus.ini) via an MD5 lookup — NOT from the ROM header.
            // Verified against real files: a ROM with header CRC1 3E5055B6 (and
            // file CRC32 434389C1) is saved as `Mario Kart 64 (U) [!]-3A67D998`,
            // the canonical database value, matching neither. We can't
            // reconstruct that without reimplementing the emulator, so instead we
            // *learn* the base by reading back what Mupen wrote during a session
            // (`learn`), persist it, and feed it through `disc_key`. The grid is
            // empty until the first save+exit through Arcadia, then self-heals.
            save_states: Some(SaveStateScheme {
                location: StateLocation::Data("mupen64plus/save"),
                key: KeyKind::DiscKey,
                parse: parse_mupen,
                learn: Some(learn_mupen_base),
            }),
            // `[CoreEvents]` `Kbd Mapping <Action>` in `mupen64plus.cfg`; values
            // are SDL keysym integers.
            hotkeys: Some(HotkeyScheme {
                base: HotkeyBase::XdgConfig,
                rel_path: "mupen64plus/mupen64plus.cfg",
                defaults: crate::hotkeys::MUPEN_DEFAULTS,
                parse: parse_mupen_hotkeys,
            }),
        })
    }

    pub fn dolphin() -> Self {
        Self::new(Spec {
            id: "dolphin",
            display_name: "Dolphin",
            platforms: &["gamecube", "wii"],
            bin_names: &["dolphin-emu"],
            flatpak_app_id: Some("org.DolphinEmu.dolphin-emu"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: true,
                screenshots: true,
            },
            // -b: batch (exit when emulation ends), -e: execute file
            args: |rom| vec!["-b".into(), "-e".into(), rom.to_string()],
            // Dolphin keys `StateSaves/<GAMEID>.sNN` by the 6-char disc id. We can
            // usually read the id from the ROM header, but a zipped/extracted ISO
            // hides the original path — so also learn it from Dolphin's own writes.
            save_states: Some(SaveStateScheme {
                location: StateLocation::Data("dolphin-emu/StateSaves"),
                key: KeyKind::DiscKey,
                parse: parse_dolphin,
                learn: Some(learn_dolphin_game_id),
            }),
            // `[Hotkeys]` in `dolphin-emu/Hotkeys.ini`, compound `Category/Name`
            // keys with backtick-wrapped values.
            hotkeys: Some(HotkeyScheme {
                base: HotkeyBase::XdgConfig,
                rel_path: "dolphin-emu/Hotkeys.ini",
                defaults: crate::hotkeys::DOLPHIN_DEFAULTS,
                parse: parse_dolphin_hotkeys,
            }),
        })
    }

    pub fn pcsx2() -> Self {
        Self::new(Spec {
            id: "pcsx2",
            display_name: "PCSX2",
            platforms: &["ps2"],
            bin_names: &["pcsx2-qt", "pcsx2"],
            flatpak_app_id: Some("net.pcsx2.PCSX2"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: true,
                screenshots: true,
            },
            // -batch: skip the GUI and boot the supplied disc directly
            args: |rom| vec!["-batch".into(), rom.to_string()],
            // PCSX2 names `sstates/<SERIAL> (<CRC>).<slot>.p2s` by the PS2 serial.
            // Read from the ROM when possible, but learn it from PCSX2's own
            // writes too, so zipped/extracted discs (whose original path we can't
            // re-read) still resolve after one play session.
            save_states: Some(SaveStateScheme {
                location: StateLocation::Config("PCSX2/sstates"),
                key: KeyKind::DiscKey,
                parse: parse_pcsx2,
                learn: Some(learn_pcsx2_serial),
            }),
            // `[Hotkeys]` in `PCSX2/inis/PCSX2.ini`, values `Keyboard/<Key>`.
            hotkeys: Some(HotkeyScheme {
                base: HotkeyBase::XdgConfig,
                rel_path: "PCSX2/inis/PCSX2.ini",
                defaults: crate::hotkeys::PCSX2_DEFAULTS,
                parse: parse_pcsx2_hotkeys,
            }),
        })
    }

    pub fn ppsspp() -> Self {
        Self::new(Spec {
            id: "ppsspp",
            display_name: "PPSSPP",
            platforms: &["psp"],
            bin_names: &["PPSSPPSDL", "PPSSPPQt", "ppsspp"],
            flatpak_app_id: Some("org.ppsspp.PPSSPP"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: true,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            // PPSSPP keys states by a content id under its own memstick tree;
            // not yet mapped.
            save_states: None,
            hotkeys: None,
        })
    }

    pub fn rpcs3() -> Self {
        Self::new(Spec {
            id: "rpcs3",
            display_name: "RPCS3",
            platforms: &["ps3"],
            bin_names: &["rpcs3"],
            flatpak_app_id: Some("net.rpcs3.RPCS3"),
            capabilities: Capabilities {
                savestates: false,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            // --no-gui: boot the given EBOOT.BIN / disc folder straight to the
            // game and exit when it closes (couch-friendly, no game-list window).
            args: |rom| vec!["--no-gui".into(), rom.to_string()],
            // RPCS3 has no user-facing savestates (capability is false).
            save_states: None,
            hotkeys: None,
        })
    }

    pub fn duckstation() -> Self {
        Self::new(Spec {
            id: "duckstation",
            display_name: "DuckStation",
            platforms: &["ps1"],
            bin_names: &["duckstation-qt", "duckstation-nogui", "duckstation"],
            flatpak_app_id: Some("org.duckstation.DuckStation"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: true, // RetroAchievements
                screenshots: true,
            },
            // -batch: boot the disc directly and close when the game exits.
            args: |rom| vec!["-batch".into(), rom.to_string()],
            // DuckStation names `savestates/<SERIAL>_resume.sav` / `_<n>.sav`.
            // PS1 serials live in raw/CHD sectors the engine can't read yet, so
            // `disc_key` is currently None for ps1 and this yields nothing live —
            // the parser is in place for when serial resolution lands.
            save_states: Some(SaveStateScheme {
                location: StateLocation::Config("duckstation/savestates"),
                key: KeyKind::DiscKey,
                parse: parse_duckstation,
                // PS1 serials live in raw/CHD sectors we can't always read, and a
                // zipped disc hides its original path entirely — so learn the
                // serial from DuckStation's own writes after a session.
                learn: Some(learn_duckstation_serial),
            }),
            // `[Hotkeys]` in `duckstation/settings.ini`, same `Keyboard/<Key>`
            // grammar as PCSX2.
            hotkeys: Some(HotkeyScheme {
                base: HotkeyBase::XdgConfig,
                rel_path: "duckstation/settings.ini",
                defaults: crate::hotkeys::DUCKSTATION_DEFAULTS,
                parse: parse_duckstation_hotkeys,
            }),
        })
    }

    pub fn melonds() -> Self {
        Self::new(Spec {
            id: "melonds",
            display_name: "melonDS",
            platforms: &["nds"],
            bin_names: &["melonDS", "melonds"],
            flatpak_app_id: Some("net.kuribo64.melonDS"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            // melonDS writes states beside the ROM (no Savestate path configured)
            // as `<stem>.ml<slot>`. Read-only discovery — no boot-into-state CLI
            // flag, so it's absent from LAUNCH_STATE_IDS and cards render
            // non-clickable. Because NDS dumps are normally zipped, discovery runs
            // against the extraction cache (the extracted file's dir + inner stem),
            // resolved in `Engine::list_save_states`.
            save_states: Some(SaveStateScheme {
                location: StateLocation::RomDir,
                key: KeyKind::Stem,
                parse: parse_melonds,
                learn: None,
            }),
            hotkeys: None,
        })
    }

    pub fn flycast() -> Self {
        Self::new(Spec {
            id: "flycast",
            display_name: "Flycast",
            platforms: &["dreamcast"],
            bin_names: &["flycast"],
            flatpak_app_id: Some("org.flycast.Flycast"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: true, // RetroAchievements
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            // Flycast writes states to its data dir (flatpak:
            // `~/.var/app/org.flycast.Flycast/data/flycast/`), keyed by ROM stem:
            // `<stem>.state` (slot 0) / `<stem>_N.state` (slot N). Read-only
            // discovery — Flycast has no boot-into-state CLI flag, so it's not in
            // LAUNCH_STATE_IDS and the cards render non-clickable.
            save_states: Some(SaveStateScheme {
                location: StateLocation::Data("flycast"),
                key: KeyKind::Stem,
                parse: parse_flycast,
                learn: None,
            }),
            hotkeys: None,
        })
    }

    pub fn mame() -> Self {
        Self::new(Spec {
            id: "mame",
            display_name: "MAME",
            platforms: &["arcade"],
            bin_names: &["mame"],
            flatpak_app_id: Some("org.mamedev.MAME"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            // MAME boots a *set name*, not a path, and resolves it against a
            // configured ROM directory. Point `-rompath` at the romset's folder
            // and pass the bare set name (the filename without `.zip`).
            args: |rom| {
                let p = std::path::Path::new(rom);
                let dir = p
                    .parent()
                    .map(|d| d.to_string_lossy().to_string())
                    .unwrap_or_default();
                let set = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_default();
                vec!["-rompath".into(), dir, set]
            },
            save_states: None,
            hotkeys: None,
        })
    }

    pub fn xemu() -> Self {
        Self::new(Spec {
            id: "xemu",
            display_name: "xemu",
            platforms: &["xbox"],
            bin_names: &["xemu"],
            flatpak_app_id: Some("app.xemu.xemu"),
            capabilities: Capabilities {
                savestates: false,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            // xemu takes the disc image as a virtual DVD.
            args: |rom| vec!["-dvd_path".into(), rom.to_string()],
            save_states: None,
            hotkeys: None,
        })
    }

    pub fn cemu() -> Self {
        Self::new(Spec {
            id: "cemu",
            display_name: "Cemu",
            platforms: &["wiiu"],
            bin_names: &["Cemu", "cemu"],
            flatpak_app_id: Some("info.cemu.Cemu"),
            capabilities: Capabilities {
                savestates: false,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            // -f: fullscreen, -g: game to launch.
            args: |rom| vec!["-f".into(), "-g".into(), rom.to_string()],
            save_states: None,
            hotkeys: None,
        })
    }

    pub fn vita3k() -> Self {
        Self::new(Spec {
            id: "vita3k",
            display_name: "Vita3K",
            platforms: &["vita"],
            bin_names: &["Vita3K", "vita3k"],
            flatpak_app_id: Some("org.vita3k.Vita3K"),
            capabilities: Capabilities {
                savestates: false,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            save_states: None,
            hotkeys: None,
        })
    }

    pub fn ryujinx() -> Self {
        Self::new(Spec {
            id: "ryujinx",
            display_name: "Ryujinx",
            platforms: &["switch"],
            bin_names: &["Ryujinx", "Ryujinx.Ava", "ryujinx"],
            flatpak_app_id: Some("org.ryujinx.Ryujinx"),
            capabilities: Capabilities {
                savestates: false,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            save_states: None,
            hotkeys: None,
        })
    }

    pub fn lime3ds() -> Self {
        Self::new(Spec {
            id: "lime3ds",
            display_name: "Lime3DS",
            platforms: &["n3ds"],
            bin_names: &["lime3ds-gui", "lime3ds", "azahar", "citra-qt", "citra"],
            flatpak_app_id: Some("io.github.lime3ds.Lime3DS"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            save_states: None,
            hotkeys: None,
        })
    }

    /// Build a launch-only standalone: it boots ROMs faithfully but Arcadia
    /// doesn't yet index its savestates (no [`SaveStateScheme`]), so its Recall
    /// State grid stays empty and its cards never claim launch-into-state. This
    /// is the long tail of single-system emulators where the win is a dedicated
    /// reference core as the platform default, not state discovery. Capabilities
    /// are uniform (saves + screenshots reported, savestates not); what varies is
    /// the per-game argument builder (most take the bare ROM; a few need a
    /// mode/media flag) and the Flatpak id (set only for the emulators that ship
    /// a Flathub app, so they're installable without sudo).
    fn launch_only(
        id: &'static str,
        display_name: &'static str,
        platforms: &'static [&'static str],
        bin_names: &'static [&'static str],
        flatpak_app_id: Option<&'static str>,
        args: fn(&str) -> Vec<String>,
    ) -> Self {
        Self::new(Spec {
            id,
            display_name,
            platforms,
            bin_names,
            flatpak_app_id,
            capabilities: Capabilities {
                savestates: false,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args,
            save_states: None,
            hotkeys: None,
        })
    }

    /// Mednafen — multi-system, used here only for the platforms where it is the
    /// reference (or only sane) standalone: PC Engine, Saturn, Virtual Boy, Lynx,
    /// WonderSwan, Neo Geo Pocket, and the Sega 8-bits. It autodetects the system
    /// from the ROM, so a bare path is enough. Savestates are discovered read-only
    /// (no startup-load CLI flag, so absent from [`LAUNCH_STATE_IDS`]): they live
    /// in `~/.mednafen/mcs/` as `<stem>.<md5>.mc<slot>`.
    pub fn mednafen() -> Self {
        Self::new(Spec {
            id: "mednafen",
            display_name: "Mednafen",
            platforms: &[
                "pcengine",
                "saturn",
                "virtualboy",
                "lynx",
                "wonderswan",
                "ngp",
                "sms",
                "gamegear",
            ],
            bin_names: &["mednafen"],
            flatpak_app_id: None,
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            save_states: Some(SaveStateScheme {
                location: StateLocation::Home(".mednafen/mcs"),
                key: KeyKind::Stem,
                parse: parse_mednafen,
                learn: None,
            }),
            // Flat `command.<action> keyboard 0x0 <SDL scancode>` lines in
            // `~/.mednafen/mednafen.cfg` (a plain dotdir, not XDG config).
            hotkeys: Some(HotkeyScheme {
                base: HotkeyBase::Home,
                rel_path: ".mednafen/mednafen.cfg",
                defaults: crate::hotkeys::MEDNAFEN_DEFAULTS,
                parse: parse_mednafen_hotkeys,
            }),
        })
    }

    /// Mesen (NES). Savestates discovered read-only (no startup-load CLI flag):
    /// `~/.config/Mesen2/SaveStates/<stem>_<slot>.mss`.
    pub fn mesen() -> Self {
        Self::new(Spec {
            id: "mesen",
            display_name: "Mesen",
            platforms: &["nes"],
            bin_names: &["Mesen", "mesen"],
            flatpak_app_id: None,
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            save_states: Some(SaveStateScheme {
                location: StateLocation::Config("Mesen2/SaveStates"),
                key: KeyKind::Stem,
                parse: parse_mesen,
                learn: None,
            }),
            hotkeys: None,
        })
    }

    pub fn stella() -> Self {
        Self::launch_only(
            "stella",
            "Stella",
            &["atari2600"],
            &["stella"],
            Some("io.github.stella_emu.Stella"),
            |rom| vec![rom.to_string()],
        )
    }

    pub fn blastem() -> Self {
        Self::new(Spec {
            id: "blastem",
            display_name: "BlastEm",
            platforms: &["genesis"],
            bin_names: &["blastem"],
            flatpak_app_id: Some("com.retrodev.blastem"),
            capabilities: Capabilities {
                savestates: true,
                saves: true,
                achievements: false,
                screenshots: true,
            },
            args: |rom| vec![rom.to_string()],
            // BlastEm stores states in a per-game folder under its data dir:
            // `data/blastem/<rom name>/slot_<N>.state` plus `quicksave.state`.
            // Read-only discovery (no boot-into-state CLI flag → not in
            // LAUNCH_STATE_IDS). The folder is named after the file BlastEm ran,
            // i.e. the (extracted) ROM stem, which `DataPerGame` joins on.
            save_states: Some(SaveStateScheme {
                location: StateLocation::DataPerGame("blastem"),
                key: KeyKind::Stem,
                parse: parse_blastem,
                learn: None,
            }),
            hotkeys: None,
        })
    }

    /// openMSX boots an MSX cartridge ROM via `-cart <file>`; our MSX extensions
    /// (`.mx1`/`.mx2`) are cartridge dumps, and the explicit flag avoids relying
    /// on extension-based media autodetection. (Requires an MSX system ROM/BIOS
    /// to actually run — a runtime prerequisite, not a launch-arg concern.)
    pub fn openmsx() -> Self {
        Self::launch_only(
            "openmsx",
            "openMSX",
            &["msx"],
            &["openmsx"],
            Some("org.openmsx.openMSX"),
            |rom| vec!["-cart".into(), rom.to_string()],
        )
    }

    pub fn gearcoleco() -> Self {
        Self::launch_only(
            "gearcoleco",
            "Gearcoleco",
            &["colecovision"],
            &["gearcoleco"],
            None,
            |rom| vec![rom.to_string()],
        )
    }

    pub fn bigpemu() -> Self {
        Self::launch_only(
            "bigpemu",
            "BigPEmu",
            &["jaguar"],
            &["BigPEmu", "bigpemu"],
            Some("com.richwhitehouse.BigPEmu"),
            |rom| vec![rom.to_string()],
        )
    }

    /// VICE's x64sc (accuracy-first C64 core; falls back to x64). `-autostart`
    /// attaches the disk/tape/cartridge and runs it, rather than dropping to the
    /// BASIC prompt as a bare positional file would.
    pub fn vice() -> Self {
        Self::launch_only("vice", "VICE", &["c64"], &["x64sc", "x64"], None, |rom| {
            vec!["-autostart".into(), rom.to_string()]
        })
    }

    /// Atari800 emulates both the 8-bit computers and the 5200 console; `-5200`
    /// forces 5200 mode so a `.a52` cartridge boots as console, not computer.
    pub fn atari800() -> Self {
        Self::launch_only(
            "atari800",
            "Atari800",
            &["atari5200"],
            &["atari800"],
            None,
            |rom| vec!["-5200".into(), rom.to_string()],
        )
    }

    /// Work around the SDL-HIDAPI vs xpadneo conflict for SDL-input emulators.
    ///
    /// A Bluetooth Xbox pad driven by the xpadneo kernel driver is enumerated by
    /// SDL's HIDAPI joystick driver, which reports the right button/axis counts
    /// but delivers *zero* input through it. Disabling HIDAPI alone is not enough:
    /// SDL's evdev backend also reports zero events for this pad. The combination
    /// that restores a full event stream — verified through both the SDL Joystick
    /// and GameController APIs (the latter is what these emulators bind to) — is
    /// HIDAPI off *plus* SDL's classic `/dev/input/jsN` backend
    /// (`SDL_JOYSTICK_HIDAPI=0` and `SDL_JOYSTICK_LINUX_CLASSIC=1`). Other
    /// controllers are usually fine on HIDAPI — some need it — so this only
    /// applies to SDL-input emulators, never overrides a value the caller already
    /// put in `ctx.env` (a per-game setting wins), and under the default `Auto`
    /// policy fires only when an xpadneo-bound controller is actually connected.
    /// See `Design_Discussions_00.md` → "Troubleshooting Log: Xbox … Mupen64Plus".
    fn apply_sdl_hidapi_workaround(&self, ctx: &LaunchContext, cmd: &mut tokio::process::Command) {
        use crate::config::HidapiWorkaround;
        // Every standalone emulator we drive reads controllers through SDL's
        // joystick subsystem and so hits the SDL-HIDAPI vs xpadneo conflict — the
        // sole exception is Dolphin, which on Linux uses its own native evdev
        // input backend (configured separately) and never touches SDL joystick
        // input. A denylist is therefore both correct and future-proof: emulators
        // that genuinely use SDL get the fix, new SDL emulators are covered
        // automatically, and anything that doesn't use SDL simply ignores these
        // env vars (a harmless no-op). Combined with the `Auto` gate below — which
        // only fires when the offending pad is actually present — broad application
        // never perturbs other controllers or setups.
        const NON_SDL_INPUT_IDS: &[&str] = &["dolphin"];
        if NON_SDL_INPUT_IDS.contains(&self.spec.id)
            || ctx.env.iter().any(|(k, _)| k == "SDL_JOYSTICK_HIDAPI")
        {
            return;
        }
        let apply = match ctx.sdl_hidapi_workaround {
            HidapiWorkaround::Off => false,
            HidapiWorkaround::Force => true,
            HidapiWorkaround::Auto => xpadneo_controller_present(),
        };
        if apply {
            cmd.env("SDL_JOYSTICK_HIDAPI", "0");
            cmd.env("SDL_JOYSTICK_LINUX_CLASSIC", "1");
        }
    }

    /// Splice a "boot into this savestate" flag into the launch args for the
    /// emulators that support loading a state at startup. Each arm yields the
    /// per-emulator flag(s) and where they go relative to the ROM (see
    /// [`LoadStatePlacement`]); every supported id is also listed in
    /// [`LAUNCH_STATE_IDS`]. Emulators without a startup-load option leave the
    /// args untouched and boot cold.
    ///
    /// Per the design rule, this only ever hands the emulator its own documented
    /// CLI flag pointing at its own state file — no state-payload parsing.
    fn apply_load_state(&self, ctx: &LaunchContext, args: &mut Vec<String>) {
        let Some(load) = &ctx.load_state else {
            return;
        };
        let (placement, extra): (LoadStatePlacement, Vec<String>) = match self.spec.id {
            // `--savestate <file>` loads the state at boot. But a state restored
            // at startup under the dynamic recompiler renders black / "puzzle" on
            // stock builds (mupen64plus-core#1089): the dynarec can't resume a
            // cold-loaded state unless it was built with NEW_DYNAREC. The
            // maintainer-confirmed, build-independent fix is to force the pure
            // interpreter for this boot via `--emumode 0`; it reliably restores
            // the state, and N64 runs full-speed on the interpreter on modern
            // hardware. Only this launch is affected — normal launches keep the
            // configured (faster) emulator.
            "mupen64plus" => (
                LoadStatePlacement::BeforeRom,
                vec![
                    "--emumode".into(),
                    "0".into(),
                    "--savestate".into(),
                    load.path.clone(),
                ],
            ),
            // PCSX2 / DuckStation both take `-statefile <path>` and, like Mupen,
            // expect options before the positional disc. The ROM is left in place
            // (args still end with it); `-batch -statefile <path> <rom>` loads the
            // state for that disc. (DuckStation docs note the boot filename is
            // optional with `-statefile`, but passing it is harmless and keeps the
            // command shape uniform across launches.)
            "pcsx2" | "duckstation" => (
                LoadStatePlacement::BeforeRom,
                vec!["-statefile".into(), load.path.clone()],
            ),
            // Dolphin loads with `-s <file>`, but its launch arg is `-e <rom>`,
            // which consumes the *next* token — so the state flag must follow the
            // ROM, not precede it. Discovery yields an absolute path, which is
            // what `-s` wants.
            "dolphin" => (
                LoadStatePlacement::AfterRom,
                vec!["-s".into(), load.path.clone()],
            ),
            // mGBA loads a state file at boot with `-t <file>` (`--savestate`),
            // an option that precedes the positional ROM. Discovery yields the
            // absolute `<stem>.ssN` path beside the ROM, which is what `-t` wants.
            "mgba" => (
                LoadStatePlacement::BeforeRom,
                vec!["-t".into(), load.path.clone()],
            ),
            // Other standalones either lack a startup-load flag (Snes9x —
            // snes9x#437) or aren't wired up yet (RPCS3 — needs savestate
            // discovery first); they boot cold and the user resumes from the
            // in-emulator menu. These must NOT be in `LAUNCH_STATE_IDS`.
            _ => return,
        };
        tracing::info!(emulator = self.spec.id, slot = load.slot, "launching into savestate");
        match placement {
            LoadStatePlacement::BeforeRom => {
                let at = args.len().saturating_sub(1); // immediately before the ROM
                for (i, a) in extra.into_iter().enumerate() {
                    args.insert(at + i, a);
                }
            }
            LoadStatePlacement::AfterRom => args.extend(extra),
        }
    }
}

#[async_trait]
impl EmulatorAdapter for StandaloneAdapter {
    fn descriptor(&self) -> AdapterDescriptor {
        AdapterDescriptor {
            id: self.spec.id.to_string(),
            display_name: self.spec.display_name.to_string(),
            platforms: self.spec.platforms.iter().map(|s| s.to_string()).collect(),
            capabilities: self.spec.capabilities,
        }
    }

    async fn detect(&self) -> Result<Vec<EmulatorInstall>, AdapterError> {
        Ok(detect::detect_emulator(
            self.spec.id,
            self.spec.display_name,
            self.spec.bin_names,
            self.spec.flatpak_app_id,
        )
        .await)
    }

    fn handles_archives(&self) -> bool {
        // MAME loads arcade romsets straight from their `.zip` (it takes a set
        // name resolved against a rompath). Every other standalone here expects
        // a plain ROM file, so the launcher pre-extracts zips for them.
        self.spec.id == "mame"
    }

    fn supports_launch_state(&self) -> bool {
        // Exactly the emulators `apply_load_state` has an arm for. Drives the
        // Recall State grid: only these get clickable cards.
        LAUNCH_STATE_IDS.contains(&self.spec.id)
    }

    async fn launch(&self, ctx: &LaunchContext) -> Result<LaunchHandle, AdapterError> {
        let mut args = (self.spec.args)(&ctx.rom_path);
        self.apply_load_state(ctx, &mut args);
        let mut cmd = detect::build_command(ctx, args)?;
        self.apply_sdl_hidapi_workaround(ctx, &mut cmd);
        let child = cmd
            .spawn()
            .map_err(|e| AdapterError::Launch(format!("{}: {e}", self.spec.id)))?;
        let pid = child.id().unwrap_or(0);
        Ok(LaunchHandle { pid, child })
    }

    async fn scan_save_states(&self, ctx: &ScanContext) -> Result<Vec<SaveState>, AdapterError> {
        let Some(scheme) = &self.spec.save_states else {
            return Ok(Vec::new());
        };
        // Resolve the key the emulator names states by. A stem always exists; a
        // disc key may not (unresolved serial), in which case there's nothing to
        // match and we return empty rather than scanning with an empty key.
        let key = match scheme.key {
            KeyKind::Stem => match Path::new(&ctx.rom_path).file_stem().and_then(|s| s.to_str()) {
                Some(stem) => stem.to_string(),
                None => return Ok(Vec::new()),
            },
            KeyKind::DiscKey => match &ctx.disc_key {
                Some(key) => key.clone(),
                None => return Ok(Vec::new()),
            },
        };
        let Some(dir) = scheme.dir(ctx.flatpak_app_id.as_deref(), &ctx.rom_path) else {
            return Ok(Vec::new());
        };
        let parse = scheme.parse;
        Ok(discover_slots(&dir, move |name| parse(name, &key)))
    }

    async fn scan_hotkeys(&self, ctx: &ScanContext) -> Result<Vec<EmulatorHotkey>, AdapterError> {
        let Some(scheme) = &self.spec.hotkeys else {
            return Ok(Vec::new());
        };
        let Some(path) = scheme.path(ctx.flatpak_app_id.as_deref()) else {
            return Ok(Vec::new());
        };
        // A missing/unreadable config is fine: overlay an empty map so the panel
        // still shows the emulator's documented defaults.
        let parsed = std::fs::read_to_string(&path)
            .map(|t| (scheme.parse)(&t))
            .unwrap_or_default();
        Ok(overlay_defaults(scheme.defaults, &parsed))
    }

    async fn learn_state_key(
        &self,
        ctx: &ScanContext,
        since: std::time::SystemTime,
    ) -> Option<String> {
        // Only schemes that can't resolve their key a priori carry a `learn`
        // function; the rest (stem, disc serial/id) need no learning.
        let scheme = self.spec.save_states.as_ref()?;
        let learn = scheme.learn?;
        let dir = scheme.dir(ctx.flatpak_app_id.as_deref(), &ctx.rom_path)?;
        crate::save_states::learn_key_in(&dir, |name| learn(name), since)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pcsx2_hotkeys_parse_from_real_section() {
        // Real `[Hotkeys]` block (Appendix A). `Keyboard/` prefix dropped, combos
        // re-joined with ` + `.
        let ini = "\
[UI]
Theme = dark
[Hotkeys]
SaveStateToSlot = Keyboard/F1
LoadStateFromSlot = Keyboard/F3
PreviousSaveStateSlot = Keyboard/Shift & Keyboard/F2
ToggleFullscreen = Keyboard/Alt & Keyboard/Return
OpenPauseMenu = Keyboard/Escape
";
        let m = parse_pcsx2_hotkeys(ini);
        assert_eq!(m.get(&HotkeyAction::SaveState).map(String::as_str), Some("F1"));
        assert_eq!(m.get(&HotkeyAction::LoadState).map(String::as_str), Some("F3"));
        assert_eq!(m.get(&HotkeyAction::PrevSlot).map(String::as_str), Some("Shift + F2"));
        assert_eq!(
            m.get(&HotkeyAction::ToggleFullscreen).map(String::as_str),
            Some("Alt + Return")
        );
        assert_eq!(m.get(&HotkeyAction::ToggleMenu).map(String::as_str), Some("Esc"));
    }

    #[test]
    fn duckstation_hotkeys_share_grammar_distinct_keys() {
        let ini = "\
[Hotkeys]
SaveSelectedSaveState = Keyboard/F2
LoadSelectedSaveState = Keyboard/F1
FastForward = Keyboard/Tab
ToggleFullscreen = Keyboard/F11
";
        let m = parse_duckstation_hotkeys(ini);
        assert_eq!(m.get(&HotkeyAction::SaveState).map(String::as_str), Some("F2"));
        assert_eq!(m.get(&HotkeyAction::LoadState).map(String::as_str), Some("F1"));
        assert_eq!(
            m.get(&HotkeyAction::FastForwardHold).map(String::as_str),
            Some("Tab")
        );
        assert_eq!(
            m.get(&HotkeyAction::ToggleFullscreen).map(String::as_str),
            Some("F11")
        );
    }

    #[test]
    fn dolphin_hotkeys_strip_backticks_and_skip_device() {
        // Real `[Hotkeys]` block: compound keys, backtick values, combos on `&`.
        let ini = "\
[Hotkeys]
Device = DInput/0/Keyboard Mouse
Save State/Save to Selected Slot = `F5`
Load State/Load from Selected Slot = `F7`
General/Toggle Fullscreen = `Alt`&`Return`
General/Stop = `Escape`
";
        let m = parse_dolphin_hotkeys(ini);
        assert_eq!(m.get(&HotkeyAction::SaveState).map(String::as_str), Some("F5"));
        assert_eq!(m.get(&HotkeyAction::LoadState).map(String::as_str), Some("F7"));
        assert_eq!(
            m.get(&HotkeyAction::ToggleFullscreen).map(String::as_str),
            Some("Alt + Return")
        );
        assert_eq!(m.get(&HotkeyAction::Exit).map(String::as_str), Some("Esc"));
        // The `Device =` line is not a hotkey.
        assert_eq!(m.len(), 4);
    }

    #[test]
    fn mupen_hotkeys_translate_sdl_keysyms() {
        // Real `[CoreEvents]` block (verified values). Values are SDL keysyms;
        // `0` (Increment Slot / Fullscreen) is unbound → dropped, falls to default.
        let ini = "\
[Core]
Version = 1
[CoreEvents]
Kbd Mapping Save State = 286
Kbd Mapping Load State = 288
Kbd Mapping Increment Slot = 0
Kbd Mapping Screenshot = 293
Kbd Mapping Pause = 112
Kbd Mapping Fast Forward = 102
Kbd Mapping Stop = 27
Kbd Mapping Reset = 290
Kbd Mapping Fullscreen = 0
";
        let m = parse_mupen_hotkeys(ini);
        assert_eq!(m.get(&HotkeyAction::SaveState).map(String::as_str), Some("F5"));
        assert_eq!(m.get(&HotkeyAction::LoadState).map(String::as_str), Some("F7"));
        assert_eq!(m.get(&HotkeyAction::Screenshot).map(String::as_str), Some("F12"));
        assert_eq!(m.get(&HotkeyAction::Pause).map(String::as_str), Some("P"));
        assert_eq!(
            m.get(&HotkeyAction::FastForwardHold).map(String::as_str),
            Some("F")
        );
        assert_eq!(m.get(&HotkeyAction::Exit).map(String::as_str), Some("Esc"));
        assert_eq!(m.get(&HotkeyAction::Reset).map(String::as_str), Some("F9"));
        // Unbound (`0`) actions are absent.
        assert!(m.get(&HotkeyAction::NextSlot).is_none());
        assert!(m.get(&HotkeyAction::ToggleFullscreen).is_none());
    }

    #[test]
    fn mednafen_hotkeys_translate_sdl_scancodes() {
        // Real flat `command.<action> keyboard 0x0 <scancode>` lines (verified).
        let cfg = "\
command.exit keyboard 0x0 69
command.fast_forward keyboard 0x0 53
command.load_state keyboard 0x0 64
command.pause keyboard 0x0 72
command.reset keyboard 0x0 67
command.save_state keyboard 0x0 62
command.state_slot_dec keyboard 0x0 45
command.state_slot_inc keyboard 0x0 46
command.take_snapshot keyboard 0x0 66
video.fs 0
";
        let m = parse_mednafen_hotkeys(cfg);
        assert_eq!(m.get(&HotkeyAction::SaveState).map(String::as_str), Some("F5"));
        assert_eq!(m.get(&HotkeyAction::LoadState).map(String::as_str), Some("F7"));
        assert_eq!(m.get(&HotkeyAction::NextSlot).map(String::as_str), Some("="));
        assert_eq!(m.get(&HotkeyAction::PrevSlot).map(String::as_str), Some("-"));
        assert_eq!(m.get(&HotkeyAction::Screenshot).map(String::as_str), Some("F9"));
        assert_eq!(m.get(&HotkeyAction::Pause).map(String::as_str), Some("Pause"));
        assert_eq!(
            m.get(&HotkeyAction::FastForwardHold).map(String::as_str),
            Some("`")
        );
        assert_eq!(m.get(&HotkeyAction::Reset).map(String::as_str), Some("F10"));
        assert_eq!(m.get(&HotkeyAction::Exit).map(String::as_str), Some("F12"));
        // The unrelated `video.fs` line is ignored.
        assert_eq!(m.len(), 9);
    }

    #[test]
    fn snes9x_gtk_accelerators_parse_and_skip_unset() {
        // `[Shortcuts]` GTK accel strings; all-`Unset` is the stock state.
        let conf = "\
[Joypad 0]
A = Unset
[Shortcuts]
GTK_state_save_current   = <Shift>F1
GTK_state_load_current   = F1
GTK_fullscreen           = <Control><Alt>F
GTK_pause                = Unset
Screenshot               = Unset
";
        let m = parse_snes9x_hotkeys(conf);
        assert_eq!(
            m.get(&HotkeyAction::SaveState).map(String::as_str),
            Some("Shift + F1")
        );
        assert_eq!(m.get(&HotkeyAction::LoadState).map(String::as_str), Some("F1"));
        assert_eq!(
            m.get(&HotkeyAction::ToggleFullscreen).map(String::as_str),
            Some("Ctrl + Alt + F")
        );
        // `Unset` bindings are dropped (not surfaced as empty rows).
        assert!(m.get(&HotkeyAction::Pause).is_none());
        assert!(m.get(&HotkeyAction::Screenshot).is_none());
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn pcsx2_filenames_map_to_slots() {
        // Real captured names: `<SERIAL> (<CRC>).<slot>.p2s`.
        assert_eq!(
            parse_pcsx2("SLUS-20415 (2999BCF9).resume.p2s", "SLUS-20415"),
            Some(-1)
        );
        assert_eq!(parse_pcsx2("SLUS-20415 (2999BCF9).00.p2s", "SLUS-20415"), Some(0));
        assert_eq!(parse_pcsx2("SLUS-20415 (2999BCF9).09.p2s", "SLUS-20415"), Some(9));
        // A different game's state, and PCSX2's `.backup` sibling, are rejected.
        assert_eq!(parse_pcsx2("SCUS-97134 (6F4056DB).00.p2s", "SLUS-20415"), None);
        assert_eq!(
            parse_pcsx2("SLUS-20415 (2999BCF9).00.p2s.backup", "SLUS-20415"),
            None
        );
    }

    #[test]
    fn duckstation_filenames_map_to_slots() {
        // Real captured names: `<SERIAL>_resume.sav` / `<SERIAL>_<n>.sav`.
        assert_eq!(parse_duckstation("SCUS-94163_resume.sav", "SCUS-94163"), Some(-1));
        assert_eq!(parse_duckstation("SCUS-94163_1.sav", "SCUS-94163"), Some(1));
        // Different serial, and the memory-card file, are rejected.
        assert_eq!(parse_duckstation("SCUS-94900_resume.sav", "SCUS-94163"), None);
        assert_eq!(parse_duckstation("SCUS-94163.mcd", "SCUS-94163"), None);
    }

    #[test]
    fn snes9x_filenames_map_to_slots() {
        assert_eq!(parse_snes9x("Super Mario World.000", "Super Mario World"), Some(0));
        assert_eq!(parse_snes9x("Super Mario World.009", "Super Mario World"), Some(9));
        // Battery save and a different game's state are skipped.
        assert_eq!(parse_snes9x("Super Mario World.srm", "Super Mario World"), None);
        assert_eq!(parse_snes9x("Zelda.000", "Super Mario World"), None);
    }

    #[test]
    fn mgba_filenames_map_to_slots() {
        // mGBA states beside the ROM: `<stem>.ss0`..`.ss9`.
        assert_eq!(parse_mgba("Metroid Fusion.ss0", "Metroid Fusion"), Some(0));
        assert_eq!(parse_mgba("Metroid Fusion.ss9", "Metroid Fusion"), Some(9));
        // Battery save, screenshot, and a different game's state are skipped.
        assert_eq!(parse_mgba("Metroid Fusion.sav", "Metroid Fusion"), None);
        assert_eq!(parse_mgba("Metroid Fusion.png", "Metroid Fusion"), None);
        assert_eq!(parse_mgba("Golden Sun.ss0", "Metroid Fusion"), None);
    }

    #[test]
    fn flycast_filenames_map_to_slots() {
        // Real captured name (slot 0): `<stem>.state` in the Flycast data dir.
        let stem = "Jet Grind Radio (USA)";
        assert_eq!(parse_flycast("Jet Grind Radio (USA).state", stem), Some(0));
        // Numbered slots: `<stem>_N.state`.
        assert_eq!(parse_flycast("Jet Grind Radio (USA)_1.state", stem), Some(1));
        assert_eq!(parse_flycast("Jet Grind Radio (USA)_9.state", stem), Some(9));
        // The `.state.net` autosave/netplay variant and another game are skipped.
        assert_eq!(parse_flycast("Jet Grind Radio (USA).state.net", stem), None);
        assert_eq!(parse_flycast("Sonic Adventure (USA).state", stem), None);
    }

    #[test]
    fn melonds_filenames_map_to_slots() {
        // Real captured name (slot 1): `<stem>.ml1` beside the extracted ROM.
        let stem = "0028 - Kirby - Canvas Curse (USA)";
        assert_eq!(parse_melonds("0028 - Kirby - Canvas Curse (USA).ml1", stem), Some(1));
        assert_eq!(parse_melonds("0028 - Kirby - Canvas Curse (USA).ml8", stem), Some(8));
        // The `.dsv` battery save and another game's state are skipped.
        assert_eq!(parse_melonds("0028 - Kirby - Canvas Curse (USA).dsv", stem), None);
        assert_eq!(parse_melonds("Mario Kart DS (U).ml1", stem), None);
    }

    #[test]
    fn blastem_filenames_map_to_slots() {
        // Real captured names from `data/blastem/<game>/`: numbered slots plus the
        // F5 quicksave. The discovery dir is already per-game, so the stem arg is
        // unused (passed empty here).
        assert_eq!(parse_blastem("slot_0.state", ""), Some(0));
        assert_eq!(parse_blastem("slot_9.state", ""), Some(9));
        assert_eq!(parse_blastem("quicksave.state", ""), Some(-1));
        // A battery save / non-state file is skipped.
        assert_eq!(parse_blastem("save.sram", ""), None);
        assert_eq!(parse_blastem("slot_x.state", ""), None);
    }

    #[test]
    fn mednafen_filenames_map_to_slots() {
        // Real captured name: `<stem>.<md5>.mc<slot>` in ~/.mednafen/mcs/.
        let stem = "Hook (Europe) (Proto)";
        assert_eq!(
            parse_mednafen("Hook (Europe) (Proto).7a07e3e59f51b620d8c51bef468d4df5.mc0", stem),
            Some(0)
        );
        assert_eq!(
            parse_mednafen("Hook (Europe) (Proto).7a07e3e59f51b620d8c51bef468d4df5.mc9", stem),
            Some(9)
        );
        // Battery save (lives in ~/.mednafen/sav, but guard anyway) and another
        // game's state are skipped.
        assert_eq!(parse_mednafen("Hook (Europe) (Proto).7a07e3e5.sav", stem), None);
        assert_eq!(parse_mednafen("Sonic.deadbeef.mc0", stem), None);
    }

    #[test]
    fn mesen_filenames_map_to_slots() {
        // Real captured name: `<stem>_<slot>.mss` in ~/.config/Mesen2/SaveStates/.
        let stem = "Addams Family, The (USA)";
        assert_eq!(parse_mesen("Addams Family, The (USA)_1.mss", stem), Some(1));
        assert_eq!(parse_mesen("Addams Family, The (USA)_10.mss", stem), Some(10));
        // Auto-save (non-numeric) and another game are skipped.
        assert_eq!(parse_mesen("Addams Family, The (USA)_auto.mss", stem), None);
        assert_eq!(parse_mesen("Super Mario Bros_1.mss", stem), None);
    }

    #[test]
    fn dolphin_filenames_map_to_slots() {
        assert_eq!(parse_dolphin("GALE01.s01", "GALE01"), Some(1));
        assert_eq!(parse_dolphin("GALE01.s10", "GALE01"), Some(10));
        // Different game id, and the input-recording sibling, are rejected.
        assert_eq!(parse_dolphin("GZLE01.s01", "GALE01"), None);
        assert_eq!(parse_dolphin("GALE01.dtm", "GALE01"), None);
    }

    #[test]
    fn mupen_filenames_map_to_slots() {
        // Real captured base: `<GoodName>-<CRC>` from Mupen's bundled database.
        let base = "Mario Kart 64 (U) [!]-3A67D998";
        assert_eq!(parse_mupen("Mario Kart 64 (U) [!]-3A67D998.st0", base), Some(0));
        assert_eq!(parse_mupen("Mario Kart 64 (U) [!]-3A67D998.st9", base), Some(9));
        // Battery saves share the base but are not savestate slots.
        assert_eq!(parse_mupen("Mario Kart 64 (U) [!]-3A67D998.eep", base), None);
        assert_eq!(parse_mupen("Mario Kart 64 (U) [!]-3A67D998.sra", base), None);
        // A different game's state, and a non-numeric slot, are rejected.
        assert_eq!(parse_mupen("Zelda (U)-1234ABCD.st0", base), None);
        assert_eq!(parse_mupen("Mario Kart 64 (U) [!]-3A67D998.stX", base), None);
    }

    #[test]
    fn mupen_base_is_learned_from_any_of_its_files() {
        // A savestate teaches the base by stripping `.stN`.
        assert_eq!(
            learn_mupen_base("Mario Kart 64 (U) [!]-3A67D998.st0").as_deref(),
            Some("Mario Kart 64 (U) [!]-3A67D998")
        );
        // Each battery-save extension teaches the same base.
        assert_eq!(
            learn_mupen_base("Mario Kart 64 (U) [!]-3A67D998.eep").as_deref(),
            Some("Mario Kart 64 (U) [!]-3A67D998")
        );
        assert_eq!(
            learn_mupen_base("Mario Kart 64 (U) [!]-3A67D998.mpk").as_deref(),
            Some("Mario Kart 64 (U) [!]-3A67D998")
        );
        assert_eq!(
            learn_mupen_base("Mario Kart 64 (U) [!]-3A67D998.fla").as_deref(),
            Some("Mario Kart 64 (U) [!]-3A67D998")
        );
        // Unrelated files in the save dir teach nothing.
        assert_eq!(learn_mupen_base("notes.txt"), None);
        assert_eq!(learn_mupen_base("Mario Kart 64 (U) [!]-3A67D998.stX"), None);
    }

    #[test]
    fn serial_keyed_emulators_learn_their_key_from_a_written_state() {
        // PCSX2: `<SERIAL> (<CRC>).<slot>.p2s` -> SERIAL, for both numbered and
        // resume slots; the `.backup` sibling teaches nothing.
        assert_eq!(
            learn_pcsx2_serial("SLUS-20415 (2999BCF9).00.p2s").as_deref(),
            Some("SLUS-20415")
        );
        assert_eq!(
            learn_pcsx2_serial("SLUS-20415 (2999BCF9).resume.p2s").as_deref(),
            Some("SLUS-20415")
        );
        assert_eq!(learn_pcsx2_serial("SLUS-20415 (2999BCF9).00.p2s.backup"), None);
        // The learned serial must satisfy the discovery parser for the same file.
        let s = learn_pcsx2_serial("SLUS-20415 (2999BCF9).07.p2s").unwrap();
        assert_eq!(parse_pcsx2("SLUS-20415 (2999BCF9).07.p2s", &s), Some(7));

        // DuckStation: `<SERIAL>_resume.sav` / `<SERIAL>_<n>.sav` -> SERIAL.
        assert_eq!(
            learn_duckstation_serial("SCUS-94163_resume.sav").as_deref(),
            Some("SCUS-94163")
        );
        assert_eq!(
            learn_duckstation_serial("SCUS-94163_2.sav").as_deref(),
            Some("SCUS-94163")
        );
        assert_eq!(learn_duckstation_serial("SCUS-94163.mcd"), None); // memory card
        let d = learn_duckstation_serial("SCUS-94163_2.sav").unwrap();
        assert_eq!(parse_duckstation("SCUS-94163_2.sav", &d), Some(2));

        // Dolphin: `<GAMEID>.sNN` -> GAMEID; the `.dtm` recording teaches nothing.
        assert_eq!(learn_dolphin_game_id("GALE01.s01").as_deref(), Some("GALE01"));
        assert_eq!(learn_dolphin_game_id("GALE01.dtm"), None);
        let g = learn_dolphin_game_id("GALE01.s03").unwrap();
        assert_eq!(parse_dolphin("GALE01.s03", &g), Some(3));
    }

    /// Build a launch context carrying a savestate to boot into, for exercising
    /// `apply_load_state`'s arg splicing without spawning anything.
    fn ctx_with_state(path: &str) -> LaunchContext {
        LaunchContext {
            executable_path: "emu".into(),
            install_source: crate::models::InstallSource::Manual,
            flatpak_app_id: None,
            rom_path: "/roms/game.rom".into(),
            platform: "test".into(),
            extra_args: Vec::new(),
            env: Vec::new(),
            core_override: None,
            load_state: Some(crate::adapters::LoadState {
                slot: 3,
                path: path.into(),
            }),
            sdl_hidapi_workaround: crate::config::HidapiWorkaround::Auto,
        }
    }

    /// Did `apply_sdl_hidapi_workaround` set `SDL_JOYSTICK_HIDAPI=0` on the
    /// command for this adapter under the given policy?
    fn sets_hidapi(adapter: &StandaloneAdapter, policy: crate::config::HidapiWorkaround) -> bool {
        env_set_to(adapter, policy, "SDL_JOYSTICK_HIDAPI", "0")
    }

    fn env_set_to(
        adapter: &StandaloneAdapter,
        policy: crate::config::HidapiWorkaround,
        key: &str,
        val: &str,
    ) -> bool {
        let mut ctx = ctx_with_state("/save/x.st3");
        ctx.sdl_hidapi_workaround = policy;
        let mut cmd = tokio::process::Command::new("emu");
        adapter.apply_sdl_hidapi_workaround(&ctx, &mut cmd);
        cmd.as_std()
            .get_envs()
            .any(|(k, v)| k == key && v == Some(val.as_ref()))
    }

    #[test]
    fn hidapi_workaround_respects_policy() {
        use crate::config::HidapiWorkaround;
        let sdl = StandaloneAdapter::mupen64plus();
        // Force always sets it; Off never does — regardless of hardware.
        assert!(sets_hidapi(&sdl, HidapiWorkaround::Force));
        assert!(!sets_hidapi(&sdl, HidapiWorkaround::Off));
        // All SDL-input standalones are covered (regression guard: the set used
        // to be a 4-emulator allowlist, leaving Bluetooth pads dead everywhere
        // else; it is now a denylist so every SDL emulator is fixed and new ones
        // are covered automatically). Spot-check a spread across systems.
        for a in [
            StandaloneAdapter::duckstation(),
            StandaloneAdapter::pcsx2(),
            StandaloneAdapter::rpcs3(),
            StandaloneAdapter::ppsspp(),
            StandaloneAdapter::melonds(),
            StandaloneAdapter::flycast(),
            StandaloneAdapter::snes9x(),
            StandaloneAdapter::mgba(),
            StandaloneAdapter::mednafen(),
            StandaloneAdapter::mesen(),
            StandaloneAdapter::xemu(),
            StandaloneAdapter::cemu(),
            StandaloneAdapter::vita3k(),
            StandaloneAdapter::ryujinx(),
            StandaloneAdapter::lime3ds(),
            StandaloneAdapter::mame(),
        ] {
            assert!(sets_hidapi(&a, HidapiWorkaround::Force));
        }
        // Dolphin is the lone exception (native evdev backend) — never touched,
        // even when forced.
        let non_sdl = StandaloneAdapter::dolphin();
        assert!(!sets_hidapi(&non_sdl, HidapiWorkaround::Force));
        // The fix is two-part: disabling HIDAPI alone leaves this pad dead on
        // SDL's evdev backend, so the classic `/dev/input/jsN` backend must be
        // enabled too (regression guard for the GameController-API event drop).
        assert!(env_set_to(&sdl, HidapiWorkaround::Force, "SDL_JOYSTICK_LINUX_CLASSIC", "1"));
        assert!(!env_set_to(&sdl, HidapiWorkaround::Off, "SDL_JOYSTICK_LINUX_CLASSIC", "1"));
    }

    #[test]
    fn hidapi_workaround_yields_to_caller_env() {
        use crate::config::HidapiWorkaround;
        let sdl = StandaloneAdapter::mupen64plus();
        let mut ctx = ctx_with_state("/save/x.st3");
        ctx.sdl_hidapi_workaround = HidapiWorkaround::Force;
        ctx.env.push(("SDL_JOYSTICK_HIDAPI".into(), "1".into()));
        let mut cmd = tokio::process::Command::new("emu");
        sdl.apply_sdl_hidapi_workaround(&ctx, &mut cmd);
        // The adapter must not stomp the caller's explicit value.
        assert!(!cmd
            .as_std()
            .get_envs()
            .any(|(k, _)| k == "SDL_JOYSTICK_HIDAPI"));
    }

    #[test]
    fn mupen_load_state_forces_interpreter_before_rom() {
        let a = StandaloneAdapter::mupen64plus();
        let mut args = (a.spec.args)("/roms/mk64.z64");
        a.apply_load_state(&ctx_with_state("/save/mk64.st3"), &mut args);
        // `--emumode 0` (interpreter workaround) + `--savestate <path>` ahead of
        // the ROM, which stays last.
        assert_eq!(
            args,
            vec!["--emumode", "0", "--savestate", "/save/mk64.st3", "/roms/mk64.z64"]
        );
    }

    #[test]
    fn pcsx2_load_state_inserts_statefile_before_rom() {
        let a = StandaloneAdapter::pcsx2();
        let mut args = (a.spec.args)("/roms/game.iso");
        a.apply_load_state(&ctx_with_state("/sstates/state.p2s"), &mut args);
        assert_eq!(
            args,
            vec!["-batch", "-statefile", "/sstates/state.p2s", "/roms/game.iso"]
        );
    }

    #[test]
    fn duckstation_load_state_inserts_statefile_before_rom() {
        let a = StandaloneAdapter::duckstation();
        let mut args = (a.spec.args)("/roms/game.chd");
        a.apply_load_state(&ctx_with_state("/savestates/s.sav"), &mut args);
        assert_eq!(
            args,
            vec!["-batch", "-statefile", "/savestates/s.sav", "/roms/game.chd"]
        );
    }

    #[test]
    fn dolphin_load_state_appends_after_rom() {
        // Dolphin's `-e <rom>` consumes the next token, so `-s <path>` must come
        // *after* the ROM, not before it.
        let a = StandaloneAdapter::dolphin();
        let mut args = (a.spec.args)("/roms/game.rvz");
        a.apply_load_state(&ctx_with_state("/StateSaves/GALE01.s03"), &mut args);
        assert_eq!(
            args,
            vec!["-b", "-e", "/roms/game.rvz", "-s", "/StateSaves/GALE01.s03"]
        );
    }

    #[test]
    fn mgba_load_state_inserts_savestate_before_rom() {
        let a = StandaloneAdapter::mgba();
        let mut args = (a.spec.args)("/roms/Metroid Fusion.gba");
        a.apply_load_state(&ctx_with_state("/roms/Metroid Fusion.ss3"), &mut args);
        assert_eq!(
            args,
            vec!["-t", "/roms/Metroid Fusion.ss3", "/roms/Metroid Fusion.gba"]
        );
    }

    #[test]
    fn snes9x_load_state_is_a_noop_and_unsupported() {
        // No CLI savestate-load exists for Snes9x; the args are untouched and the
        // adapter must report it can't launch into a state (so the UI gates it).
        let a = StandaloneAdapter::snes9x();
        let mut args = (a.spec.args)("/roms/smw.sfc");
        let before = args.clone();
        a.apply_load_state(&ctx_with_state("/roms/smw.000"), &mut args);
        assert_eq!(args, before);
        assert!(!a.supports_launch_state());
    }

    #[test]
    fn launch_state_capability_matches_apply_arms() {
        // Single source of truth: every id reported as supporting launch-into-state
        // must actually produce extra args, and nothing else may claim support.
        for id in LAUNCH_STATE_IDS {
            let a = match *id {
                "mupen64plus" => StandaloneAdapter::mupen64plus(),
                "pcsx2" => StandaloneAdapter::pcsx2(),
                "duckstation" => StandaloneAdapter::duckstation(),
                "dolphin" => StandaloneAdapter::dolphin(),
                "mgba" => StandaloneAdapter::mgba(),
                other => panic!("LAUNCH_STATE_IDS has no test constructor for {other}"),
            };
            assert!(a.supports_launch_state(), "{id} should support launch state");
            let mut args = (a.spec.args)("/roms/game.rom");
            let before_len = args.len();
            a.apply_load_state(&ctx_with_state("/s/state"), &mut args);
            assert!(args.len() > before_len, "{id} produced no load-state args");
        }
    }

    #[test]
    fn snes9x_scheme_scans_rom_directory() {
        // RomDir + Stem: finds `<stem>.00N` beside the ROM, ignores the battery
        // save and other games' states.
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        std::fs::write(dir.join("Super Mario World.smc"), b"rom").unwrap();
        std::fs::write(dir.join("Super Mario World.000"), b"s0").unwrap();
        std::fs::write(dir.join("Super Mario World.001"), b"s1").unwrap();
        std::fs::write(dir.join("Super Mario World.srm"), b"bat").unwrap();
        std::fs::write(dir.join("Zelda.000"), b"other").unwrap();

        let found = discover_slots(dir, |name| parse_snes9x(name, "Super Mario World"));
        let slots: Vec<i64> = found.iter().map(|s| s.slot).collect();
        assert_eq!(slots, vec![0, 1]);
        assert_eq!(found[0].label, "Slot 0");
    }
}
