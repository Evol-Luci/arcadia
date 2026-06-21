# Input Setup Module (hotkeys + per-system controllers)

**Status:** Research + implementation blueprint. No code yet. This is the *write*
counterpart to the shipped read-only Hotkey Display panel
(`HOTKEY_DISPLAY_ROLLOUT.md`, `core/dreamvault/src/hotkeys.rs`,
`adapters/standalone.rs`, `adapters/retroarch.rs`). It crosses an architectural
line (see §2), so it is its own feature with its own review bar.

**Scope (expanded).** Two write surfaces under one module and one write spine:

- **Part A — keyboard hotkeys.** Save/load/slot/screenshot/pause/etc. Global to
  an emulator. The simpler half; it proves the write machinery (backup, atomic
  write, running-guard, surgical edit) on low-risk ground. §3–§9.
- **Part B — controllers, per system.** The pad mapping for the *console being
  emulated*. **The target button set differs per system** (NES has 2 buttons;
  N64 has an analog stick + 4 C-buttons + Z; PS2 has two sticks + L3/R3; the
  Virtual Boy has *two* d-pads). This is the larger, harder half and the reason
  for the "one last pass." §10–§14.

**The end-state the user is steering toward:** Arcadia becomes the single place
to set up input for every system. The Controller page grows **gamepad profiles
that are assigned to a system**; the setup module is the mechanism that
*materializes* a profile into the config of whichever emulator launches that
system. Hotkeys, controller profiles, the existing app-nav profiles, and the
launch-time controller bridge all sit on one spine. §13 is the cohesion check.

> **What changed since the first sketch.** The display panel is shipped code, so
> Part A is a precise inverse of real readers (every reader has a line number to
> invert — see §12). Part B is genuinely new and is where this revision focuses.

---

## 1. Why this exists (the gap it fills)

**Hotkeys:** some emulators ship no bindings (snes9x writes everything `Unset`,
empty defaults table at `standalone.rs:663`), so the display panel has nothing to
show and hides itself. A "Set up hotkeys" button closes that loop.

**Controllers:** the deeper gap. Several emulators ship **no controller bindings
at all** until configured in their own GUI — most painfully Mednafen, whose
Virtual Boy gamepad (`vb.input.builtin.gamepad.*`) ships empty so *nothing
responds* until the user hand-configures it in-game (this is documented in
project memory as a real, recurring user pain point). Worse, each console needs a
*different* mapping: configuring "the controller" once is meaningless because the
N64 pad, the PS2 pad, and the Virtual Boy's twin-d-pad layout share almost
nothing. The user wants to do this per system, from Arcadia, across the board —
not just for the emulators that start empty, but for **all** of them, so Arcadia
is the canonical input-setup surface.

---

## 2. The architectural line this crosses (read first)

House rule #1 (encoded in `controller.rs`, the savestate scanner, the hotkey
*display* panel): **discover, never reimplement; read emulator config, never
write it, never intercept input.** This feature **writes emulator config.** A
deliberate, eyes-open departure, governed by:

- **Opt-in and user-initiated.** Never a binding the user didn't ask for.
- **Surgical, not generative.** Edit the one value/line that changes; leave every
  other byte intact. Never regenerate config from our model (it would drop
  everything we don't model). Crux for both parts — see §3 and §11.
- **Back up before writing.** Mirror the on-disk precedent (`GCPadNew.ini.bak.*`
  from the Dolphin setup work). Write a `.bak` before the first edit.
- **Write atomically.** temp-file + `rename` (Arcadia's own `config.rs:160` uses a
  plain write; emulator configs deserve better).
- **Respect a running emulator.** Many emulators rewrite their whole config on
  exit; writing under them clobbers our edit. Guard via the open `play_sessions`
  row (§6).

**Two write philosophies already exist in the codebase — reconcile them
explicitly (this is the key going-forward decision):**

1. **Surgical write into the user's own config** (what Part A proposes). Best for
   *persistent setup* the user wants to keep — "configure my SNES controller."
2. **Scratch config + point the emulator at it** via a config-dir flag (Mupen
   `--configdir`, RetroArch `--appendconfig`), never touching the user's config —
   the plan written into `controller.rs:9-20` for the *launch-time controller
   bridge*. Best for *per-game/per-launch overrides* that shouldn't be permanent.

**Decision for this module:** the user is asking to *set up* their controller
(persistent, "saved to a certain system"), so **Part B uses the surgical-write
philosophy** — the profile is materialized into the emulator's own config and
stays there. The scratch-config bridge remains the complementary *launch-time*
path for transient per-game overrides (future work, `controller.rs` TODO). Both
share the same backup + atomic-write + token-encoder spine; they differ only in
*where* the bytes land. The plan must keep them clearly labelled so neither
silently becomes the other.

---

## PART A — KEYBOARD HOTKEYS (the proven spine)

## 3. Central problem: surgical write-back

A reader is pure `&str -> HashMap`. A writer is **not** `HashMap -> String` (that
discards comments, ordering, unmodelled keys). It is `(&str, action, binding) ->
String` returning the original text with exactly one value changed (or one line
inserted). Three write shapes, mirroring the reader families:

- **3a. INI key=value in place** (PCSX2, DuckStation, Dolphin, Mupen, snes9x):
  reuse the section-tracking of `ini_section_pairs` (`hotkeys.rs:92`); replace the
  matched value preserving spacing; insert at section end if absent; append
  `[Section]` if the section is absent. Each is the inverse of an existing
  `*_HOTKEY_MAP` (read right-to-left).
- **3b. Flat whitespace lines** (Mednafen `command.<action> keyboard 0x0
  <scancode>`): inverse of `parse_mednafen_hotkeys` (`standalone.rs:524`).
- **3c. Value encoders** (inverse of normalization): display string → native
  token, per grammar.

| reader value parser | location | inverse encoder |
|---|---|---|
| `normalize_key` | `hotkeys.rs:119` | `denormalize_key` (per-emulator case) |
| `parse_device_combo` | `standalone.rs:390` | `"Shift + F1"` → `Keyboard/Shift & Keyboard/F1` |
| `parse_backtick_combo` | `standalone.rs:404` | → `` `Alt`&`Return` `` |
| `parse_gtk_accel` | `standalone.rs:549` | → `<Shift>F1` |
| `parse_mupen_keysym` | `standalone.rs:497` | `"F5"` → `286` (inverse `sdl_keysym_name`) |
| `parse_mednafen` scancode | `standalone.rs:524` | `"F5"` → `62` (inverse `sdl_scancode_name`) |

Numeric inverses (`sdl_keysym_name` `hotkeys.rs:294`, `sdl_scancode_name` `:313`)
are small; round-trip tests (`code(name(c)) == c`) come first.

## 4. "Apply recommended" — cheapest valuable version

A single button that writes a known-good set. For most emulators the **defaults
tables already in `hotkeys.rs`** (`:190`–`:282`) *are* the recommended set: run
each row through the §3c encoder and write. **Wrinkle:** ship-empty emulators
(snes9x, `defaults: &[]`) need a hand-authored `SNES9X_RECOMMENDED` table for the
writer, separate from the empty *display* table. So inverting an emulator that
already has a defaults table costs only encoder + writer; inverting a ship-empty
one also costs authoring a recommended set.

## 5. Plumbing (mirror `scan_hotkeys` end-to-end)

- **`hotkeys.rs`:** `denormalize_key`, `sdl_keysym_code`, `sdl_scancode_code`,
  `ini_set_value` (pure surgical editor), `backup_then_write` (the single
  disk-touch: `.bak` then temp+rename). Engine `set_game_hotkey` /
  `apply_recommended_hotkeys` (twins of `game_hotkeys` `hotkeys.rs:360`, copying
  its resolution + `ScanContext` build).
- **`HotkeyScheme`** (`standalone.rs:367`) gains `write: Option<HotkeyWriter>`
  (`{ recommended: &[...], set: fn(text, action, display) -> String }`). Reuse
  `HotkeyScheme::path` (`:378`) for the write target.
- **Trait** (`adapters/mod.rs:168`): `set_hotkey` + `apply_recommended_hotkeys`,
  default `Err(Unsupported)`. Standalone applies all edits in one read→edit→write
  (one backup per apply); RetroArch overrides directly (`retroarch.rs:154`).
- **Tauri/commands/types/frontend:** `set_game_hotkey`,
  `apply_recommended_hotkeys`; `GameHotkeys` (`GameDetail.tsx:813`) empty-state
  button (replace `if (list.length === 0) return null;` at `:819`) + confirm
  dialog + query invalidation.

## 6. Running-emulator guard

No central process registry. Launch records a `play_sessions` row
(`stats.rs:241`), `ended_at` set on exit (`:288`). **Use the open-session query:**
an emulator is in use if a row for its `emulator_id` has `ended_at IS NULL`.
Cheap, no new state; only knows about Arcadia-launched emulators, so pair it with
the confirm dialog. PID liveness (pids captured at `stats.rs:225`) is a stronger
fallback if needed.

## 7. UX

Two entry points: Game Details Hotkeys panel (empty → "Set up hotkeys"; populated
→ "Customize") and an emulator settings page. MVP is **Apply recommended** (no
capture needed); per-key capture is polish. Confirm dialog states plainly that
Arcadia will modify the emulator's own config and that a backup is saved.

## 8. Testing (Part A)

Round-trip encoders; surgical edit leaves all other bytes identical (extend the
cross-section guard from `hotkeys.rs:390`); insert/append paths; `backup_then_write`
leaves correct `.bak` and never truncates; running-guard refuses on open session;
no-writer adapters return `Unsupported`.

## 9. Build order (Part A)

1. Write infra (`backup_then_write`, `ini_set_value`, numeric inverses) + tests.
2. First writer = a clean reader (PCSX2 or DuckStation) — full vertical slice.
3. **snes9x** — the motivating case: author `SNES9X_RECOMMENDED`, `parse_gtk_accel`
   inverse, `[Shortcuts]` writer; ship "Apply recommended."
4. Remaining Tier-A/B writers opportunistically (Dolphin, Mupen, Mednafen,
   RetroArch).
5. Per-key capture UI as polish.

---

## PART B — PER-SYSTEM CONTROLLERS (the expansion)

## 10. Why controllers are categorically harder than hotkeys

Three differences drive the whole design and are the "make sure this works going
forward" core:

1. **The target is per *system*, not per emulator.** Hotkeys are global to an
   emulator (one `[Hotkeys]` block). Controller bindings are per emulated console
   and, in multi-system emulators, live in **per-system config namespaces**:
   - Mednafen: `vb.input.builtin.gamepad.*` (Virtual Boy), `psx.input.port1.*`
     (PS1), `pce.input.port1.*` (PC Engine) — *distinct sections per system in one
     file*. (Verified pattern; the VB namespace is in project memory.)
   - RetroArch: one RetroPad abstraction, but the *meaningful* buttons differ per
     system (SNES B/Y/X/A vs N64's odd map vs Genesis A/B/C); the labels the user
     sees must be the console's, even though the underlying `input_player1_*` keys
     are uniform.
   So the write target is keyed by **(adapter, system)**, a finer grain than
   hotkeys' per-adapter scheme.

2. **The target button *set* differs per system.** There is no universal pad.
   Each console needs its own canonical target layout (§11). A generic
   "A/B/X/Y/LB/RB" schematic (what `Controller.tsx` draws today) cannot express
   the N64's C-buttons or the Virtual Boy's two d-pads.

3. **Physical→emulator token translation is the genuinely hard part** (the reason
   `controller.rs:9` deferred the bridge). The browser Gamepad API gives W3C
   standard indices; emulators want device- and SDL-version-specific tokens
   (`SDL/0/Button S`, `evdev/0/SOUTH`, Mupen `button(7)`, RetroArch
   `input_player1_a_btn = "0"`, Mednafen `joystick 0x<guid> button_3`). **Project
   memory's hard-won lesson: stop guessing device strings/tokens — the only
   reliable path was the emulator's own click-to-detect.** Arcadia writing blind
   is the highest risk in this whole module. Mitigations in §12c.

## 11. The per-system target-controller model (new data)

Introduce a static catalogue: for each console, the buttons its controller
exposes. This is the spine of Part B and is reusable by the UI, the writers, and
the profile model.

```rust
// new: core/dreamvault/src/console_pads.rs (or extend platforms.rs)
pub struct ConsolePad {
    pub system: &'static str,             // platform id ("snes", "n64", "virtualboy")
    pub inputs: &'static [ConsoleInput],  // the buttons/sticks this console has
}
pub struct ConsoleInput {
    pub id: &'static str,     // canonical token: "a","b","c_up","z","l_stick"
    pub label: &'static str,  // UI label: "A", "C ▲", "Z", "Left Stick"
    pub kind: InputKind,
}
pub enum InputKind {
    Button,   // single digital press → captured as a W3C button index
    Dpad,     // a directional group; each direction is its own ConsoleInput
              //   (id "dpad_up"/"dpad_down"/…) so it maps to either a hat or a button
    Stick,    // an analog 2-axis group; each axis± is its own ConsoleInput
              //   (id "l_stick_x"/"l_stick_y"/…) so it maps to a W3C axis or button
    Trigger,  // analog shoulder; may report as axis OR button depending on pad
}
```

`Dpad`/`Stick` are *group* markers for the UI; the catalogue still enumerates one
`ConsoleInput` per discrete capturable direction/axis so every entry maps to exactly
one W3C descriptor. The encoder (§12c) decides axis-vs-button per device.

Drafted catalogue for the four spanning systems (2-button → dual-shoulder + analog):

```rust
const NES: &[ConsoleInput] = &[
    dpad(),  // dpad_up/down/left/right
    btn("a", "A"), btn("b", "B"),
    btn("start", "Start"), btn("select", "Select"),
];

const SNES: &[ConsoleInput] = &[
    dpad(),
    btn("b", "B"), btn("a", "A"), btn("y", "Y"), btn("x", "X"),
    btn("l", "L"), btn("r", "R"),
    btn("start", "Start"), btn("select", "Select"),
];

const N64: &[ConsoleInput] = &[
    dpad(),
    stick("stick", "Analog Stick"),   // stick_x_pos/neg, stick_y_pos/neg
    btn("a", "A"), btn("b", "B"),
    btn("c_up", "C ▲"), btn("c_down", "C ▼"),
    btn("c_left", "C ◀"), btn("c_right", "C ▶"),
    btn("l", "L"), btn("r", "R"), btn("z", "Z"),
    btn("start", "Start"),
];

const PS1: &[ConsoleInput] = &[   // DualShock target
    dpad(),
    stick("l_stick", "Left Stick"), stick("r_stick", "Right Stick"),
    btn("cross", "✕"), btn("circle", "○"),
    btn("square", "□"), btn("triangle", "△"),
    btn("l1", "L1"), btn("l2", "L2"), btn("r1", "R1"), btn("r2", "R2"),
    btn("l3", "L3"), btn("r3", "R3"),
    btn("start", "Start"), btn("select", "Select"),
];
```

(`dpad()`, `stick(id,label)`, `btn(id,label)` are tiny const helpers that expand
to the per-direction/per-axis `ConsoleInput`s; `l2`/`r2` are `Trigger` kind on
DualShock-class targets so analog-trigger pads round-trip.)

Representative layouts for the remaining systems (to finalize as each writer lands):

| System | Distinctive inputs (beyond a basic d-pad) |
|---|---|
| NES | A, B, Start, Select |
| SNES | B, A, Y, X, L, R, Start, Select |
| N64 | A, B, **C-up/down/left/right**, L, R, **Z**, Start, **analog stick** |
| GBA | A, B, L, R, Start, Select |
| Genesis | A, B, C (+ X, Y, Z, Mode on 6-button), Start |
| PS1 | △ ○ ✕ □, L1 L2 R1 R2, Start, Select (DualShock: + 2 sticks + L3 R3) |
| PS2 | DualShock2 (PS1 + analog buttons, 2 sticks, L3/R3) |
| GameCube | A, B, X, Y, **Z**, L, R, Start, **control stick**, **C-stick** |
| Dreamcast | A, B, X, Y, **analog L/R triggers**, Start, analog stick |
| Saturn | A B C, X Y Z, L R, Start |
| PC Engine | I, II, Run, Select (6-button: III–VI) |
| **Virtual Boy** | **two d-pads (L + R)**, A, B, L R triggers, Start, Select |

The Virtual Boy and N64 rows alone prove a per-system layout is mandatory — and
both are emulators the user already wrestled with (memory). The UI maps the
user's *physical* pad (already captured live in `Controller.tsx`) onto *these*
targets, one console at a time.

## 12. Controller write architecture

### 12a. The (adapter, system) write scheme
A controller-config scheme keyed by adapter **and** system:

```rust
struct ControllerScheme {
    base: HotkeyBase,            // reuse the resolver (XdgConfig / Home)
    rel_path: &'static str,      // emulator config file
    // map a console input id -> the config key for THIS system, e.g.
    // ("a", "vb.input.builtin.gamepad.a") for Mednafen VB.
    section_for: fn(system: &str) -> Option<SystemBinding>,
    set: fn(text, key, encoded_token) -> String,  // surgical writer (Part A spine)
}
```

For single-system standalones (snes9x, DuckStation, mGBA) the system is implicit.
For multi-system emulators (Mednafen, RetroArch) `section_for` selects the right
namespace/player block. RetroArch's keys are uniform but the *labels* come from
`ConsolePad`, so the same writer serves every RetroArch system.

Concrete namespace resolution (`section_for(system) -> key prefix`):

| Adapter | system | Namespace / block | Example key for input `a` |
|---|---|---|---|
| RetroArch | *any* | `[file is per-core but keys are uniform]` | `input_player1_a_btn` (label from `ConsolePad`) |
| Mednafen | virtualboy | `vb.input.builtin.gamepad.*` | `vb.input.builtin.gamepad.a` |
| Mednafen | psx | `psx.input.port1.gamepad.*` | `psx.input.port1.gamepad.cross` |
| Mednafen | nes | `nes.input.port1.gamepad.*` | `nes.input.port1.gamepad.a` |
| Mednafen | snes | `snes.input.port1.gamepad.*` | `snes.input.port1.gamepad.a` |
| Mupen64Plus | n64 | `[Input-SDL-Control1]` | `A Button = button(N)` |
| DuckStation | psx | `[Pad1]` | `Cross = SDL-0/FaceSouth` |
| snes9x | snes | joypad-1 INI block | per-button key |

`section_for` returns `None` for an (adapter, system) pair we don't yet support,
which the engine surfaces as `Unsupported` — the same honest non-answer the read
side gives. The `console_input_id → key` half of the map lives beside each
adapter; the `console_input_id → W3C descriptor` half is the user's profile; the
`W3C → native token` half is §12c. Three small maps, composed — none of them
guesses.

### 12b. Capture, don't type
Reuse the live Gamepad polling already in `Controller.tsx` (`navigator.getGamepads`,
pressed-index detection at `:78`). The setup flow per console input: "Press the
button for **C ▲**" → capture the W3C index/axis → store in the profile. No
free-text. This is the same interaction the live tester already implements;
Part B turns "show what's pressed" into "record what's pressed for this target."

### 12c. Physical (W3C) → emulator token — the hard inverse
Per-adapter encoder from `(w3c_index_or_axis, device_identity) -> native token`.
Three honesty rules, straight from the memory log:

1. **Read the real device identity at write time**, never guess. SDL exposes the
   joystick GUID/name; capture it so the written token names the *actual* device
   (`SDL/0/<name>`, evdev GUID, Mednafen `joystick 0x<guid>`). Guessing device
   strings is exactly what failed repeatedly (Dolphin SDL-vs-evdev saga).
2. **Prefer emulators with stable, documented token schemes first** (RetroArch's
   numeric `input_player1_a_btn`, Mupen's `button(N)`). Defer
   GUID/backend-fragile ones (Dolphin's evdev tokens) until the stable ones prove
   the pipeline.
3. **Always back up; offer "open the emulator's own pad config" as the escape
   hatch.** If Arcadia's write doesn't take, the user can still click-to-detect in
   the emulator — we never trap them.

This encoder is the controller analogue of §3c and is the single riskiest
component; it gets the most test coverage and the most conservative rollout.

#### The W3C canonical-index insight (why this is more tractable than the read side)
Capture happens through `navigator.getGamepads`, which reports the **W3C Standard
Gamepad** layout — a *fixed, spec-defined* index map regardless of the physical
pad. So a stored binding `btn:0` always means "the bottom face button," `axis:0-`
always means "left stick left," etc. We are not guessing the device's raw button
order; the browser already normalized it. The encoder's job is therefore the much
narrower one of mapping these **canonical** indices to each emulator's token
grammar, with device *identity* (not button order) read live for the few schemes
that name the device.

W3C Standard Gamepad reference (the only indices we capture):

| W3C | Meaning | W3C | Meaning |
|---|---|---|---|
| `btn:0` | bottom face (✕/A/B-snes) | `btn:9` | Start |
| `btn:1` | right face (○/B/A-snes) | `btn:10` | L3 (left stick click) |
| `btn:2` | left face (□/X/Y-snes) | `btn:11` | R3 (right stick click) |
| `btn:3` | top face (△/Y/X-snes) | `btn:12` | D-pad up |
| `btn:4` | L1 / left shoulder | `btn:13` | D-pad down |
| `btn:5` | R1 / right shoulder | `btn:14` | D-pad left |
| `btn:6` | L2 / left trigger | `btn:15` | D-pad right |
| `btn:7` | R2 / right trigger | `axis:0±` | left stick X (− left) |
| `btn:8` | Select / Back | `axis:1±` | left stick Y (− up) |
| `btn:16` | Guide (often absent) | `axis:2±`/`axis:3±` | right stick X/Y |

Note the face-button *meaning* differs per console (SNES B is the bottom button,
PS1 ✕ is the bottom button) — that remapping is the user's capture choice recorded
per `ConsoleInput`, not the encoder's concern. The encoder only translates
"`btn:0` on device D → native token."

#### Per-emulator encoder grammars (priority order)
Each adapter implements `encode(w3c: &str, dev: &DeviceIdentity) -> Option<String>`.

| Emulator | System(s) | Button form | Axis form | Names device? |
|---|---|---|---|---|
| **RetroArch** | all (`input_playerN_<btn>_btn = N`) | the raw SDL button number | `_axis = +N`/`-N` | no — per-port `input_device` GUID set once |
| **Mupen64Plus** | n64 | `button(N)` | `axis(N+,N-)` | yes — `[Input-SDL-Control1]` keys, device by SDL name/idx |
| **Mednafen** | vb, psx, … | `joystick 0x<guid> 0x<btnbit>` | `joystick 0x<guid> 0x<axisbit>` | **yes — GUID embedded in every token** |
| **snes9x** | snes | port/button INI keys | (digital pad) | yes — joypad id |
| **DuckStation** | psx | `SDL-N/<Name>` (e.g. `FaceSouth`) | `SDL-N/<Axis>±` | yes — `SDL-N` index |
| **Dolphin** *(last)* | gc, wii | evdev `SOUTH/EAST/…` | `Axis N±` | yes — evdev `Device =` line |

Two grammar tiers fall out: **(A) index-stable, device-light** (RetroArch — the W3C
number maps almost 1:1 to the SDL button number; device pinned once via the port's
GUID) and **(B) device-embedded** (Mednafen/Dolphin — every token carries device
identity, so a wrong identity silently kills *all* bindings). Tier A ships first to
prove the pipeline; Tier B only after a real round-trip. Mednafen's GUID-per-token
is exactly the format the **read** side already parses in `parse_mednafen_hotkeys`
(`standalone.rs:524`), so its encoder is the byte-inverse of code we already trust.

#### `DeviceIdentity`, captured not guessed
At apply-time (not capture-time) Arcadia reads the live SDL view of the assigned
pad — index, name, and GUID — and threads it into the encoder. This is the one
runtime fact the profile deliberately does **not** store (it can change between
sessions / replugs). Source of truth options, cheapest first: the value the
emulator's own config already names (read it back, reuse verbatim), else an SDL
probe like the controller round-4 work in memory. If identity can't be resolved,
the encoder returns `None` and the UI falls back to the §12c-rule-3 escape hatch
rather than writing a guess.

### 12d. Plumbing
Same shape as Part A: `ControllerScheme` on the standalone `Spec`; trait methods
`set_controller_binding(ctx, system, input_id, captured)` and
`apply_recommended_controller(ctx, system)`; engine twins; Tauri commands;
frontend. The write target reuses `HotkeyScheme::path` resolution and the
`backup_then_write` spine — nothing new touches disk.

## 13. Cohesion check — how it all fits (the "going forward" verification)

This is what the user asked to confirm. The pieces and how they connect:

- **`ConsolePad` catalogue (§11)** — the per-system target layouts. New, static,
  shared by UI + writers + profiles. *Single source of truth for "what buttons
  does this console have."*
- **Gamepad profile — separate type (decision locked).** Today
  `ControllerProfile` (`config.rs:88`) is `logical-action -> physical-label` over
  a fixed app-nav `ACTIONS` list (`Controller.tsx:38`) — an *app-navigation*
  concept. The user wants *per-system* console mappings. **Decision: add a new
  `SystemControllerProfile` type rather than overloading `ControllerProfile`.**

  ```rust
  // config.rs
  pub struct SystemControllerProfile {
      pub id: String,
      pub name: String,
      pub system: String,                    // platform id; keys constrained to its ConsolePad
      pub bindings: BTreeMap<String, String>, // console_input_id -> W3C descriptor ("btn:0","axis:1-")
  }
  // ControllerConfig gains:
  //   pub system_profiles: Vec<SystemControllerProfile>,
  //   pub system_assignments: BTreeMap<String, String>, // system_id -> profile_id (active per system)
  ```

  Binding *values* are compact, device-agnostic **W3C descriptors** (`btn:N`,
  `axis:N+`/`axis:N-`) — the standard-gamepad index, not a native token and not a
  device GUID. The device identity (SDL GUID/name) is read **live at apply-time**
  by the §12c encoder, never stored in the profile (memory lesson: identity is
  captured, not guessed). Four reasons the separate type wins over
  `system: Option<String>` on the existing struct:
  1. **Different lifecycle** — `ControllerProfile` is advisory app-nav state;
     `SystemControllerProfile` is the materialization source for emulator config.
  2. **Constrained key space** — system-profile keys are limited to that system's
     `ConsolePad` input ids; app-nav keys are the fixed `ACTIONS` list. One struct
     would conflate two disjoint, separately-validated key vocabularies.
  3. **Needs a per-system assignment map** (`system_assignments`) that the app-nav
     profile model has no concept of.
  4. **No sentinel branching** — `Option<String>` would force `if system.is_some()`
     forks through every CRUD/read path anyway; two types make the branch a type.
- **Profile → emulator config = the setup module (Part B).** Assigning a profile
  to a system and hitting "Apply" runs each binding through the §12c encoder and
  surgically writes it into the config of the emulator that launches that system
  (resolved exactly like `game_hotkeys` does: override → `emulator_for_platform`).
- **Controller page** gains: per-system profile creation (pick a system → the
  `ConsolePad` layout drives the capture UI), assign-to-system, and "Apply to
  emulator." The existing live tester (`Controller.tsx` `Gamepad`) becomes the
  capture surface. The existing Compatibility panel (HIDAPI workaround) and
  app-nav profiles stay as-is.
- **Launch-time bridge (future, `controller.rs` TODO)** — the *other* write
  philosophy (scratch config + `--appendconfig`/`--configdir`). It consumes the
  **same** `SystemControllerProfile` and the **same** three-map composition
  (§12a key map · profile W3C bindings · §12c encoder). The *only* difference is
  the write destination and lifetime: the setup module's surgical writer targets
  the user's real config file (persistent); the bridge's writer emits a throwaway
  fragment into a temp `--configdir` consumed for one launch (transient). Both
  call the identical `encode(w3c, dev)` — so an encoder proven by the setup module
  is automatically correct for the bridge. The fork is a *destination* parameter,
  never a second data model or a second encoder. Concretely: factor the writer as
  `materialize(profile, system, dev) -> Vec<(key, token)>` (pure, shared), then
  two thin sinks — `write_surgical(path, pairs)` vs `write_fragment(tmpdir, pairs)`.

**Does the plan hold going forward? Yes, if these invariants are kept:**
1. Profiles are the source of truth; emulator config is a *materialization* of a
   profile, never the canonical store.
2. Target layouts live in one `ConsolePad` catalogue, never hardcoded per writer.
3. The write spine (backup + atomic + surgical + running-guard) is shared by
   hotkeys, controller setup, and the future bridge.
4. Device identity is *captured*, never guessed (the memory lesson).
5. Part A ships and proves the spine before Part B's harder encoders land on it.

## 14. Build order (Part B, after Part A)

1. `ConsolePad` catalogue for the headline systems (start NES/SNES/N64/PS1 — span
   the variety: 2-button, shoulder buttons, analog+C-buttons, dual-shoulder).
2. Add `SystemControllerProfile` + `system_profiles`/`system_assignments` to
   `ControllerConfig`; migrate the Controller page to per-system capture using the
   live tester (W3C descriptors as binding values).
3. First controller writer = **RetroArch** (uniform stable tokens, serves the
   most systems) — proves the W3C→token encoder on the safest grammar.
4. **Mednafen Virtual Boy** — the motivating empty-config case; per-system
   namespace writer (`vb.input.*`). Resolves the recurring memory pain point.
5. Remaining standalones by token-scheme stability (Mupen, snes9x, DuckStation …;
   Dolphin's evdev/GUID tokens last).
6. Wire the same profiles into the launch-time bridge (separate, later).

---

## 15. Open questions / risks

1. **Config rewritten on exit** — running guard (§6) + backup.
2. **We don't model every key/button** — surgical writer must leave unmodelled
   lines byte-identical (test, extending `hotkeys.rs:390`'s cross-section guard).
3. **Device-token translation is fragile** (§12c) — capture real identity, prefer
   stable schemes, always keep the emulator's own click-to-detect as escape hatch.
   This is the single biggest risk; treat each adapter's encoder as unproven until
   round-tripped against a real device.
4. **Profile model fork** — *resolved* (§13): a separate `SystemControllerProfile`
   type, not an evolved `ControllerProfile`. Disjoint key spaces, distinct
   lifecycle, and a per-system assignment map made the new type the right call.
5. **Multi-controller / player 2+** — Part B should bind player 1 first; the
   `section_for` map and profile model must leave room for `portN` without a
   redesign.
6. **Wii / hybrid input** (Wiimote+nunchuk vs classic vs GC) — genuinely
   multi-modal; defer, document, don't pretend a single layout fits.
7. **Combo/modifier canonicalization** (Part A) and **axis vs button capture
   ambiguity** (Part B, e.g. analog triggers reported as buttons) — pick canonical
   forms in the encoders and test re-reading our own writes.
8. **Flatpak vs native path drift** — write the same file the reader reads
   (`HotkeyScheme::path` / `xdg_base`).

---

## 16. File-by-file change list (when implementation begins)

**Part A (backend):** `src/hotkeys.rs` (`denormalize_key`, `sdl_*_code`,
`ini_set_value`, `backup_then_write`, engine `set_game_hotkey` /
`apply_recommended_hotkeys`, `SNES9X_RECOMMENDED`); `adapters/mod.rs` (`set_hotkey`
+ `apply_recommended_hotkeys`, default `Unsupported`); `adapters/standalone.rs`
(`write` on `HotkeyScheme`, per-emulator `write_*_hotkeys`); `adapters/retroarch.rs`.

**Part B (backend):** `src/console_pads.rs` (new — `ConsolePad` catalogue);
`config.rs` (new `SystemControllerProfile`; `system_profiles` +
`system_assignments` on `ControllerConfig`); `controller.rs` (per-system profile
CRUD + materialize); `adapters/mod.rs` (`set_controller_binding` /
`apply_recommended_controller`); `adapters/standalone.rs` (`ControllerScheme` on
`Spec`, per-(adapter,system) writers, W3C→token encoders);
`adapters/retroarch.rs`.

**Desktop bridge:** `commands.rs` + `main.rs` — `set_game_hotkey`,
`apply_recommended_hotkeys`, `set_controller_binding`,
`apply_recommended_controller`, per-system profile commands.

**Frontend:** `api/{types,commands}.ts`; `views/GameDetail.tsx` (`GameHotkeys`
setup button); `views/Controller.tsx` (per-system profile capture reusing the live
tester, assign-to-system, apply-to-emulator).

---

## 17. Reference anchors

- Readers to invert (Part A): `parse_device_combo` (`standalone.rs:390`),
  `parse_backtick_combo` (`:404`), `parse_gtk_accel` (`:549`), `parse_mupen_keysym`
  (`:497`), `parse_mednafen_hotkeys` (`:524`), `parse_retroarch_hotkeys`
  (`retroarch.rs:188`), `normalize_key` (`hotkeys.rs:119`).
- Numeric tables to invert: `sdl_keysym_name` (`hotkeys.rs:294`), `sdl_scancode_name`
  (`:313`).
- INI logic to reuse for surgical edits: `ini_section_pairs` (`hotkeys.rs:92`) + its
  cross-section test (`:390`).
- Defaults tables (double as recommended sets): `hotkeys.rs:190`–`282`.
- Path resolution for write targets: `HotkeyScheme::path` (`standalone.rs:378`),
  `xdg_base` (`:101`).
- Engine method to twin: `Engine::game_hotkeys` (`hotkeys.rs:360`).
- Trait defaults to twin: `scan_hotkeys` (`adapters/mod.rs:168`).
- Running-state source: open `play_sessions` row (`stats.rs:241`/`:288`); live pid
  (`stats.rs:225`).
- **Controller foundations:** `ControllerProfile` / `ControllerConfig`
  (`config.rs:88`/`:55`); profile CRUD (`controller.rs:75`-`110`); the
  controller-bridge plan + index-translation problem (`controller.rs:9-29`); live
  capture + pad schematic + app-nav `ACTIONS` (`Controller.tsx:38`,`:71`,`:256`);
  per-game launch env channel `LaunchContext.env` (`adapters/mod.rs:83`).
- Per-system controller-namespace precedent: Mednafen `vb.input.builtin.gamepad.*`
  (project memory — Virtual Boy controller config, resolved 2026-06-16).
- Config-write precedents: `AppConfig::save` (`config.rs:157`); Dolphin
  `GCPadNew.ini.bak.*` setup + `apply_sdl_hidapi_workaround` (`standalone.rs:1309`).
</content>
