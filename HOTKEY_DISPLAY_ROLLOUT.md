# Hotkey Display Rollout

**Status:** Research + planning only. No code changed. This document is the
implementation blueprint.

**Goal:** On the Game Details page, show a clean, read-only panel of the
*emulator hotkeys* (save state, load state, next/prev slot, screenshot, pause,
fast-forward, fullscreen, menu, exit, …) for the emulator that would actually
launch this game — its resolved default. The user opens a game, sees "while
you're playing, F2 = Save State, F4 = Load State" without leaving Arcadia.

---

## 1. Design principles (inherited from the codebase)

This feature is a near-exact structural twin of **Recall State** (savestate
discovery). It obeys the same house rules already encoded in the adapters:

1. **Discovery, never reimplementation.** We only *read and classify* the
   emulator's own config files. We never write them, never intercept input, and
   never change how the emulator behaves. (Same rule the savestate scanner and
   `controller.rs` follow — Arcadia documents state, it doesn't own input.)
2. **Resolve the emulator exactly as launch does.** A game's hotkeys are the
   hotkeys of the emulator that would boot it: explicit per-game override →
   platform default (`emulator_for_platform`). This is the identical resolution
   used by `Engine::list_save_states` and `game_supports_launch_state`.
3. **Flatpak-aware paths.** Config dirs are resolved through the existing
   `xdg_base(flatpak_app_id, config)` redirect so a Flatpak install reads
   `~/.var/app/<id>/config/...` and a native install reads `~/.config/...`.
4. **Honest about gaps.** If we can't parse an emulator's hotkeys (or it stores
   them as opaque numeric keycodes we haven't mapped), the panel is hidden for
   that emulator rather than showing garbage — exactly how Snes9x renders
   read-only Recall cards instead of pretending.
5. **Defaults overlay.** Most users never customize hotkeys, so their config
   omits the entry entirely (or writes `"nul"` / `Unset`). The panel must fall
   back to each emulator's *documented default* and overlay only the user's
   actual customizations on top. Without this, the panel is empty for the common
   case. (See §6.)

---

## 2. Coverage matrix — all 26 implemented adapters

Every adapter registered in `AdapterRegistry::builtin()`
(`core/dreamvault/src/adapters/mod.rs:186`) is accounted for. Each row gives the
hotkey config location, its format, the parse **tier**, a **disposition**
(rollout phase or why it's deferred), and **confidence** (how the format was
established). Items marked *Verified* were read off this machine on 2026-06-16;
*Documented* relies on the emulator's known config format and needs a real
config to confirm before coding; *Probe* needs investigation during
implementation.

### Parse tiers
- **A — human-readable INI/JSON.** Values are emulator-named keys (`F2`,
  `Keyboard/F1`, `` `Shift`&`F1` ``, `Ctrl+O`). Parse + light normalization.
- **B — numeric key/scancode.** Action names are readable but the *value* is an
  integer needing a translation table. Three distinct numeric spaces appear, so
  each needs its own table: **SDL keysym** (Mupen), **SDL scancode** (Mednafen),
  **Qt key/scancode** (mGBA, melonDS), **device-keycode pairs** (PPSSPP).
- **C — opaque / GTK-accelerator / compiled-default.** The real bindings live in
  compiled defaults or a hard-to-parse serialized blob; a defaults table carries
  the common case and override parsing is best-effort or skipped.
- **— (none).** Menu-driven emulator with no meaningful keyboard-hotkey set
  worth surfacing (typically `savestates: false`); panel stays hidden.

### The matrix

| Emulator | Adapter id | Config (native; Flatpak redirects via `xdg_base`) | Format | Tier | Disposition | Confidence |
|---|---|---|---|---|---|---|
| RetroArch | `retroarch` | `retroarch/retroarch.cfg` | top-level `input_save_state = "f2"` | A | **Phase 1** | Verified |
| PCSX2 | `pcsx2` | `PCSX2/inis/PCSX2.ini` `[Hotkeys]` | `SaveStateToSlot = Keyboard/F1` | A | **Phase 1** | Verified |
| DuckStation | `duckstation` | `duckstation/settings.ini` `[Hotkeys]` | `SaveSelectedSaveState = Keyboard/F2` | A | **Phase 1** | Verified |
| Dolphin | `dolphin` | `dolphin-emu/Hotkeys.ini` `[Hotkeys]` | `Save State/Save to Selected Slot = `` `F5` `` | A | **Phase 1** | Verified |
| Lime3DS | `lime3ds` | `lime3ds-emu/qt-config.ini` `[Shortcuts]` | `…\KeySeq=Ctrl+O` (Qt KeySeq strings) | A | **Phase 1** | Documented |
| Mesen | `mesen` | `Mesen2/settings.json` | JSON, named keys under input/shortcut keys | A | **Phase 1** | Documented |
| Mupen64Plus | `mupen64plus` | `mupen64plus/mupen64plus.cfg` `[CoreEvents]` | `Kbd Mapping Save State = 286` (SDL keysym) | B | **Phase 2** | Verified |
| Mednafen | `mednafen` | `~/.mednafen/mednafen.cfg` | `command.save_state keyboard 0x0 62` (SDL scancode) | B | **Phase 2** | Verified |
| mGBA | `mgba` | `mgba/config.ini` `[shortcutKey]` | Qt keycode ints (empty here ⇒ defaults) | B | **Phase 2** | Verified |
| melonDS | `melonds` | `melonDS/melonDS.toml` | `HK_FastForwardToggle = -1` (Qt scancode; `-1` unbound) | B | **Phase 2** | Verified |
| PPSSPP | `ppsspp` | `ppsspp/PSP/SYSTEM/controls.ini` `[ControlMapping]` | `Save State = 1-134` (device-keycode) | B | **Phase 2** | Documented |
| Snes9x | `snes9x` | `snes9x/snes9x.conf` `[Window State]` | `GTK_state_save_current = Unset` (GTK accel) | C | **Phase 3** | Verified |
| BlastEm | `blastem` | `blastem/blastem.cfg` `[bindings]` | `ui.save_state` keys (compiled defaults common) | C | **Phase 3** | Documented |
| Stella | `stella` | `stella/stellarc` | serialized `keymap`/`combomap` blob | C | **Phase 3** | Verified-absent |
| VICE | `vice` | `vice/sdl-vicerc` + `.vhk` hotkey files | VICE SDL hotkey grammar | C | **Phase 3** | Documented |
| Atari800 | `atari800` | `~/.atari800.cfg` | sparse; UI keys mostly compiled | C | **Phase 3** | Documented |
| openMSX | `openmsx` | `openmsx/settings.xml` + Tcl bindings | Tcl `bind` commands | C | **Phase 3** | Documented |
| Gearcoleco | `gearcoleco` | `gearcoleco/config.ini` | SDL; compiled defaults | C | **Phase 3** | Documented |
| MAME | `mame` | `mame/cfg/default.cfg` (XML) | `<input>` UI ports; `ui.cfg` | C | **Phase 3** | Documented |
| BigPEmu | `bigpemu` | `BigPEmu/BigPEmuConfig.bigpcfg` (JSON) | JSON input block | C | **Phase 3 (probe)** | Documented |
| Flycast | `flycast` | `flycast/emu.cfg` | **no keyboard-hotkey lines** — compiled defaults | C | **Phase 3 (defaults-only)** | Verified |
| RPCS3 | `rpcs3` | `rpcs3/*.yml` | menu-driven; `savestates:false` | — | **Deferred** (no hotkey surface) | Verified |
| xemu | `xemu` | `xemu/xemu.toml` | menu-driven; `savestates:false` | — | **Deferred** | Documented |
| Cemu | `cemu` | `Cemu/settings.xml` | menu-driven; `savestates:false` | — | **Deferred** | Documented |
| Vita3K | `vita3k` | `Vita3K/config.yml` | ImGui menu; `savestates:false` | — | **Deferred** | Documented |
| Ryujinx | `ryujinx` | `Ryujinx/Config.json` | menu-driven; `savestates:false` | — | **Deferred** | Documented |

**Reading the dispositions:**
- **Phase 1 (6):** readable INI/JSON. RetroArch, PCSX2, DuckStation, Dolphin
  verified; Lime3DS + Mesen need one real config each to confirm grammar.
- **Phase 2 (5):** numeric translation tables. Three table types
  (SDL keysym, SDL scancode, Qt) — Mednafen's action names are the cleanest
  (`command.save_state`), so it's a strong Phase-2 lead alongside Mupen.
- **Phase 3 (9):** defaults-dominant. Ship the per-emulator **defaults table**
  (high value, low effort) and add override parsing opportunistically. Flycast
  is confirmed defaults-only (its `emu.cfg` holds no keyboard hotkeys).
- **Deferred (5):** modern, menu-driven, no savestate model — the hotkey panel
  stays hidden (returns `[]`). Revisit only if users ask for fullscreen/
  screenshot/exit hints on these.

The empty-state already covers every not-yet-implemented emulator gracefully, so
phases ship independently and Deferred rows need no code.

---

## 3. Normalized model

Define a canonical action set so the UI is uniform regardless of which emulator
each game uses. These are the actions worth surfacing (the rest are noise):

```
SaveState, LoadState, NextSlot, PrevSlot, Screenshot, Pause,
FastForwardHold, FastForwardToggle, Rewind, ToggleFullscreen, ToggleMenu, Exit, Reset
```
(Fast-forward is split into hold/toggle — see the enum note below.)

Proposed Rust shape (new `core/dreamvault/src/hotkeys.rs`, mirroring
`save_states.rs`):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmulatorHotkey {
    pub action: HotkeyAction,   // canonical enum -> stable id for the UI
    pub label: String,          // "Save State", "Fast Forward (hold)"
    pub binding: String,        // normalized human string, e.g. "F2", "Shift + F1", "Tab"
    pub is_default: bool,       // true when sourced from our defaults table (not the user's config)
    pub device: HotkeyDevice,   // Keyboard | Controller (controller bindings shown secondarily)
}
```

`binding` is always a display string — we normalize each emulator's raw token
(`f2`, `Keyboard/F1`, `` `Shift`&`F1` ``, SDL keysym `286`) into a single
consistent form (`F2`, `Shift + F1`). Normalization rules live per-adapter.

**`HotkeyAction` is a `Vec`-friendly enum, not a unique key.** RetroArch binds
*two* fast-forward actions (hold + toggle) and most emulators expose per-slot
save/load alongside "selected slot". So the canonical set carries a variant tag
rather than assuming one binding per action:

```rust
pub enum HotkeyAction {
    SaveState, LoadState, NextSlot, PrevSlot, Screenshot,
    Pause, FastForwardHold, FastForwardToggle, Rewind,
    ToggleFullscreen, ToggleMenu, Exit, Reset,
}
```

Splitting `FastForwardHold`/`FastForwardToggle` (vs one `FastForward` + a
modifier field) keeps the UI a flat label→binding list with no special-casing —
each row is one action. Per-slot save/load (Dolphin `Save State Slot 1..10`) is
**deliberately excluded** from Phase 1: we show only the "selected slot" + the
next/prev-slot keys, which is the couch-friendly summary. Surfacing all ten slot
rows is noise; revisit only if requested.

---

## 4. Architecture / plumbing (mirror Recall State end-to-end)

Every layer already has a Recall-State analogue to copy. Concrete touch points:

### 4a. Adapter trait — `core/dreamvault/src/adapters/mod.rs`
Add a default-empty method to `EmulatorAdapter`:

```rust
/// Read this emulator's hotkey bindings from its own config, normalized to the
/// canonical action set. Read-only. Default empty: an adapter opts in by
/// knowing where its config lives and how bindings are named. `ctx` reuses
/// ScanContext for the Flatpak app-id redirect (rom_path/disc_key unused here).
async fn scan_hotkeys(&self, _ctx: &ScanContext) -> Result<Vec<EmulatorHotkey>, AdapterError> {
    Ok(Vec::new())
}
```

`ScanContext` already carries `flatpak_app_id`, which is all the reader needs;
no new context type required. (Could trim to a tiny `HotkeyContext` later, but
reuse keeps the dispatch identical to `scan_save_states`.)

### 4b. Per-adapter implementations
- **RetroArch** (`adapters/retroarch.rs`): parse `retroarch.cfg`, map the
  `input_<action>` keys, skip `"nul"`, attach `_btn` joypad bindings.
- **Standalone** (`adapters/standalone.rs`): add an optional `hotkeys` field to
  `Spec` (parallel to the existing `save_states: Option<SaveStateScheme>`),
  describing the config location (reuse `StateLocation` / `xdg_base`), the
  section name, and a per-emulator parse fn. Wire PCSX2, DuckStation, Dolphin in
  Phase 1; Mupen/mGBA/Snes9x in later phases. Emulators with `hotkeys: None`
  return empty.

A shared INI helper (`core/dreamvault/src/hotkeys.rs`) does the generic
`[Section]`-scoped `key = value` scan, since PCSX2/DuckStation/Dolphin/Mupen all
share that grammar; each adapter supplies only its key→action map + value
normalizer. This matches how `save_states.rs` exposes `discover_slots` +
per-adapter `parse_*` closures.

### 4c. Engine method — `core/dreamvault/src/hotkeys.rs`
```rust
impl Engine {
    pub async fn game_hotkeys(&self, game_id: &str) -> Result<Vec<EmulatorHotkey>> {
        // 1. get_game -> resolve emulator (override else emulator_for_platform)
        // 2. adapters.get(adapter_id)
        // 3. build ScanContext { flatpak_app_id: flatpak_app_id_of(&emulator), .. }
        // 4. adapter.scan_hotkeys(&ctx) -> overlay onto defaults table
    }
}
```
Copy the resolution block verbatim from `list_save_states`
(`save_states.rs:225-267`).

### 4d. Tauri command — `desktop/src-tauri/src/commands.rs`
```rust
#[tauri::command]
pub async fn game_hotkeys(state: State<'_, AppState>, game_id: String)
    -> CmdResult<Vec<EmulatorHotkey>> {
    map(state.engine.game_hotkeys(&game_id).await)
}
```
Register it in `desktop/src-tauri/src/main.rs` `generate_handler![]` (next to
`game_supports_launch_state`, line ~139).

### 4e. Frontend API — `desktop/src/api/commands.ts` + `desktop/src/api/types.ts`
- `types.ts`: add `EmulatorHotkey`, `HotkeyAction`, `HotkeyDevice` (mirroring the
  Rust serde shape — `#[serde(rename_all = "camelCase")]` or snake to match the
  existing convention used by `SaveState`).
- `commands.ts`: `gameHotkeys: (gameId) => invoke<EmulatorHotkey[]>("game_hotkeys", { gameId })`.

### 4f. UI — `desktop/src/views/GameDetail.tsx`
New `GameHotkeys({ gameId })` component, styled like `GameSaveStates`
(`GameDetail.tsx:722`). Rendered in the detail column near Recall State. Uses
`useQuery(["game-hotkeys", gameId], () => api.gameHotkeys(gameId))`. Hidden when
the list is empty. See §7.

---

## 5. Data flow (one diagram)

```
GameDetail.tsx  ── api.gameHotkeys(gameId) ──▶ invoke("game_hotkeys")
   ▲                                                     │
   │ EmulatorHotkey[]                                    ▼
   │                                   commands::game_hotkeys
   │                                                     │
   │                                   Engine::game_hotkeys(game_id)
   │                                   ├ resolve emulator (override|platform default)
   │                                   ├ adapters.get(adapter_id)
   │                                   ├ scan_hotkeys(ScanContext{flatpak_app_id})
   │                                   │     └ read+parse config file (read-only)
   └───────────────────────────────── └ overlay parsed bindings onto defaults table
```

---

## 6. Parsing strategy & the defaults problem

### Tier A grammar (Phase 1)
- **RetroArch:** top-level `input_<X> = "<key>"`. Map (verified against this
  machine's `retroarch.cfg`):

  | config token | canonical | value here |
  |---|---|---|
  | `input_save_state` | SaveState | `f2` |
  | `input_load_state` | LoadState | `f4` |
  | `input_state_slot_increase` | NextSlot | `f7` |
  | `input_state_slot_decrease` | PrevSlot | `f6` |
  | `input_screenshot` | Screenshot | `f8` |
  | `input_pause_toggle` | Pause | `p` |
  | `input_hold_fast_forward` | FastForward (hold) | `l` |
  | `input_toggle_fast_forward` | FastForward (toggle) | `space` |
  | `input_rewind` | Rewind | `r` |
  | `input_menu_toggle` | ToggleMenu | `f1` |
  | `input_exit_emulator` | Exit | `escape` |
  | `input_reset` | Reset | `h` |

  **Nuance — two fast-forward actions.** RetroArch exposes both a *hold*
  (`hold_fast_forward`) and a *toggle* (`toggle_fast_forward`). Surface both
  (label "Fast Forward (hold)" / "Fast Forward (toggle)") rather than collapsing
  — the canonical `FastForward` action needs a `modifier`/`variant` field, or
  promote both to distinct actions. Normalize values to title-case (`f2`→`F2`,
  `escape`→`Esc`, `space`→`Space`).

  **Controller hotkeys are unbound by default.** Every hotkey `_btn`/`_axis`/
  `_mbtn` sibling reads `"nul"` here (only the `input_player1_*_btn` *game-input*
  lines carry real pad bindings, which are not hotkeys). So for RetroArch the
  honest display is **keyboard-only**; don't render empty controller rows.
  `input_enable_hotkey*` is also `"nul"` (no modifier gate) — note it only if set.
- **PCSX2 / DuckStation:** `[Hotkeys]` section, `Action = Keyboard/F1`. Strip the
  `Keyboard/` device prefix; split combos on ` & ` and re-join as `+`. PCSX2 keys:
  `SaveStateToSlot`, `LoadStateFromSlot`, `NextSaveStateSlot`,
  `PreviousSaveStateSlot`, `Screenshot`, `TogglePause`, `HoldTurbo`/`ToggleTurbo`,
  `ToggleFullscreen`, `OpenPauseMenu`. DuckStation keys:
  `SaveSelectedSaveState`, `LoadSelectedSaveState`, `SelectNextSaveStateSlot`,
  `SelectPreviousSaveStateSlot`, `Screenshot`, `TogglePause`, `FastForward`,
  `ToggleFullscreen`, `OpenPauseMenu`.
- **Dolphin:** `[Hotkeys]` with compound `Category/Name = ` `` `F5` `` keys.
  Strip backticks; `&` = combo. Keys of interest: `Save State/Save to Selected
  Slot`, `Load State/Load from Selected Slot`, `General/Take Screenshot`,
  `General/Toggle Pause`, `General/Toggle Fullscreen`, `General/Stop`→Exit.
  Skip the `Device = ...` line.
- **Lime3DS** *(Documented — confirm against a real config first)*: Citra-derived
  `qt-config.ini` `[Shortcuts]`, percent-encoded compound keys
  (`Main%20Window\Load%20File\KeySeq=Ctrl+O`). URL-decode the key, read the
  `…\KeySeq=` value (already a Qt key-sequence string like `Ctrl+O`, `F2`); split
  combos on `+`. Limited savestate hotkeys — surface what exists (Load File,
  fullscreen, screenshot, frame-advance) and lean on defaults otherwise.
- **Mesen** *(Documented)*: Mesen 2 stores everything in `Mesen2/settings.json`.
  Hotkeys live under the input/shortcut-keys object as named keys → key-code or
  key-name fields. Parse with `serde_json`, map the shortcut names to canonical
  actions. JSON, so no INI helper — but still Tier A (human-readable). Confirm the
  exact JSON path on a populated config before coding.

### Tier B grammar (Phase 2)
- **Mupen64Plus:** `[CoreEvents]` `Kbd Mapping <Action> = <SDL keysym int>`.
  Verified full action set on this machine (the `Kbd Mapping` keys are stable
  human strings — that's the join key; the *value* is the integer to translate):

  | config key | canonical | keysym here | name |
  |---|---|---|---|
  | `Kbd Mapping Save State` | SaveState | `286` | F5 |
  | `Kbd Mapping Load State` | LoadState | `288` | F7 |
  | `Kbd Mapping Increment Slot` | NextSlot | `0` | *(unbound)* |
  | `Kbd Mapping Screenshot` | Screenshot | `293` | F12 |
  | `Kbd Mapping Pause` | Pause | `112` | P |
  | `Kbd Mapping Fast Forward` | FastForward | `102` | F |
  | `Kbd Mapping Stop` | Exit | `27` | Esc |
  | `Kbd Mapping Reset` | Reset | `290` | F9 |
  | `Kbd Mapping Fullscreen` | ToggleFullscreen | `0` | *(unbound)* |

  The SDL keysym space is tractable because the values cluster: **codes < 128 are
  ASCII** (48–57 = `0`–`9`, lowercase letters, `27` = Esc, `47` = `/`), and
  **282–293 = F1–F12** (`SDLK_F1 = 282`, so F5 = 286, F7 = 288, F9 = 290, F10 =
  291, F11 = 292, F12 = 293). A small `sdl_keysym_name(u32)` covering ASCII
  printables + Esc + F1–F12 + a few named keys handles every value seen; unknown
  codes render as `key <n>` rather than vanishing. `0` ⇒ unbound (fall to
  default / hide). `Joy Mapping <Action>` lines exist but are empty here →
  controller hotkeys unbound, keyboard-only display (as with RetroArch).
- **Mednafen** *(Verified)*: `~/.mednafen/mednafen.cfg`, lines
  `command.<action> keyboard 0x0 <SDL scancode>`. Action names are clean and map
  directly: `command.save_state`→SaveState, `command.load_state`→LoadState,
  `command.state_slot_inc`→NextSlot, `command.state_slot_dec`→PrevSlot,
  `command.take_snapshot`→Screenshot, `command.pause`→Pause,
  `command.fast_forward`→FastForward, `command.exit`→Exit. The value is an **SDL
  *scancode*** (not keysym — different table from Mupen): `SDL_SCANCODE_F1 = 58`,
  so F5 = 62, F7 = 64, F9 = 66, F12 = 69 (matches the verified values
  `save_state=62`, `load_state=64`, `take_snapshot=66`, `exit=69`). Need a
  `sdl_scancode_name(u32)` table (letters 4–29, digits 30–39, F1–F12 58–69, plus
  named keys). The `0x0` field is the modifier mask. Not Flatpak-redirected — base
  is `~/.mednafen` (a `StateLocation::Home`, already modelled).
- **melonDS** *(Verified)*: `melonDS/melonDS.toml`, keys `HK_<Action> = <int>`
  (`HK_FastForwardToggle`, `HK_FullscreenToggle`, `HK_Reset`, `HK_FrameStep`, …).
  Value is a **Qt key/scancode int**; `-1` ⇒ unbound (fall to default). melonDS
  exposes fewer of our canonical actions (no plain save/load-state hotkey in the
  `HK_` set — those are toolbar/menu actions), so coverage is partial: surface
  FastForward, Fullscreen, Reset, FrameStep and rely on defaults otherwise.
  Shares the Qt table with mGBA.
- **mGBA:** `[shortcutKey]` Qt keycodes — **section empty on this machine**, the
  common case, so the defaults table carries it entirely. A Qt::Key→name table is
  only needed once we parse overrides; defer until a populated config shows up.
  Shares the Qt table with melonDS.
- **PPSSPP** *(Documented)*: `PSP/SYSTEM/controls.ini` `[ControlMapping]`,
  `Save State = <device>-<keycode>` (e.g. `1-134`, comma-separated for multiple
  bindings). Device `1` = keyboard; the keycode is PPSSPP's NKCODE (Android-style
  key constants). Needs an NKCODE→name table; lower confidence, confirm against a
  real config.

### Tier C grammar (Phase 3) — defaults-dominant
These ship as a **defaults table first** (the high-value, low-effort win);
override parsing is added opportunistically where the format is cheap.
- **Snes9x:** `[Window State]` `GTK_state_save_current = Unset`. `Unset` ⇒ use
  default. Override values are GTK accelerator strings (`<Shift>F1`). snes9x-gtk's
  defaults table covers the common case.
- **Flycast** *(Verified defaults-only)*: `flycast/emu.cfg` carries **no**
  keyboard-hotkey lines — Flycast's keyboard shortcuts are compiled-in. Ship
  defaults only; no override parsing.
- **BlastEm:** `blastem.cfg` `[bindings]` with `ui.save_state` / `ui.load_state` /
  `ui.exit` keys when present; otherwise compiled defaults.
- **VICE / Atari800 / openMSX / Gearcoleco / MAME / BigPEmu:** each has a config
  but in a bespoke grammar (VICE `.vhk` hotkey files, openMSX Tcl `bind`, MAME
  XML `<input>`, BigPEmu JSON). Defaults table per emulator; parse overrides only
  if a maintainer adds the reader. BigPEmu's JSON is the most tractable if any are
  pursued (probe its `BigPEmuConfig.bigpcfg` input block).

### Deferred emulators (panel hidden)
RPCS3, xemu, Cemu, Vita3K, Ryujinx are menu-driven with `savestates: false` and
no meaningful keyboard-hotkey set. `scan_hotkeys` returns `[]` and they carry no
defaults table, so the engine yields an empty list and the UI hides the panel.
No code beyond the trait default is needed for these.

### Defaults table (required for all tiers)
A static, per-emulator `DEFAULT_HOTKEYS: &[(HotkeyAction, &str)]` capturing each
emulator's documented out-of-the-box bindings. Engine overlays user config on
top: parsed binding wins; otherwise show the default (flagged `is_default:
true`). This is what makes the panel useful for the 90% of users who never
rebind. Evidence this is mandatory: mGBA's `[shortcutKey]` section is empty and
Snes9x writes `Unset` — both are pure-defaults today.

---

## 7. Frontend component design

`GameHotkeys` — clone the structure of `GameSaveStates`:

```
┌ HOTKEYS ────────────────────────────────────────────┐
│  (emulator name, e.g. "RetroArch")                   │
│  ┌──────────────┬──────────────┐                     │
│  │ Save State   │ F2           │   ← two-column      │
│  │ Load State   │ F4           │     key/value rows  │
│  │ Next Slot    │ F7           │                     │
│  │ Screenshot   │ F8           │                     │
│  │ Pause        │ P            │                     │
│  │ Fast Forward │ L  (hold)    │                     │
│  │ Menu         │ F1           │                     │
│  │ Exit         │ Esc          │                     │
│  └──────────────┴──────────────┘                     │
│  Defaults shown where you haven't customized them.   │
└──────────────────────────────────────────────────────┘
```

- Static, non-interactive (display only) — no `Focusable` wrappers needed
  beyond what spatial-nav requires for scroll. Bindings render as `<kbd>`-style
  chips using the existing `glass`/`surface-2` classes.
- Hidden entirely when `gameHotkeys` returns `[]` (unsupported emulator).
- Optionally tag default-sourced rows subtly (e.g. dimmer) and surface the
  emulator's config-file path so power users know where to rebind. Keep it
  uncluttered — matches the page's existing density.
- Controller bindings (Tier-A `_btn`, etc.), if parsed, render as a secondary
  line or a toggle; keyboard is the primary view.

---

## 8. Rollout phases

All 26 adapters are assigned. Each phase ships independently; the empty-state
covers everything not yet implemented, so partial rollout never regresses.

| Phase | Scope | Emulators (adapter id) | Systems covered |
|---|---|---|---|
| **1** | Tier A readers + defaults + full plumbing + UI | `retroarch`, `pcsx2`, `duckstation`, `dolphin` (verified) → then `lime3ds`, `mesen` (confirm config) | All RetroArch retro + PS1/PS2/GC/Wii; then 3DS + NES |
| **2** | Tier B numeric tables (SDL keysym, SDL scancode, Qt, NKCODE) | `mupen64plus`, `mednafen`, `mgba`, `melonds`, `ppsspp` | N64, PCE/Saturn/VB/Lynx/WS/NGP/SMS/GG, GB/GBC/GBA, NDS, PSP |
| **3** | Tier C defaults tables (+ opportunistic override parse) | `snes9x`, `flycast`, `blastem`, `vice`, `atari800`, `openmsx`, `gearcoleco`, `mame`, `bigpemu` | SNES, Dreamcast, Genesis, C64, A5200, MSX, ColecoVision, Arcade, Jaguar |
| **—** | Deferred (panel hidden, no code) | `rpcs3`, `xemu`, `cemu`, `vita3k`, `ryujinx` | PS3, Xbox, Wii U, Vita, Switch |

**Effort shape per phase:**
- **Phase 1** is the full vertical slice (trait method, engine, command, types,
  UI) plus 4 verified readers; Lime3DS/Mesen are small follow-on readers once
  their configs are confirmed.
- **Phase 2** is mostly the three numeric translation tables (SDL keysym, SDL
  scancode, Qt key) — write once, reuse across emulators sharing a space.
- **Phase 3** is overwhelmingly **defaults tables** (a static slice per
  emulator); override parsers are optional add-ons. This is where the "90% of
  users never rebind" assumption pays off most.
- **Deferred** rows need nothing beyond the trait's default-empty `scan_hotkeys`.

Phase 1 alone is a complete, shippable feature.

---

## 9. Testing strategy

Follows the established pattern: pure parsers, unit-tested against **real
captured config snippets** (the samples in §2 are ready to use as fixtures).

- Per-emulator parse tests: feed a real `[Hotkeys]` block, assert the canonical
  map (e.g. PCSX2 `SaveStateToSlot = Keyboard/F1` → `{SaveState, "F1"}`; combo
  `Keyboard/Alt & Keyboard/Return` → `"Alt + Return"`).
- Defaults overlay test: config missing an action ⇒ default returned with
  `is_default: true`; config present ⇒ user value with `is_default: false`;
  `"nul"`/`Unset`/`0` ⇒ treated as missing.
- Numeric-table tests (Phase 2), one per space: SDL keysym `286 → "F5"`,
  `27 → "Esc"` (Mupen); SDL scancode `62 → "F5"`, `69 → "F12"` (Mednafen); Qt key
  `-1 → unbound` (mGBA/melonDS); NKCODE (PPSSPP). Keep the three tables separate —
  the same integer means different keys in each space.
- Flatpak redirect test: with a `flatpak_app_id`, the reader targets
  `~/.var/app/<id>/config/...` (reuse `xdg_base`, already tested indirectly).
- `Engine::game_hotkeys` resolution test (in-memory pool, temp config): override
  vs platform-default picks the right adapter — analogous to existing
  save-state engine tests.
- No filesystem writes anywhere — assert read-only by construction (parsers take
  `&str`, only the dir resolver touches disk, read-only).

---

## 10. Open questions / risks

1. **Defaults drift.** Emulator default hotkeys change between versions. The
   defaults table is a snapshot; mitigate by sourcing from the user's config
   first and only falling back to defaults. Acceptable: defaults are a hint, not
   a guarantee — the panel can footnote "defaults shown; your version may
   differ."
2. **Keycode tables are large.** SDL keysym and Qt::Key enums are big. Map only
   the keys that actually appear in hotkey contexts (function keys, digits,
   common modifiers, a handful of named keys) and render unknown codes as a raw
   fallback (`key 0x4A`) rather than omitting.
3. **Config may not exist** (emulator installed but never launched). Reader
   returns empty → engine serves pure defaults. Fine.
4. **Multiple config profiles** (e.g. PCSX2 per-game inis, RetroArch
   game/core-specific overrides). Phase 1 reads the global config only;
   per-game override configs are a documented future enhancement, not MVP.
5. **Controller bindings vs keyboard.** Many users play on a pad. Tier-A
   `_btn`/`Joy Mapping` give raw indices that are device-specific and not
   human-friendly (same hard problem `controller.rs` documents). Keyboard
   bindings are the reliable, legible win; controller display is best-effort and
   can be deferred.
6. **`ScanContext` reuse vs a dedicated type.** Reusing `ScanContext` keeps
   dispatch identical to savestates but carries unused `rom_path`/`disc_key`.
   Acceptable; revisit only if it causes confusion.

---

## 11. File-by-file change list (when implementation begins)

**Backend (core/dreamvault):**
- `src/hotkeys.rs` *(new)* — `EmulatorHotkey`/`HotkeyAction`/`HotkeyDevice`
  types, shared `[Section]` INI helper, per-emulator defaults tables, the numeric
  translation tables (`sdl_keysym_name`, `sdl_scancode_name`, `qt_key_name`,
  `nkcode_name` — added per phase), and `Engine::game_hotkeys`.
- `src/lib.rs` — `pub mod hotkeys;`.
- `src/adapters/mod.rs` — add `scan_hotkeys` default method to `EmulatorAdapter`;
  re-export hotkey types.
- `src/adapters/retroarch.rs` — implement `scan_hotkeys` (Phase 1).
- `src/adapters/standalone.rs` — add optional `hotkeys` to `Spec`; implement
  `scan_hotkeys`; wire by phase:
  - **P1:** `pcsx2`, `duckstation`, `dolphin`, `lime3ds`, `mesen`.
  - **P2:** `mupen64plus`, `mednafen`, `mgba`, `melonds`, `ppsspp`.
  - **P3 (defaults-first):** `snes9x`, `flycast`, `blastem`, `vice`, `atari800`,
    `openmsx`, `gearcoleco`, `mame`, `bigpemu`.
  - **Deferred (no change):** `rpcs3`, `xemu`, `cemu`, `vita3k`, `ryujinx` keep
    the default-empty trait method.
  Note: Mednafen's grammar (`command.<action> keyboard 0x0 <scancode>`) and
  Mesen's JSON aren't `[Section]` key=value, so they get bespoke line/JSON
  parsers rather than the shared INI helper; everything else can use it.

**Desktop bridge:**
- `desktop/src-tauri/src/commands.rs` — `game_hotkeys` command + import type.
- `desktop/src-tauri/src/main.rs` — register in `generate_handler![]`.

**Frontend:**
- `desktop/src/api/types.ts` — `EmulatorHotkey`, `HotkeyAction`, `HotkeyDevice`.
- `desktop/src/api/commands.ts` — `gameHotkeys`.
- `desktop/src/views/GameDetail.tsx` — `GameHotkeys` component + render it.

---

## 12. Reference anchors (for the implementer)

- Resolution + dispatch to copy: `core/dreamvault/src/save_states.rs:225` (`list_save_states`).
- Flatpak path redirect: `core/dreamvault/src/adapters/standalone.rs:96` (`xdg_base`), `StateLocation::dir` at `:78`.
- `Spec` struct + per-emulator definitions: `core/dreamvault/src/adapters/standalone.rs:344`.
- Trait + `ScanContext`: `core/dreamvault/src/adapters/mod.rs:103` / `:117`.
- UI twin to clone: `desktop/src/views/GameDetail.tsx:722` (`GameSaveStates`).
- Command registration block: `desktop/src-tauri/src/main.rs:103`.
- `flatpak_app_id_of(&emulator)` helper: used in `save_states.rs:264` (from `crate::saves`).

---

## Related future work

Emulators that ship with **no bindings at all** (snes9x writes everything
`Unset`; others ship empty/opaque configs) leave the display panel hidden because
there's nothing to read. A deferred follow-on — letting the user *set up* those
bindings from inside Arcadia (which would have Arcadia **write** emulator config,
crossing the read-only house rule) — is captured in `HOTKEY_SETUP_MODULE.md`.

---

## Appendix A — Verified key→action maps (copy-ready for the implementer)

Captured from this machine's configs on 2026-06-16. The **left column is the
stable join key** the parser matches on; the right is the value seen (proof the
grammar parses, and a sanity check for the defaults table).

**PCSX2** — `PCSX2/inis/PCSX2.ini` `[Hotkeys]`, value form `Keyboard/<Key>`,
combos joined with ` & `:

| config key | canonical | value here |
|---|---|---|
| `SaveStateToSlot` | SaveState | `Keyboard/F1` |
| `LoadStateFromSlot` | LoadState | `Keyboard/F3` |
| `NextSaveStateSlot` | NextSlot | `Keyboard/F2` |
| `PreviousSaveStateSlot` | PrevSlot | `Keyboard/Shift & Keyboard/F2` |
| `Screenshot` | Screenshot | `Keyboard/F8` |
| `TogglePause` | Pause | `Keyboard/Space` |
| `HoldTurbo` | FastForwardHold | `Keyboard/Period` |
| `ToggleTurbo` | FastForwardToggle | `Keyboard/Tab` |
| `ToggleFullscreen` | ToggleFullscreen | `Keyboard/Alt & Keyboard/Return` |
| `OpenPauseMenu` | ToggleMenu | `Keyboard/Escape` |

**DuckStation** — `duckstation/settings.ini` `[Hotkeys]`, identical
`Keyboard/<Key>` grammar:

| config key | canonical | value here |
|---|---|---|
| `SaveSelectedSaveState` | SaveState | `Keyboard/F2` |
| `LoadSelectedSaveState` | LoadState | `Keyboard/F1` |
| `SelectNextSaveStateSlot` | NextSlot | `Keyboard/F4` |
| `SelectPreviousSaveStateSlot` | PrevSlot | `Keyboard/F3` |
| `Screenshot` | Screenshot | `Keyboard/F10` |
| `TogglePause` | Pause | `Keyboard/Space` |
| `FastForward` | FastForwardHold | `Keyboard/Tab` |
| `ToggleFullscreen` | ToggleFullscreen | `Keyboard/F11` |
| `OpenPauseMenu` | ToggleMenu | `Keyboard/Escape` |

PCSX2 + DuckStation share one parser (split on `/` to drop the `Keyboard`
device, split on ` & ` for combos, re-join `+`). Only the key→action map differs.

**Dolphin** — `dolphin-emu/Hotkeys.ini` `[Hotkeys]`, compound `Category/Name`
keys, value backtick-wrapped, combos joined with `&`:

| config key | canonical | value here |
|---|---|---|
| `Save State/Save to Selected Slot` | SaveState | `` `F5` `` |
| `Load State/Load from Selected Slot` | LoadState | `` `F7` `` |
| `General/Take Screenshot` | Screenshot | `` `F9` `` |
| `General/Toggle Pause` | Pause | `` `F10` `` |
| `General/Toggle Fullscreen` | ToggleFullscreen | `` `Alt`&`Return` `` |
| `General/Stop` | Exit | `` `Escape` `` |

Skip the leading `Device = ...` line. Dolphin has no next/prev-slot hotkey by
default (it uses `Select State Slot N`); show "selected slot" only. Parser:
strip backticks, split on `&`, re-join `+`.

## Appendix B — Defaults table shape

One static slice per emulator; the engine overlays parsed user bindings on top
(parsed wins; else default with `is_default: true`). Example for RetroArch
(values are RetroArch's documented out-of-box keys, which happen to match §6):

```rust
const RETROARCH_DEFAULTS: &[(HotkeyAction, &str)] = &[
    (HotkeyAction::SaveState,         "F2"),
    (HotkeyAction::LoadState,         "F4"),
    (HotkeyAction::NextSlot,          "F7"),
    (HotkeyAction::PrevSlot,          "F6"),
    (HotkeyAction::Screenshot,        "F8"),
    (HotkeyAction::Pause,             "P"),
    (HotkeyAction::FastForwardHold,   "L"),
    (HotkeyAction::FastForwardToggle, "Space"),
    (HotkeyAction::Rewind,            "R"),
    (HotkeyAction::ToggleMenu,        "F1"),
    (HotkeyAction::Exit,              "Esc"),
    (HotkeyAction::Reset,             "H"),
];
```

Same shape for PCSX2/DuckStation/Dolphin/Mupen/mGBA/Snes9x, sourced from each
emulator's documented defaults. Overlay precedence is the only logic:
`user_binding.unwrap_or(default)`.
