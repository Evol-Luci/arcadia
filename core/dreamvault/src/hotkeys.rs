//! Emulator hotkey discovery (the read-only twin of [`crate::save_states`]).
//!
//! For the game the user is looking at, surface the *emulator hotkeys* (save
//! state, load state, slot navigation, screenshot, pause, fast-forward,
//! fullscreen, menu, exit, …) of the emulator that would actually launch it —
//! its resolved default. Per the house rule this only ever *reads and
//! classifies* the emulator's own config; it never writes config, intercepts
//! input, or changes how the emulator behaves (the same rule `controller.rs` and
//! the savestate scanner follow).
//!
//! Most users never rebind, so an emulator's config usually omits the entry
//! entirely. Each adapter therefore overlays the user's parsed bindings onto a
//! static *defaults table* ([`overlay_defaults`]): the parsed binding wins;
//! otherwise the documented default shows, flagged `is_default: true`. An
//! emulator we can't parse (or have no defaults for) simply yields an empty list
//! and the UI hides the panel — honest about gaps rather than showing garbage.

use crate::error::{EngineError, Result};
use crate::saves::flatpak_app_id_of;
use crate::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The canonical hotkey actions worth surfacing, uniform across emulators. Fast
/// forward is split into hold/toggle so each UI row is a single label→binding
/// pair with no special-casing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyAction {
    SaveState,
    LoadState,
    NextSlot,
    PrevSlot,
    Screenshot,
    Pause,
    FastForwardHold,
    FastForwardToggle,
    Rewind,
    ToggleFullscreen,
    ToggleMenu,
    Exit,
    Reset,
}

impl HotkeyAction {
    /// Human label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            HotkeyAction::SaveState => "Save State",
            HotkeyAction::LoadState => "Load State",
            HotkeyAction::NextSlot => "Next Slot",
            HotkeyAction::PrevSlot => "Previous Slot",
            HotkeyAction::Screenshot => "Screenshot",
            HotkeyAction::Pause => "Pause",
            HotkeyAction::FastForwardHold => "Fast Forward (hold)",
            HotkeyAction::FastForwardToggle => "Fast Forward (toggle)",
            HotkeyAction::Rewind => "Rewind",
            HotkeyAction::ToggleFullscreen => "Fullscreen",
            HotkeyAction::ToggleMenu => "Menu",
            HotkeyAction::Exit => "Exit",
            HotkeyAction::Reset => "Reset",
        }
    }
}

/// Which input device a binding is for. Keyboard is the reliable, legible win;
/// controller bindings are device-specific and best-effort (deferred for now).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HotkeyDevice {
    Keyboard,
    Controller,
}

/// One resolved hotkey for the UI: the canonical action, its human label, the
/// normalized display binding (`"F2"`, `"Shift + F1"`, `"Tab"`), whether it came
/// from the defaults table (not the user's own config), and the input device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorHotkey {
    pub action: HotkeyAction,
    pub label: String,
    pub binding: String,
    pub is_default: bool,
    pub device: HotkeyDevice,
}

/// Collect `key = value` pairs inside a named `[Section]` of an INI-style config.
/// Keys may themselves contain `/` and spaces (Dolphin's
/// `Save State/Save to Selected Slot`), so we only split on the *first* `=`.
/// Comment lines (`#`, `;`) and the section header are skipped. Pure over the
/// text, so it's unit-testable without touching the filesystem.
pub(crate) fn ini_section_pairs(text: &str, section: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut in_section = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_section = name.trim() == section;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            out.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    out
}

/// Normalize one raw key token into a consistent display string. Handles the
/// casing differences between emulators (RetroArch's lowercase `f2`/`escape` vs.
/// PCSX2's title-case `F1`/`Escape`): function keys → `F<n>`, single letters →
/// uppercase, a handful of named keys to friendly forms, everything else
/// title-cased. Returns empty for the unbound sentinels (`nul`, empty).
pub(crate) fn normalize_key(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return String::new();
    }
    let lower = raw.to_ascii_lowercase();
    match lower.as_str() {
        "nul" | "unset" => return String::new(),
        "escape" | "esc" => return "Esc".to_string(),
        "space" => return "Space".to_string(),
        "return" | "enter" => return "Return".to_string(),
        "tab" => return "Tab".to_string(),
        "period" => return ".".to_string(),
        "comma" => return ",".to_string(),
        "alt" => return "Alt".to_string(),
        "shift" => return "Shift".to_string(),
        "ctrl" | "control" => return "Ctrl".to_string(),
        "backspace" => return "Backspace".to_string(),
        "delete" | "del" => return "Delete".to_string(),
        _ => {}
    }
    // Function keys: `f5` -> `F5`.
    if let Some(n) = lower.strip_prefix('f') {
        if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) {
            return format!("F{n}");
        }
    }
    // Single letter or digit -> uppercase.
    if raw.len() == 1 {
        return raw.to_ascii_uppercase();
    }
    // Fallback: title-case the first character, keep the rest.
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// Overlay a parsed `action -> binding` map onto an emulator's defaults table.
/// The defaults table is the canonical action set (and display order) for that
/// emulator; for each action the user's parsed binding wins, else the documented
/// default shows flagged `is_default: true`. All bindings are keyboard for now.
pub(crate) fn overlay_defaults(
    defaults: &[(HotkeyAction, &str)],
    parsed: &HashMap<HotkeyAction, String>,
) -> Vec<EmulatorHotkey> {
    defaults
        .iter()
        .map(|(action, default)| {
            let (binding, is_default) = match parsed.get(action) {
                Some(b) => (b.clone(), false),
                None => (default.to_string(), true),
            };
            EmulatorHotkey {
                action: *action,
                label: action.label().to_string(),
                binding,
                is_default,
                device: HotkeyDevice::Keyboard,
            }
        })
        .collect()
}

// ---- Per-emulator defaults tables -----------------------------------------
// Documented out-of-the-box keyboard bindings. The engine overlays the user's
// own config on top (parsed wins), so these only show for actions the user has
// not rebound — the common case, since most users never customize hotkeys.

/// RetroArch documented defaults (RetroArch's stock `input_*` keys).
pub(crate) const RETROARCH_DEFAULTS: &[(HotkeyAction, &str)] = &[
    (HotkeyAction::SaveState, "F2"),
    (HotkeyAction::LoadState, "F4"),
    (HotkeyAction::NextSlot, "F7"),
    (HotkeyAction::PrevSlot, "F6"),
    (HotkeyAction::Screenshot, "F8"),
    (HotkeyAction::Pause, "P"),
    (HotkeyAction::FastForwardHold, "L"),
    (HotkeyAction::FastForwardToggle, "Space"),
    (HotkeyAction::Rewind, "R"),
    (HotkeyAction::ToggleMenu, "F1"),
    (HotkeyAction::Exit, "Esc"),
    (HotkeyAction::Reset, "H"),
];

/// PCSX2 documented defaults (`[Hotkeys]`).
pub(crate) const PCSX2_DEFAULTS: &[(HotkeyAction, &str)] = &[
    (HotkeyAction::SaveState, "F1"),
    (HotkeyAction::LoadState, "F3"),
    (HotkeyAction::NextSlot, "F2"),
    (HotkeyAction::PrevSlot, "Shift + F2"),
    (HotkeyAction::Screenshot, "F8"),
    (HotkeyAction::Pause, "Space"),
    (HotkeyAction::FastForwardHold, "."),
    (HotkeyAction::FastForwardToggle, "Tab"),
    (HotkeyAction::ToggleFullscreen, "Alt + Return"),
    (HotkeyAction::ToggleMenu, "Esc"),
];

/// DuckStation documented defaults (`[Hotkeys]`).
pub(crate) const DUCKSTATION_DEFAULTS: &[(HotkeyAction, &str)] = &[
    (HotkeyAction::SaveState, "F2"),
    (HotkeyAction::LoadState, "F1"),
    (HotkeyAction::NextSlot, "F4"),
    (HotkeyAction::PrevSlot, "F3"),
    (HotkeyAction::Screenshot, "F10"),
    (HotkeyAction::Pause, "Space"),
    (HotkeyAction::FastForwardHold, "Tab"),
    (HotkeyAction::ToggleFullscreen, "F11"),
    (HotkeyAction::ToggleMenu, "Esc"),
];

/// Dolphin documented defaults (`[Hotkeys]`). Dolphin has no next/prev-slot
/// hotkey by default (it uses `Select State Slot N`), so only the selected-slot
/// save/load are surfaced.
pub(crate) const DOLPHIN_DEFAULTS: &[(HotkeyAction, &str)] = &[
    (HotkeyAction::SaveState, "F5"),
    (HotkeyAction::LoadState, "F7"),
    (HotkeyAction::Screenshot, "F9"),
    (HotkeyAction::Pause, "F10"),
    (HotkeyAction::ToggleFullscreen, "Alt + Return"),
    (HotkeyAction::Exit, "Esc"),
];

/// Mupen64Plus documented defaults (`[CoreEvents]` `Kbd Mapping <Action>`).
/// Mirrors the stock SDL-keysym values shipped in `mupen64plus.cfg`; the engine
/// overlays the user's own bindings on top. Increment-slot and fullscreen are
/// unbound out of the box (`0`), so they're not surfaced.
pub(crate) const MUPEN_DEFAULTS: &[(HotkeyAction, &str)] = &[
    (HotkeyAction::SaveState, "F5"),
    (HotkeyAction::LoadState, "F7"),
    (HotkeyAction::Screenshot, "F12"),
    (HotkeyAction::Pause, "P"),
    (HotkeyAction::FastForwardHold, "F"),
    (HotkeyAction::Reset, "F9"),
    (HotkeyAction::Exit, "Esc"),
];

/// Mednafen documented defaults (`command.<action> keyboard 0x0 <SDL scancode>`
/// in `~/.mednafen/mednafen.cfg`). Mirrors the stock scancodes; overlaid by the
/// user's own config.
pub(crate) const MEDNAFEN_DEFAULTS: &[(HotkeyAction, &str)] = &[
    (HotkeyAction::SaveState, "F5"),
    (HotkeyAction::LoadState, "F7"),
    (HotkeyAction::NextSlot, "="),
    (HotkeyAction::PrevSlot, "-"),
    (HotkeyAction::Screenshot, "F9"),
    (HotkeyAction::Pause, "Pause"),
    (HotkeyAction::FastForwardHold, "`"),
    (HotkeyAction::Reset, "F10"),
    (HotkeyAction::Exit, "F12"),
];

/// mGBA documented defaults. mGBA's `[shortcutKey]` section is empty out of the
/// box (its real bindings are compiled in) and stores any overrides as Qt
/// keycodes we don't translate yet, so this table carries the headline actions.
/// Save/load default to the slot-1 quick state (`F1` load, `Shift + F1` save),
/// mGBA's stable convention; the murkier fast-forward/fullscreen/reset bindings
/// vary and are intentionally omitted rather than guessed.
pub(crate) const MGBA_DEFAULTS: &[(HotkeyAction, &str)] = &[
    (HotkeyAction::SaveState, "Shift + F1"),
    (HotkeyAction::LoadState, "F1"),
];

// ---- Numeric key translation tables ---------------------------------------
// Several emulators store hotkeys as integers in one of two distinct numeric
// spaces. The *same integer* means different keys in each space, so each gets
// its own translator. Only the keys that actually appear in hotkey contexts are
// mapped (function keys, digits, printable ASCII, a few named keys); anything
// unmapped renders as `key <n>` rather than vanishing.

/// SDL *keysym* → display name (Mupen64Plus). Keysyms are ASCII for printables
/// (`112` = `p`, `47` = `/`), with `SDLK_F1 = 282` … `F12 = 293`. `0` is the
/// unbound sentinel → empty (falls back to the defaults table).
pub(crate) fn sdl_keysym_name(code: u32) -> String {
    match code {
        0 => String::new(),
        8 => "Backspace".to_string(),
        9 => "Tab".to_string(),
        13 => "Return".to_string(),
        27 => "Esc".to_string(),
        32 => "Space".to_string(),
        127 => "Delete".to_string(),
        282..=293 => format!("F{}", code - 281),
        33..=126 => ((code as u8) as char).to_ascii_uppercase().to_string(),
        n => format!("key {n}"),
    }
}

/// SDL *scancode* → display name (Mednafen). A different space from keysyms:
/// letters `4`–`29` = `A`–`Z`, digits `30`–`39` = `1`–`0`, `SDL_SCANCODE_F1 =
/// 58` … `F12 = 69`, plus the common punctuation/navigation scancodes. `0` ⇒
/// empty (unbound → default).
pub(crate) fn sdl_scancode_name(code: u32) -> String {
    match code {
        0 => String::new(),
        4..=29 => ((b'A' + (code - 4) as u8) as char).to_string(),
        30..=38 => ((b'1' + (code - 30) as u8) as char).to_string(),
        39 => "0".to_string(),
        40 => "Return".to_string(),
        41 => "Esc".to_string(),
        42 => "Backspace".to_string(),
        43 => "Tab".to_string(),
        44 => "Space".to_string(),
        45 => "-".to_string(),
        46 => "=".to_string(),
        47 => "[".to_string(),
        48 => "]".to_string(),
        49 => "\\".to_string(),
        51 => ";".to_string(),
        52 => "'".to_string(),
        53 => "`".to_string(),
        54 => ",".to_string(),
        55 => ".".to_string(),
        56 => "/".to_string(),
        58..=69 => format!("F{}", code - 57),
        70 => "PrintScreen".to_string(),
        71 => "ScrollLock".to_string(),
        72 => "Pause".to_string(),
        73 => "Insert".to_string(),
        74 => "Home".to_string(),
        75 => "PageUp".to_string(),
        76 => "Delete".to_string(),
        77 => "End".to_string(),
        78 => "PageDown".to_string(),
        79 => "Right".to_string(),
        80 => "Left".to_string(),
        81 => "Down".to_string(),
        82 => "Up".to_string(),
        n => format!("key {n}"),
    }
}

impl Engine {
    /// The hotkeys of the emulator that would launch this game, normalized to the
    /// canonical action set and overlaid on that emulator's defaults. Resolves the
    /// emulator exactly as launch does (explicit per-game override → platform
    /// default), so the panel matches what the user will actually press. Returns
    /// empty (panel hidden) for emulators we don't map or when no emulator
    /// resolves.
    pub async fn game_hotkeys(&self, game_id: &str) -> Result<Vec<EmulatorHotkey>> {
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
        let ctx = crate::adapters::ScanContext {
            rom_path: game.rom_path.clone(),
            platform: game.platform.clone(),
            flatpak_app_id: flatpak_app_id_of(&emulator),
            disc_key: None,
        };
        Ok(adapter.scan_hotkeys(&ctx).await.unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_section_scopes_and_splits_on_first_eq() {
        let text = "\
[Other]
SaveStateToSlot = nope
[Hotkeys]
SaveStateToSlot = Keyboard/F1
Save State/Save to Selected Slot = `F5`
; a comment
Device = DInput/0/Foo
[Trailing]
Ignored = yes
";
        let pairs = ini_section_pairs(text, "Hotkeys");
        assert!(pairs.contains(&("SaveStateToSlot".into(), "Keyboard/F1".into())));
        // Key containing a slash/space is preserved; value keeps its `=`-free rest.
        assert!(pairs.contains(&(
            "Save State/Save to Selected Slot".into(),
            "`F5`".into()
        )));
        // The same key in another section is not leaked in.
        assert!(!pairs.iter().any(|(_, v)| v == "nope"));
        assert!(!pairs.iter().any(|(k, _)| k == "Ignored"));
    }

    #[test]
    fn normalize_key_handles_each_casing() {
        assert_eq!(normalize_key("f2"), "F2");
        assert_eq!(normalize_key("F11"), "F11");
        assert_eq!(normalize_key("escape"), "Esc");
        assert_eq!(normalize_key("space"), "Space");
        assert_eq!(normalize_key("Period"), ".");
        assert_eq!(normalize_key("p"), "P");
        assert_eq!(normalize_key("Return"), "Return");
        assert_eq!(normalize_key("nul"), "");
        assert_eq!(normalize_key(""), "");
    }

    #[test]
    fn overlay_prefers_parsed_else_default() {
        let mut parsed = HashMap::new();
        parsed.insert(HotkeyAction::SaveState, "F9".to_string());
        let out = overlay_defaults(RETROARCH_DEFAULTS, &parsed);

        let save = out.iter().find(|h| h.action == HotkeyAction::SaveState).unwrap();
        assert_eq!(save.binding, "F9");
        assert!(!save.is_default);

        // An action absent from the parsed config falls back to the default.
        let load = out.iter().find(|h| h.action == HotkeyAction::LoadState).unwrap();
        assert_eq!(load.binding, "F4");
        assert!(load.is_default);

        // The table defines the canonical set/order regardless of the parsed map.
        assert_eq!(out.len(), RETROARCH_DEFAULTS.len());
    }

    #[test]
    fn sdl_keysym_translates_mupen_values() {
        // Verified Mupen values: F5/F7/F12 function keys, P/F ASCII, Esc.
        assert_eq!(sdl_keysym_name(286), "F5");
        assert_eq!(sdl_keysym_name(288), "F7");
        assert_eq!(sdl_keysym_name(290), "F9");
        assert_eq!(sdl_keysym_name(293), "F12");
        assert_eq!(sdl_keysym_name(112), "P");
        assert_eq!(sdl_keysym_name(102), "F");
        assert_eq!(sdl_keysym_name(27), "Esc");
        assert_eq!(sdl_keysym_name(47), "/");
        // 0 is the unbound sentinel; unknown codes degrade to `key <n>`.
        assert_eq!(sdl_keysym_name(0), "");
        assert_eq!(sdl_keysym_name(9999), "key 9999");
    }

    #[test]
    fn sdl_scancode_translates_mednafen_values() {
        // Verified Mednafen values: F1=58 so F5=62 … F12=69; named/punctuation.
        assert_eq!(sdl_scancode_name(62), "F5");
        assert_eq!(sdl_scancode_name(64), "F7");
        assert_eq!(sdl_scancode_name(66), "F9");
        assert_eq!(sdl_scancode_name(67), "F10");
        assert_eq!(sdl_scancode_name(69), "F12");
        assert_eq!(sdl_scancode_name(72), "Pause");
        assert_eq!(sdl_scancode_name(53), "`");
        assert_eq!(sdl_scancode_name(45), "-");
        assert_eq!(sdl_scancode_name(46), "=");
        assert_eq!(sdl_scancode_name(4), "A");
        assert_eq!(sdl_scancode_name(39), "0");
        assert_eq!(sdl_scancode_name(0), "");
        assert_eq!(sdl_scancode_name(9999), "key 9999");
    }
}
