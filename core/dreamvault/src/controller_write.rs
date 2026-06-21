//! Materializing per-system controller profiles into emulator config.
//!
//! This is the write side of the input-setup module and the inverse of the
//! read-only hotkey scanners. A [`crate::config::SystemControllerProfile`] stores
//! bindings as device-agnostic **W3C Standard Gamepad** descriptors (`btn:N`,
//! `axis:N+`/`axis:N-`) captured from the live pad in the shell. To make those
//! bindings real, each emulator needs a per-adapter *encoder* that turns a W3C
//! descriptor into that emulator's native token, plus a *surgical* writer that
//! edits one config line in place (never a whole-file rewrite — that would drop
//! comments, ordering, and keys we don't model).
//!
//! The two halves are kept pure here so the riskiest logic — the token
//! translation that has burned this project before ("stop guessing device
//! strings; the W3C layout is already normalized, so map indices, not raw
//! buttons") — is fully unit-tested before it ever touches a real config file.
//!
//! RetroArch ships first: its `retroarch.cfg` joypad keys are flat
//! `input_playerN_<name>_btn = "N"` / `_axis = "±N"` lines with a stable,
//! documented RetroPad abstraction, so the W3C index maps almost 1:1. Inputs
//! whose console→RetroPad mapping is core-config-dependent (N64 C-buttons) are
//! deliberately left unencoded rather than guessed — the core's own remap UI is
//! the escape hatch.

/// A parsed W3C Standard Gamepad descriptor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum W3cInput {
    /// A button index (`btn:5`).
    Button(u32),
    /// An axis half (`axis:1-` => index 1, negative).
    Axis(u32, AxisSign),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSign {
    Pos,
    Neg,
}

impl AxisSign {
    fn glyph(self) -> char {
        match self {
            AxisSign::Pos => '+',
            AxisSign::Neg => '-',
        }
    }
}

/// Parse a stored W3C descriptor. Returns `None` for anything malformed so a
/// corrupt profile entry is skipped rather than written as garbage.
pub fn parse_w3c(desc: &str) -> Option<W3cInput> {
    let desc = desc.trim();
    if let Some(rest) = desc.strip_prefix("btn:") {
        return rest.parse::<u32>().ok().map(W3cInput::Button);
    }
    if let Some(rest) = desc.strip_prefix("axis:") {
        let (num, sign) = if let Some(n) = rest.strip_suffix('+') {
            (n, AxisSign::Pos)
        } else if let Some(n) = rest.strip_suffix('-') {
            (n, AxisSign::Neg)
        } else {
            return None;
        };
        return num.parse::<u32>().ok().map(|i| W3cInput::Axis(i, sign));
    }
    None
}

/// One resolved config assignment: the full config key and its quoted value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub key: String,
    pub value: String,
}

/// Map a console input id to its RetroPad config-key base (the `<name>` in
/// `input_playerN_<name>_btn`). `None` means the console→RetroPad mapping is not
/// stable enough to write blind (e.g. N64 C-buttons depend on the core's analog
/// vs c-button remap mode) — the caller surfaces these as unencoded.
fn retropad_base(input_id: &str) -> Option<&'static str> {
    Some(match input_id {
        "dpad_up" => "up",
        "dpad_down" => "down",
        "dpad_left" => "left",
        "dpad_right" => "right",
        "start" => "start",
        "select" => "select",
        // RetroPad is modeled on the SNES face layout; NES/SNES map 1:1.
        "a" => "a",
        "b" => "b",
        "x" => "x",
        "y" => "y",
        "l" | "l1" => "l",
        "r" | "r1" => "r",
        "l2" => "l2",
        "r2" => "r2",
        "l3" => "l3",
        "r3" => "r3",
        // PlayStation faces — the documented libretro PSX RetroPad mapping.
        "cross" => "b",
        "circle" => "a",
        "square" => "y",
        "triangle" => "x",
        // N64 Z is RetroPad L2 by the mupen64plus_next default.
        "z" => "l2",
        // Left analog stick (N64 single stick + DualShock left stick).
        "stick_left" | "l_stick_left" => "l_x_minus",
        "stick_right" | "l_stick_right" => "l_x_plus",
        "stick_up" | "l_stick_up" => "l_y_minus",
        "stick_down" | "l_stick_down" => "l_y_plus",
        // Right analog stick (DualShock).
        "r_stick_left" => "r_x_minus",
        "r_stick_right" => "r_x_plus",
        "r_stick_up" => "r_y_minus",
        "r_stick_down" => "r_y_plus",
        _ => return None,
    })
}

/// Encode one (console input, captured W3C descriptor) pair into a RetroArch
/// `retroarch.cfg` assignment for the given player (1-based). Returns `None` when
/// the input has no stable RetroPad mapping, leaving it for the core's remap UI.
pub fn encode_retroarch(player: u8, input_id: &str, w3c: W3cInput) -> Option<Assignment> {
    let base = retropad_base(input_id)?;
    let (suffix, value) = match w3c {
        W3cInput::Button(n) => ("btn", format!("{n}")),
        W3cInput::Axis(n, sign) => ("axis", format!("{}{}", sign.glyph(), n)),
    };
    Some(Assignment {
        key: format!("input_player{player}_{base}_{suffix}"),
        value: format!("\"{value}\""),
    })
}

/// Turn a whole profile into the set of RetroArch assignments to write. Skips
/// malformed descriptors and inputs without a stable RetroPad mapping; the
/// returned `unencoded` list names the latter so the UI can point the user at the
/// core's own remap for them.
pub fn materialize_retroarch(
    player: u8,
    bindings: &std::collections::BTreeMap<String, String>,
) -> (Vec<Assignment>, Vec<String>) {
    let mut out = Vec::new();
    let mut unencoded = Vec::new();
    for (input_id, desc) in bindings {
        match parse_w3c(desc).and_then(|w| encode_retroarch(player, input_id, w)) {
            Some(a) => out.push(a),
            None => unencoded.push(input_id.clone()),
        }
    }
    (out, unencoded)
}

/// Surgically set `key = "value"` in a flat (sectionless) config such as
/// `retroarch.cfg`: replace the value of an existing `key = ...` line in place,
/// preserving every other byte; append a new line if the key is absent. Pure over
/// the text so it is unit-tested without touching disk; the atomic backup+write
/// to the real file happens at the IO boundary in the caller.
pub fn set_flat_key(text: &str, key: &str, quoted_value: &str) -> String {
    let mut replaced = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| {
            if replaced {
                return line.to_string();
            }
            let trimmed = line.trim_start();
            // match `key` followed by optional spaces then `=`
            if let Some(rest) = trimmed.strip_prefix(key) {
                if rest.trim_start().starts_with('=') {
                    replaced = true;
                    let indent = &line[..line.len() - trimmed.len()];
                    return format!("{indent}{key} = {quoted_value}");
                }
            }
            line.to_string()
        })
        .collect();
    if !replaced {
        lines.push(format!("{key} = {quoted_value}"));
    }
    let mut joined = lines.join("\n");
    // Preserve a trailing newline if the original had one.
    if text.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

// ── Mupen64Plus (Tier B) ────────────────────────────────────────────────────
//
// RetroArch (above) is Tier A: its config speaks the *normalized* W3C index, so
// `btn:9` is written as `9`. Mupen64Plus's SDL input plugin is Tier B — in manual
// mode it reads *raw SDL-joystick* element indices, which differ from the
// normalized layout (e.g. this project's Xbox pad reports Start as joystick
// `button(11)`, not the normalized `9`). Writing the normalized number would bind
// the wrong physical control. So the Mupen encoder takes a [`PadMapping`] — the
// device's resolved normalized→raw translation, probed from the *same libSDL2 the
// emulator uses* (guaranteeing the indices agree) — and never guesses: an input
// whose raw element can't be resolved is returned unencoded for the core's own UI.

/// A raw SDL *joystick* element, as Mupen's SDL input plugin addresses it in
/// manual mode. This is the target of the translation, distinct from the
/// normalized [`W3cInput`] the profile stores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawElement {
    Button(u32),
    /// A full analog axis (the direction sign is supplied by the capture, not the
    /// SDL bind, which names only the axis index).
    Axis(u32),
    Hat(u32, HatDir),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HatDir {
    Up,
    Down,
    Left,
    Right,
}

impl HatDir {
    fn word(self) -> &'static str {
        match self {
            HatDir::Up => "Up",
            HatDir::Down => "Down",
            HatDir::Left => "Left",
            HatDir::Right => "Right",
        }
    }
}

// SDL_GameControllerButton / SDL_GameControllerAxis enum values (SDL2). These are
// the index space the SDL bind API answers in — *not* the W3C layout — so the
// W3C→SDL step below remaps before the per-device PadMapping is consulted.
const SDL_BTN_A: u32 = 0;
const SDL_BTN_B: u32 = 1;
const SDL_BTN_X: u32 = 2;
const SDL_BTN_Y: u32 = 3;
const SDL_BTN_BACK: u32 = 4;
const SDL_BTN_GUIDE: u32 = 5;
const SDL_BTN_START: u32 = 6;
const SDL_BTN_LSTICK: u32 = 7;
const SDL_BTN_RSTICK: u32 = 8;
const SDL_BTN_LSHOULDER: u32 = 9;
const SDL_BTN_RSHOULDER: u32 = 10;
const SDL_BTN_DPAD_UP: u32 = 11;
const SDL_BTN_DPAD_DOWN: u32 = 12;
const SDL_BTN_DPAD_LEFT: u32 = 13;
const SDL_BTN_DPAD_RIGHT: u32 = 14;

const SDL_AXIS_LEFTX: u32 = 0;
const SDL_AXIS_LEFTY: u32 = 1;
const SDL_AXIS_RIGHTX: u32 = 2;
const SDL_AXIS_RIGHTY: u32 = 3;
const SDL_AXIS_TRIGGERLEFT: u32 = 4;
const SDL_AXIS_TRIGGERRIGHT: u32 = 5;

/// A SDL *gamecontroller* element — the intermediate the W3C index maps to before
/// the device mapping resolves it to a raw joystick element.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SdlGc {
    Button(u32),
    Axis(u32, AxisSign),
}

/// Translate a normalized W3C descriptor to the SDL gamecontroller element it
/// denotes. The W3C/gilrs button order (South=0…Guide=16, axes left=0/1,
/// right=2/3) differs from SDL's enum order, and the two analog triggers are
/// *buttons* in W3C but *axes* in SDL — both reconciled here. `None` for indices
/// outside the standard layout (extra paddles, etc.).
fn w3c_to_sdl_gc(w3c: W3cInput) -> Option<SdlGc> {
    Some(match w3c {
        W3cInput::Button(n) => match n {
            0 => SdlGc::Button(SDL_BTN_A),
            1 => SdlGc::Button(SDL_BTN_B),
            2 => SdlGc::Button(SDL_BTN_X),
            3 => SdlGc::Button(SDL_BTN_Y),
            4 => SdlGc::Button(SDL_BTN_LSHOULDER),
            5 => SdlGc::Button(SDL_BTN_RSHOULDER),
            6 => SdlGc::Axis(SDL_AXIS_TRIGGERLEFT, AxisSign::Pos), // LT: button in W3C, axis in SDL
            7 => SdlGc::Axis(SDL_AXIS_TRIGGERRIGHT, AxisSign::Pos), // RT
            8 => SdlGc::Button(SDL_BTN_BACK),
            9 => SdlGc::Button(SDL_BTN_START),
            10 => SdlGc::Button(SDL_BTN_LSTICK),
            11 => SdlGc::Button(SDL_BTN_RSTICK),
            12 => SdlGc::Button(SDL_BTN_DPAD_UP),
            13 => SdlGc::Button(SDL_BTN_DPAD_DOWN),
            14 => SdlGc::Button(SDL_BTN_DPAD_LEFT),
            15 => SdlGc::Button(SDL_BTN_DPAD_RIGHT),
            16 => SdlGc::Button(SDL_BTN_GUIDE),
            _ => return None,
        },
        W3cInput::Axis(n, sign) => match n {
            0 => SdlGc::Axis(SDL_AXIS_LEFTX, sign),
            1 => SdlGc::Axis(SDL_AXIS_LEFTY, sign),
            2 => SdlGc::Axis(SDL_AXIS_RIGHTX, sign),
            3 => SdlGc::Axis(SDL_AXIS_RIGHTY, sign),
            _ => return None,
        },
    })
}

/// The device's resolved translation from SDL gamecontroller elements to raw SDL
/// joystick elements — i.e. the answer to `SDL_GameControllerGetBindFor{Button,
/// Axis}` for every element, for one physical pad. Built at apply-time from the
/// live SDL view (see the desktop probe); kept here as plain data so the encoder
/// stays pure and unit-testable against a hand-built map.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PadMapping {
    buttons: std::collections::BTreeMap<u32, RawElement>,
    axes: std::collections::BTreeMap<u32, RawElement>,
}

impl PadMapping {
    pub fn new() -> Self {
        Self::default()
    }
    /// Record the raw element a SDL gamecontroller *button* resolves to.
    pub fn set_button(&mut self, sdl_button: u32, raw: RawElement) {
        self.buttons.insert(sdl_button, raw);
    }
    /// Record the raw element a SDL gamecontroller *axis* resolves to.
    pub fn set_axis(&mut self, sdl_axis: u32, raw: RawElement) {
        self.axes.insert(sdl_axis, raw);
    }
    pub fn is_empty(&self) -> bool {
        self.buttons.is_empty() && self.axes.is_empty()
    }
}

/// A raw element plus the effective direction sign carried from the W3C capture
/// (only meaningful when the element is an axis half).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ResolvedRaw {
    element: RawElement,
    sign: AxisSign,
}

/// Resolve a normalized W3C descriptor to this device's raw joystick element.
fn resolve_raw(w3c: W3cInput, mapping: &PadMapping) -> Option<ResolvedRaw> {
    match w3c_to_sdl_gc(w3c)? {
        SdlGc::Button(b) => mapping.buttons.get(&b).map(|&element| ResolvedRaw {
            element,
            sign: AxisSign::Pos,
        }),
        SdlGc::Axis(a, sign) => mapping
            .axes
            .get(&a)
            .map(|&element| ResolvedRaw { element, sign }),
    }
}

/// Format a resolved raw element as a Mupen value token (`button(11)`,
/// `axis(5+)`, `hat(0 Up)`).
fn mupen_token(r: ResolvedRaw) -> String {
    match r.element {
        RawElement::Button(n) => format!("button({n})"),
        RawElement::Hat(n, dir) => format!("hat({n} {})", dir.word()),
        RawElement::Axis(n) => format!("axis({n}{})", r.sign.glyph()),
    }
}

/// Map an N64 *digital* console input id to its `[Input-SDL-ControlN]` config key.
/// The analog stick (`stick_*`) is not here — it folds into the combined `X Axis`
/// / `Y Axis` keys in [`materialize_mupen`]. `None` for ids that aren't N64 digital
/// inputs.
fn mupen_digital_key(input_id: &str) -> Option<&'static str> {
    Some(match input_id {
        "dpad_up" => "DPad U",
        "dpad_down" => "DPad D",
        "dpad_left" => "DPad L",
        "dpad_right" => "DPad R",
        "a" => "A Button",
        "b" => "B Button",
        "c_up" => "C Button U",
        "c_down" => "C Button D",
        "c_left" => "C Button L",
        "c_right" => "C Button R",
        "l" => "L Trig",
        "r" => "R Trig",
        "z" => "Z Trig",
        "start" => "Start",
        _ => return None,
    })
}

/// Encode one N64 digital input into a Mupen `key = "token"` assignment using the
/// device mapping. `None` if the input isn't a Mupen digital key or its raw
/// element can't be resolved (left for the escape hatch).
pub fn encode_mupen_digital(
    input_id: &str,
    w3c: W3cInput,
    mapping: &PadMapping,
) -> Option<Assignment> {
    let key = mupen_digital_key(input_id)?;
    let resolved = resolve_raw(w3c, mapping)?;
    Some(Assignment {
        key: key.to_string(),
        value: format!("\"{}\"", mupen_token(resolved)),
    })
}

/// Build the combined `X Axis` / `Y Axis` value from a pair of stick-direction
/// captures. Both directions must resolve to the *same* raw analog axis (the
/// normal analog-stick case); anything else (stick mapped to a hat/buttons) is
/// rejected so we never write a token Mupen's analog parser would choke on.
fn mupen_axis_pair(
    neg: Option<&str>,
    pos: Option<&str>,
    mapping: &PadMapping,
) -> Option<String> {
    let neg = neg.and_then(parse_w3c).and_then(|w| resolve_raw(w, mapping))?;
    let pos = pos.and_then(parse_w3c).and_then(|w| resolve_raw(w, mapping))?;
    match (neg.element, pos.element) {
        (RawElement::Axis(a), RawElement::Axis(b)) if a == b => Some(format!("axis({a}-,{a}+)")),
        _ => None,
    }
}

/// Turn an N64 profile into the Mupen `[Input-SDL-ControlN]` assignments to write,
/// resolving every binding through the device mapping. Digital inputs map 1:1; the
/// four `stick_*` directions fold into the two combined analog axis keys. Inputs
/// that can't be resolved (or a stick that isn't a clean analog axis) are returned
/// in `unencoded` for the core's own remap UI.
pub fn materialize_mupen(
    bindings: &std::collections::BTreeMap<String, String>,
    mapping: &PadMapping,
) -> (Vec<Assignment>, Vec<String>) {
    let mut out = Vec::new();
    let mut unencoded = Vec::new();

    // Digital inputs first, in catalogue-stable (BTreeMap) order.
    for (input_id, desc) in bindings {
        if input_id.starts_with("stick_") {
            continue; // handled as a pair below
        }
        match parse_w3c(desc).and_then(|w| encode_mupen_digital(input_id, w, mapping)) {
            Some(a) => out.push(a),
            None => unencoded.push(input_id.clone()),
        }
    }

    // Analog stick: combine the captured halves into Mupen's two-ended axis keys.
    let get = |id: &str| bindings.get(id).map(String::as_str);
    let has_stick = ["stick_left", "stick_right", "stick_up", "stick_down"]
        .iter()
        .any(|id| bindings.contains_key(*id));
    if has_stick {
        match mupen_axis_pair(get("stick_left"), get("stick_right"), mapping) {
            Some(v) => out.push(Assignment {
                key: "X Axis".to_string(),
                value: format!("\"{v}\""),
            }),
            None => {
                if bindings.contains_key("stick_left") {
                    unencoded.push("stick_left".to_string());
                }
                if bindings.contains_key("stick_right") {
                    unencoded.push("stick_right".to_string());
                }
            }
        }
        match mupen_axis_pair(get("stick_up"), get("stick_down"), mapping) {
            Some(v) => out.push(Assignment {
                key: "Y Axis".to_string(),
                value: format!("\"{v}\""),
            }),
            None => {
                if bindings.contains_key("stick_up") {
                    unencoded.push("stick_up".to_string());
                }
                if bindings.contains_key("stick_down") {
                    unencoded.push("stick_down".to_string());
                }
            }
        }
    }

    (out, unencoded)
}

/// Surgically set `key = value` *within a named INI section* (`[section]`),
/// preserving every other byte — the sectioned analogue of [`set_flat_key`], for
/// configs like `mupen64plus.cfg` where the same key (`mode`, `device`, `A Button`)
/// recurs under each `[Input-SDL-ControlN]`. Keys may contain spaces. The value is
/// written verbatim (the caller quotes if needed). If the key is missing the line
/// is inserted at the end of the section; if the section is missing it is appended.
pub fn set_ini_key_in_section(text: &str, section: &str, key: &str, value: &str) -> String {
    set_ini_key_in_section_fmt(text, section, key, value, true)
}

/// Like [`set_ini_key_in_section`] but writes `key=value` with **no spaces** around
/// the `=`, matching emulators (mGBA) whose own writer emits the compact form and
/// whose INI reader may not trim surrounding whitespace.
pub fn set_ini_key_in_section_compact(text: &str, section: &str, key: &str, value: &str) -> String {
    set_ini_key_in_section_fmt(text, section, key, value, false)
}

fn set_ini_key_in_section_fmt(
    text: &str,
    section: &str,
    key: &str,
    value: &str,
    spaced: bool,
) -> String {
    let render = |indent: &str, key: &str, value: &str| {
        if spaced {
            format!("{indent}{key} = {value}")
        } else {
            format!("{indent}{key}={value}")
        }
    };
    let header = format!("[{section}]");
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();

    // Locate the target section's body span [start, end).
    let mut sec_start: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        if line.trim() == header {
            sec_start = Some(i + 1);
            break;
        }
    }
    let trailing_nl = text.ends_with('\n');
    let join = |lines: Vec<String>| {
        let mut s = lines.join("\n");
        if trailing_nl {
            s.push('\n');
        }
        s
    };

    let Some(start) = sec_start else {
        // No such section — append it with the single key.
        if !lines.is_empty() {
            lines.push(String::new());
        }
        lines.push(header);
        lines.push(render("", key, value));
        return join(lines);
    };

    // End of section = next `[...]` header or EOF.
    let mut end = lines.len();
    for (off, line) in lines.iter().enumerate().skip(start) {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            end = off;
            break;
        }
    }

    // Replace the key in place if present in this section.
    for line in lines.iter_mut().take(end).skip(start) {
        let lhs = line.split('=').next().unwrap_or("");
        if lhs.trim() == key {
            let indent = &line[..line.len() - line.trim_start().len()];
            *line = render(indent, key, value);
            return join(lines);
        }
    }

    // Absent within the section — insert at the end of the section body.
    lines.insert(end, render("", key, value));
    join(lines)
}

// ── Mednafen (Tier B, GUID-embedded) ─────────────────────────────────────────
//
// Mednafen is Tier B like Mupen — it reads *raw SDL-joystick* element indices —
// but with two extra wrinkles. (1) Its config is flat, whitespace-delimited
// `key value` lines (no `=`, no sections), and every gamepad binding *embeds the
// device GUID*: `vb.input.builtin.gamepad.a joystick 0x<32-hex-guid> button_0`.
// Get the GUID wrong and every binding silently points at no device. (2) The
// d-pad is backend-divergent: SDL's GameController API reports this project's pad
// d-pad as a *hat*, but Mednafen flattens hats into virtual *axes* (`abs_6`/
// `abs_7`) and there is no version-stable arithmetic to derive those indices from
// the SDL hat. So we never guess the d-pad axes — we read them verbatim from the
// existing config (Mednafen's own ground truth) and reuse that convention; absent
// a prior binding the d-pad is left unencoded for Mednafen's in-game config UI.
//
// Everything else (faces, shoulders, triggers, the right d-pad bound to the right
// analog stick) resolves through the same [`PadMapping`] the probe builds, so the
// raw indices are guaranteed to agree with what Mednafen records.

/// Map an Arcadia platform id to Mednafen's internal system prefix (the `vb` in
/// `vb.input.builtin.gamepad.*`). `None` for systems we don't yet encode for
/// Mednafen.
fn mednafen_system_prefix(system: &str) -> Option<&'static str> {
    Some(match system {
        "virtualboy" => "vb",
        _ => return None,
    })
}

/// Map a console input id to its Mednafen builtin-gamepad input suffix (the `a`
/// in `vb.input.builtin.gamepad.a`). The left d-pad uses Mednafen's `*-l` suffix
/// and the right d-pad — conventionally the right analog stick — uses `*-r`.
/// `None` for ids that aren't Virtual Boy gamepad inputs.
fn mednafen_vb_suffix(input_id: &str) -> Option<&'static str> {
    Some(match input_id {
        "a" => "a",
        "b" => "b",
        "rapid_a" => "rapid_a",
        "rapid_b" => "rapid_b",
        "l" => "lt",
        "r" => "rt",
        "start" => "start",
        "select" => "select",
        "dpad_up" => "up-l",
        "dpad_down" => "down-l",
        "dpad_left" => "left-l",
        "dpad_right" => "right-l",
        "r_dpad_up" => "up-r",
        "r_dpad_down" => "down-r",
        "r_dpad_left" => "left-r",
        "r_dpad_right" => "right-r",
        _ => return None,
    })
}

/// The full Mednafen config key for one (system, input) pair, e.g.
/// `vb.input.builtin.gamepad.up-l`.
fn mednafen_key(system: &str, input_id: &str) -> Option<String> {
    let prefix = mednafen_system_prefix(system)?;
    let suffix = mednafen_vb_suffix(input_id)?;
    Some(format!("{prefix}.input.builtin.gamepad.{suffix}"))
}

/// Format a raw element as a Mednafen element token (`button_3`, `abs_6-`).
/// Hats are intentionally unsupported: Mednafen flattens hats to virtual axes
/// and we never synthesize that index (see module note) — a hat returns `None`
/// so the d-pad falls back to the config-derived axis convention or the escape
/// hatch.
fn mednafen_element(element: RawElement, sign: AxisSign) -> Option<String> {
    match element {
        RawElement::Button(n) => Some(format!("button_{n}")),
        RawElement::Axis(n) => Some(format!("abs_{n}{}", sign.glyph())),
        RawElement::Hat(_, _) => None,
    }
}

/// The d-pad direction → (axis-relative sign) Mednafen records when the d-pad is
/// a flattened axis pair: left/up are the negative half, right/down the positive
/// half (matching a working `mednafen.cfg`). Returns which axis (X vs Y) and the
/// sign for a given W3C d-pad button index (12=up,13=down,14=left,15=right).
fn vb_dpad_axis(w3c_button: u32, x_axis: u32, y_axis: u32) -> Option<(u32, AxisSign)> {
    Some(match w3c_button {
        12 => (y_axis, AxisSign::Neg), // up
        13 => (y_axis, AxisSign::Pos), // down
        14 => (x_axis, AxisSign::Neg), // left
        15 => (x_axis, AxisSign::Pos), // right
        _ => return None,
    })
}

/// Encode one VB input into a Mednafen `key value` assignment. `dpad_axes` is the
/// `(x_axis, y_axis)` convention read from the existing config; when present, a
/// d-pad input captured as the physical d-pad (W3C buttons 12-15) is encoded as
/// the corresponding flattened axis half. All other inputs resolve through the
/// device `mapping`. `None` leaves the input for Mednafen's own config UI.
fn encode_mednafen(
    system: &str,
    input_id: &str,
    w3c: W3cInput,
    mapping: &PadMapping,
    guid: &str,
    dpad_axes: Option<(u32, u32)>,
) -> Option<Assignment> {
    let key = mednafen_key(system, input_id)?;
    // D-pad captured as the physical d-pad: prefer the config-derived axis pair
    // (Mednafen's flattened-hat convention) over the probe's hat, which Mednafen
    // can't address.
    let element = if let (W3cInput::Button(b @ 12..=15), Some((x, y))) = (w3c, dpad_axes) {
        let (axis, sign) = vb_dpad_axis(b, x, y)?;
        format!("abs_{axis}{}", sign.glyph())
    } else {
        let resolved = resolve_raw(w3c, mapping)?;
        mednafen_element(resolved.element, resolved.sign)?
    };
    Some(Assignment {
        key,
        value: format!("joystick 0x{guid} {element}"),
    })
}

/// Turn a Mednafen-system profile into the `key value` assignments to write,
/// resolving each binding through the device mapping and embedding `guid`. The
/// d-pad reuses the `dpad_axes` convention from the existing config (ground
/// truth) — see the module note on backend divergence. Inputs that can't be
/// resolved (including the d-pad when no prior axis convention exists) are
/// returned in `unencoded` for Mednafen's in-game config.
pub fn materialize_mednafen(
    system: &str,
    bindings: &std::collections::BTreeMap<String, String>,
    mapping: &PadMapping,
    guid: &str,
    dpad_axes: Option<(u32, u32)>,
) -> (Vec<Assignment>, Vec<String>) {
    let mut out = Vec::new();
    let mut unencoded = Vec::new();
    for (input_id, desc) in bindings {
        match parse_w3c(desc)
            .and_then(|w| encode_mednafen(system, input_id, w, mapping, guid, dpad_axes))
        {
            Some(a) => out.push(a),
            None => unencoded.push(input_id.clone()),
        }
    }
    (out, unencoded)
}

/// Read the device GUID an existing `mednafen.cfg` uses for `system`'s builtin
/// gamepad — the most robust GUID source, since it is exactly what Mednafen will
/// match against. Scans for the first `<prefix>.input.builtin.gamepad.* joystick
/// 0x<guid> ...` line. Returns the bare hex (no `0x`). `None` if unconfigured.
pub fn mednafen_config_guid(text: &str, system: &str) -> Option<String> {
    let prefix = mednafen_system_prefix(system)?;
    let needle = format!("{prefix}.input.builtin.gamepad.");
    for line in text.lines() {
        let mut toks = line.split_whitespace();
        let key = toks.next()?;
        if !key.starts_with(&needle) {
            continue;
        }
        if toks.next() != Some("joystick") {
            continue;
        }
        if let Some(g) = toks.next().and_then(|t| t.strip_prefix("0x")) {
            if !g.is_empty() {
                return Some(g.to_string());
            }
        }
    }
    None
}

/// Read the `(x_axis, y_axis)` convention an existing `mednafen.cfg` uses for
/// `system`'s left d-pad (Mednafen's flattened-hat axes). Reads the X axis from a
/// `left-l`/`right-l` `abs_N±` token and the Y axis from `up-l`/`down-l`. `None`
/// if either axis isn't found (the caller then leaves the d-pad unencoded).
pub fn mednafen_config_dpad_axes(text: &str, system: &str) -> Option<(u32, u32)> {
    let prefix = mednafen_system_prefix(system)?;
    let mut x_axis = None;
    let mut y_axis = None;
    for line in text.lines() {
        let mut toks = line.split_whitespace();
        let Some(key) = toks.next() else { continue };
        let Some(suffix) = key.strip_prefix(&format!("{prefix}.input.builtin.gamepad.")) else {
            continue;
        };
        let slot = match suffix {
            "left-l" | "right-l" => &mut x_axis,
            "up-l" | "down-l" => &mut y_axis,
            _ => continue,
        };
        // element is the last token: `abs_6-`.
        if let Some(n) = toks
            .last()
            .and_then(|e| e.strip_prefix("abs_"))
            .map(|rest| rest.trim_end_matches(['+', '-']))
            .and_then(|n| n.parse::<u32>().ok())
        {
            slot.get_or_insert(n);
        }
    }
    Some((x_axis?, y_axis?))
}

/// Surgically set a Mednafen `key value` setting: replace the value of an
/// existing `key ...` line in place (preserving every other byte), or append the
/// line if the key is absent. Mednafen's config is flat and whitespace-delimited
/// (no `=`), so this matches on the first whitespace-separated token.
pub fn set_mednafen_setting(text: &str, key: &str, value: &str) -> String {
    let mut replaced = false;
    let mut lines: Vec<String> = text
        .lines()
        .map(|line| {
            if replaced {
                return line.to_string();
            }
            if line.split_whitespace().next() == Some(key) {
                replaced = true;
                return format!("{key} {value}");
            }
            line.to_string()
        })
        .collect();
    if !replaced {
        lines.push(format!("{key} {value}"));
    }
    let mut joined = lines.join("\n");
    if text.ends_with('\n') {
        joined.push('\n');
    }
    joined
}

// ── Snes9x (Tier B, raw SDL-joystick indices) ────────────────────────────────
//
// snes9x-gtk reads *raw SDL-joystick* element indices (like Mupen), written into
// `[Joypad N]` sections of `snes9x.conf` as human tokens:
//   A    = Joystick 1 Button 0
//   L    = Joystick 1 Axis 2 + 40%
// "Joystick M" is the 1-based device number (SDL index 0 → "Joystick 1"). The raw
// indices only agree with the pad's live SDL view, so faces / shoulders / triggers
// resolve through the same [`PadMapping`] the probe builds.
//
// The d-pad is the wrinkle (cf. Mednafen): snes9x-gtk has NO hat token — verified
// against the binary's only joystick format strings, `%u Button %u` and
// `%u Axis %u %c %u`. It flattens SDL hats into virtual *axes* appended after the
// real ones, and that flattened index isn't derivable without the joystick's axis
// count. So when the probe resolves a d-pad direction to a hat we never synthesize
// the axis: the existing `[Joypad N]` d-pad lines (snes9x's own ground truth) are
// left untouched, and — absent a prior binding — the input is reported unencoded
// for snes9x's in-app input detect.

/// snes9x's default analog deadzone threshold, written as a percentage on every
/// axis token (matches the value snes9x's own input UI records).
const SNES9X_AXIS_THRESHOLD: u32 = 40;

/// Map a console input id to its `[Joypad N]` key in `snes9x.conf`. SNES-shaped
/// (the RetroPad/SNES face layout). `None` for ids that aren't SNES pad inputs.
fn snes9x_pad_key(input_id: &str) -> Option<&'static str> {
    Some(match input_id {
        "dpad_up" => "Up",
        "dpad_down" => "Down",
        "dpad_left" => "Left",
        "dpad_right" => "Right",
        "a" => "A",
        "b" => "B",
        "x" => "X",
        "y" => "Y",
        "l" => "L",
        "r" => "R",
        "start" => "Start",
        "select" => "Select",
        _ => return None,
    })
}

/// Format a resolved raw element as a snes9x joystick token. Hats return `None`:
/// snes9x has no hat token (see module note), so a hat-resolved input falls back
/// to config reuse / the in-app detect rather than a token snes9x can't parse.
fn snes9x_token(joy_num: u32, r: ResolvedRaw) -> Option<String> {
    match r.element {
        RawElement::Button(n) => Some(format!("Joystick {joy_num} Button {n}")),
        RawElement::Axis(n) => Some(format!(
            "Joystick {joy_num} Axis {n} {} {SNES9X_AXIS_THRESHOLD}%",
            r.sign.glyph()
        )),
        RawElement::Hat(_, _) => None,
    }
}

/// Turn a SNES profile into the `[Joypad N]` assignments to write, resolving each
/// binding through the device `mapping`. `joy_num` is the 1-based snes9x device
/// number (read from the existing config, else 1). Inputs that can't be resolved —
/// including any d-pad direction that probes as an unaddressable hat — are returned
/// in `unencoded`.
pub fn materialize_snes9x(
    bindings: &std::collections::BTreeMap<String, String>,
    mapping: &PadMapping,
    joy_num: u32,
) -> (Vec<Assignment>, Vec<String>) {
    let mut out = Vec::new();
    let mut unencoded = Vec::new();
    for (input_id, desc) in bindings {
        let Some(key) = snes9x_pad_key(input_id) else {
            unencoded.push(input_id.clone());
            continue;
        };
        let token = parse_w3c(desc)
            .and_then(|w| resolve_raw(w, mapping))
            .and_then(|r| snes9x_token(joy_num, r));
        match token {
            Some(value) => out.push(Assignment { key: key.to_string(), value }),
            None => unencoded.push(input_id.clone()),
        }
    }
    (out, unencoded)
}

/// Read the 1-based device number an existing `[section]` in `snes9x.conf` already
/// binds to (the `M` in `... = Joystick M Button 0`). Scans for the first
/// `Joystick <M> ...` value. `None` if the section binds no joystick (the caller
/// then defaults to 1, i.e. the first pad).
pub fn snes9x_joystick_number(text: &str, section: &str) -> Option<u32> {
    let header = format!("[{section}]");
    let mut in_section = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_section = t == header;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((_, rhs)) = line.split_once('=') {
            let mut toks = rhs.split_whitespace();
            if toks.next() == Some("Joystick") {
                if let Some(n) = toks.next().and_then(|n| n.parse::<u32>().ok()) {
                    return Some(n);
                }
            }
        }
    }
    None
}

/// Whether `[section]` in `snes9x.conf` already holds a real (non-`Unset`) binding
/// for `input_id`. Used to avoid nagging the user about a d-pad that snes9x bound
/// as a flattened axis — which the probe sees as an unaddressable hat, but which is
/// already working in the config and must be left untouched.
pub fn snes9x_has_binding(text: &str, section: &str, input_id: &str) -> bool {
    let Some(key) = snes9x_pad_key(input_id) else {
        return false;
    };
    let header = format!("[{section}]");
    let mut in_section = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_section = t == header;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((lhs, rhs)) = line.split_once('=') {
            if lhs.trim() == key {
                return rhs.trim() != "Unset" && !rhs.trim().is_empty();
            }
        }
    }
    false
}

// ── mGBA (Tier B, raw SDL-joystick indices) ──────────────────────────────────
//
// mGBA-qt reads *raw SDL-joystick* element indices into `[<platform>.input.SDLB]`
// of `config.ini` (platform = "gba" for GBA, "gb" for Game Boy / Color). It writes
// `key=value` with no spaces and three binding shapes, one of which is inverted:
//   key<Name>        = <button_index>          (game key ← joystick button)
//   axis<Name>Axis   = <sign><axis_index>      (game key ← analog half …)
//   axis<Name>Value  = <signed threshold>      (… with its activation threshold)
//   hat<Idx><Dir>    = <game-key ENUM value>   (hat direction → game key — inverted)
// The raw indices only agree with the pad's live SDL view, so every binding
// resolves through the same [`PadMapping`] the probe builds. Unlike snes9x, mGBA
// addresses SDL hats natively, so a d-pad that probes as a hat encodes directly.
//
// NB: mGBA can hold a button *and* an axis binding for the same game key (its own
// autoconfig binds the left stick to the d-pad alongside the real hat). We write
// the one representation the user captured and never aggressively clear the other,
// matching the surgical philosophy of the rest of this module.

/// mGBA's default analog→digital activation threshold magnitude (`0x4000`); the
/// written `axis<Name>Value` carries this signed to match the half-axis direction.
const MGBA_AXIS_THRESHOLD: i32 = 16384;

/// Map an Arcadia platform id to mGBA's config-section platform prefix. GB and GBC
/// share mGBA's single "gb" core/platform. `None` for non-mGBA systems.
fn mgba_platform(system: &str) -> Option<&'static str> {
    Some(match system {
        "gb" | "gbc" => "gb",
        "gba" => "gba",
        _ => return None,
    })
}

/// The `[<platform>.input.SDLB]` section name for a system's player-1 SDL bindings.
pub fn mgba_section(system: &str) -> Option<String> {
    Some(format!("{}.input.SDLB", mgba_platform(system)?))
}

/// The `[<platform>.input-profile.<name>]` section name for a system's per-device
/// bindings. mGBA's **Qt** frontend loads this section *after* the generic SDLB
/// one (keyed by `SDL_JoystickName`) and lets it win, so a write that only touches
/// SDLB has no in-game effect when such a profile exists. `name` is the raw SDL
/// joystick name (e.g. "Xbox Wireless Controller"). `None` for non-mGBA systems.
pub fn mgba_profile_section(system: &str, name: &str) -> Option<String> {
    Some(format!("{}.input-profile.{}", mgba_platform(system)?, name))
}

/// Map a console input id to mGBA's (config key-name suffix, game-key ENUM value).
/// The suffix names the `key<Name>` / `axis<Name>*` lines; the enum value is what a
/// `hat<Idx><Dir>` line stores (mGBA's inverted hat form). `None` for ids that
/// aren't Game Boy / GBA pad inputs.
fn mgba_game_key(input_id: &str) -> Option<(&'static str, u32)> {
    Some(match input_id {
        "a" => ("A", 0),
        "b" => ("B", 1),
        "select" => ("Select", 2),
        "start" => ("Start", 3),
        "dpad_right" => ("Right", 4),
        "dpad_left" => ("Left", 5),
        "dpad_up" => ("Up", 6),
        "dpad_down" => ("Down", 7),
        "l" => ("L", 8),
        "r" => ("R", 9),
        _ => return None,
    })
}

/// Turn a Game Boy / GBA profile into the `[<platform>.input.SDLB]` assignments to
/// write, resolving each binding through the device `mapping`. Buttons emit one
/// line, analog halves emit the `Axis`+`Value` pair, and hats emit mGBA's inverted
/// `hat<Idx><Dir> = <gamekey>` line. Inputs that can't be resolved are returned in
/// `unencoded` for mGBA's own input detect.
pub fn materialize_mgba(
    bindings: &std::collections::BTreeMap<String, String>,
    mapping: &PadMapping,
) -> (Vec<Assignment>, Vec<String>) {
    let mut out = Vec::new();
    let mut unencoded = Vec::new();
    for (input_id, desc) in bindings {
        let Some((name, gamekey)) = mgba_game_key(input_id) else {
            unencoded.push(input_id.clone());
            continue;
        };
        match parse_w3c(desc).and_then(|w| resolve_raw(w, mapping)) {
            Some(r) => match r.element {
                RawElement::Button(n) => out.push(Assignment {
                    key: format!("key{name}"),
                    value: n.to_string(),
                }),
                RawElement::Axis(n) => {
                    out.push(Assignment {
                        key: format!("axis{name}Axis"),
                        value: format!("{}{n}", r.sign.glyph()),
                    });
                    let threshold = match r.sign {
                        AxisSign::Pos => MGBA_AXIS_THRESHOLD,
                        AxisSign::Neg => -MGBA_AXIS_THRESHOLD,
                    };
                    out.push(Assignment {
                        key: format!("axis{name}Value"),
                        value: threshold.to_string(),
                    });
                }
                RawElement::Hat(idx, dir) => out.push(Assignment {
                    key: format!("hat{idx}{}", dir.word()),
                    value: gamekey.to_string(),
                }),
            },
            None => unencoded.push(input_id.clone()),
        }
    }
    (out, unencoded)
}

// ---------------------------------------------------------------------------
// Tier C: SDL named-token PlayStation standalones (PCSX2, DuckStation)
//
// These emulators bind through SDL's *standard gamepad* abstraction, so a
// binding is a friendly token name (`SDL-0/FaceSouth`, `SDL-1/A`, `-LeftX`) —
// NOT a raw element index and NOT keyed by a device GUID. The token namespace is
// exactly the W3C standard-gamepad layout, so the mapping is device-independent:
// no SDL probe, no identity probe. The only per-device bit is the `SDL-N` index
// prefix, which we read from the existing config (and default to `SDL-0`).
// ---------------------------------------------------------------------------

/// Which emulator's token vocabulary to emit. PCSX2 and DuckStation agree on
/// most names (DPad/shoulders/sticks/Back/Start) but differ on face buttons and
/// trigger spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PsDialect {
    Pcsx2,
    DuckStation,
}

/// PlayStation console input id → INI key in the emulator's `[Pad1]` section.
/// DualShock-shaped, identical for PCSX2 (PS2/DualShock2) and DuckStation (PS1).
fn ps_pad_key(input_id: &str) -> Option<&'static str> {
    Some(match input_id {
        "cross" => "Cross",
        "circle" => "Circle",
        "square" => "Square",
        "triangle" => "Triangle",
        "dpad_up" => "Up",
        "dpad_down" => "Down",
        "dpad_left" => "Left",
        "dpad_right" => "Right",
        "l1" => "L1",
        "l2" => "L2",
        "r1" => "R1",
        "r2" => "R2",
        "l3" => "L3",
        "r3" => "R3",
        "start" => "Start",
        "select" => "Select",
        "l_stick_up" => "LUp",
        "l_stick_down" => "LDown",
        "l_stick_left" => "LLeft",
        "l_stick_right" => "LRight",
        "r_stick_up" => "RUp",
        "r_stick_down" => "RDown",
        "r_stick_left" => "RLeft",
        "r_stick_right" => "RRight",
        _ => return None,
    })
}

/// W3C standard-gamepad element → the emulator's SDL token (without the `SDL-N/`
/// prefix). Returns `None` for indices outside the standard layout so the caller
/// records them unencoded rather than guessing.
fn ps_sdl_token(w3c: W3cInput, dialect: PsDialect) -> Option<String> {
    use PsDialect::{DuckStation, Pcsx2};
    let tok = match w3c {
        W3cInput::Button(n) => match n {
            // Face buttons: PCSX2 uses positional Face*, DuckStation SDL letters.
            0 => match dialect {
                Pcsx2 => "FaceSouth",
                DuckStation => "A",
            },
            1 => match dialect {
                Pcsx2 => "FaceEast",
                DuckStation => "B",
            },
            2 => match dialect {
                Pcsx2 => "FaceWest",
                DuckStation => "X",
            },
            3 => match dialect {
                Pcsx2 => "FaceNorth",
                DuckStation => "Y",
            },
            4 => "LeftShoulder",
            5 => "RightShoulder",
            // Analog triggers as buttons; DuckStation suffixes `~` (half-axis).
            6 => match dialect {
                Pcsx2 => "+LeftTrigger",
                DuckStation => "+LeftTrigger~",
            },
            7 => match dialect {
                Pcsx2 => "+RightTrigger",
                DuckStation => "+RightTrigger~",
            },
            8 => "Back",
            9 => "Start",
            10 => "LeftStick",
            11 => "RightStick",
            12 => "DPadUp",
            13 => "DPadDown",
            14 => "DPadLeft",
            15 => "DPadRight",
            16 => "Guide",
            _ => return None,
        }
        .to_string(),
        W3cInput::Axis(n, sign) => {
            let axis = match n {
                0 => "LeftX",
                1 => "LeftY",
                2 => "RightX",
                3 => "RightY",
                _ => return None,
            };
            format!("{}{}", sign.glyph(), axis)
        }
    };
    Some(tok)
}

/// Encode a full PlayStation profile into `[Pad1]` `Key = SDL-N/Token` lines for
/// PCSX2 or DuckStation. `device_prefix` is the `SDL-N` index the emulator uses
/// for the pad (read from config, else `SDL-0`).
pub fn materialize_ps_named(
    bindings: &std::collections::BTreeMap<String, String>,
    dialect: PsDialect,
    device_prefix: &str,
) -> (Vec<Assignment>, Vec<String>) {
    let mut out = Vec::new();
    let mut unencoded = Vec::new();
    for (input_id, desc) in bindings {
        let Some(key) = ps_pad_key(input_id) else {
            unencoded.push(input_id.clone());
            continue;
        };
        match parse_w3c(desc).and_then(|w| ps_sdl_token(w, dialect)) {
            Some(tok) => out.push(Assignment {
                key: key.to_string(),
                value: format!("{device_prefix}/{tok}"),
            }),
            None => unencoded.push(input_id.clone()),
        }
    }
    (out, unencoded)
}

/// Read the `SDL-N` device index an existing `[section]` already uses for its
/// bindings (e.g. the pad enumerated as `SDL-1`). Scans the section for the first
/// `Key = SDL-<n>/...` value. `None` if unconfigured ⇒ caller defaults to `SDL-0`.
pub fn ps_device_prefix(text: &str, section: &str) -> Option<String> {
    let header = format!("[{section}]");
    let mut in_section = false;
    for line in text.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            in_section = t == header;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some((_, rhs)) = line.split_once('=') {
            let rhs = rhs.trim();
            if let Some(rest) = rhs.strip_prefix("SDL-") {
                let idx: String = rest.chars().take_while(char::is_ascii_digit).collect();
                if !idx.is_empty() {
                    return Some(format!("SDL-{idx}"));
                }
            }
        }
    }
    None
}

/// Extract Mednafen's *own* calculated joystick ID from its startup enumeration
/// line (`  ID: 0x<32-hex> - <name>`). This ID is evdev-derived (bus/vendor/
/// product/version + capability counts) and **backend-independent** — verified
/// identical under SDL's HIDAPI, evdev, and classic joystick backends. Crucially
/// it is NOT the SDL GUID (`SDL_JoystickGetGUIDString`): Mednafen keys its config
/// bindings by this calculated ID, so writing the SDL GUID leaves the pad dead.
/// Parses the first matching line; returns the bare 32-hex (no `0x`).
pub fn parse_mednafen_joystick_id(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("ID: 0x") {
            let hex: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
            if hex.len() == 32 {
                return Some(hex);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn parses_button_and_axis_descriptors() {
        assert_eq!(parse_w3c("btn:0"), Some(W3cInput::Button(0)));
        assert_eq!(parse_w3c("btn:11"), Some(W3cInput::Button(11)));
        assert_eq!(parse_w3c("axis:1-"), Some(W3cInput::Axis(1, AxisSign::Neg)));
        assert_eq!(parse_w3c("axis:2+"), Some(W3cInput::Axis(2, AxisSign::Pos)));
        assert_eq!(parse_w3c(" btn:3 "), Some(W3cInput::Button(3)));
    }

    #[test]
    fn rejects_malformed_descriptors() {
        assert_eq!(parse_w3c("btn:"), None);
        assert_eq!(parse_w3c("axis:1"), None); // missing sign
        assert_eq!(parse_w3c("hat:0"), None);
        assert_eq!(parse_w3c(""), None);
        assert_eq!(parse_w3c("btn:-1"), None);
    }

    #[test]
    fn encodes_digital_button() {
        let a = encode_retroarch(1, "a", W3cInput::Button(0)).unwrap();
        assert_eq!(a.key, "input_player1_a_btn");
        assert_eq!(a.value, "\"0\"");
    }

    #[test]
    fn encodes_dpad_and_player_index() {
        let a = encode_retroarch(2, "dpad_up", W3cInput::Button(12)).unwrap();
        assert_eq!(a.key, "input_player2_up_btn");
        assert_eq!(a.value, "\"12\"");
    }

    #[test]
    fn encodes_analog_stick_axis() {
        let a = encode_retroarch(1, "stick_left", W3cInput::Axis(0, AxisSign::Neg)).unwrap();
        assert_eq!(a.key, "input_player1_l_x_minus_axis");
        assert_eq!(a.value, "\"-0\"");
        let r = encode_retroarch(1, "r_stick_right", W3cInput::Axis(2, AxisSign::Pos)).unwrap();
        assert_eq!(r.key, "input_player1_r_x_plus_axis");
        assert_eq!(r.value, "\"+2\"");
    }

    #[test]
    fn psx_faces_map_to_retropad() {
        assert_eq!(retropad_base("cross"), Some("b"));
        assert_eq!(retropad_base("circle"), Some("a"));
        assert_eq!(retropad_base("square"), Some("y"));
        assert_eq!(retropad_base("triangle"), Some("x"));
    }

    #[test]
    fn unstable_inputs_are_not_guessed() {
        assert_eq!(retropad_base("c_up"), None);
        assert!(encode_retroarch(1, "c_left", W3cInput::Button(3)).is_none());
    }

    #[test]
    fn materialize_separates_encoded_and_unencoded() {
        let mut b = BTreeMap::new();
        b.insert("a".to_string(), "btn:0".to_string());
        b.insert("c_up".to_string(), "btn:3".to_string()); // no stable mapping
        b.insert("dpad_up".to_string(), "garbage".to_string()); // malformed
        let (encoded, unencoded) = materialize_retroarch(1, &b);
        assert_eq!(encoded.len(), 1);
        assert_eq!(encoded[0].key, "input_player1_a_btn");
        assert_eq!(unencoded.len(), 2);
        assert!(unencoded.contains(&"c_up".to_string()));
        assert!(unencoded.contains(&"dpad_up".to_string()));
    }

    #[test]
    fn set_flat_key_replaces_existing_in_place() {
        let cfg = "# header\ninput_player1_a_btn = \"5\"\ninput_player1_b_btn = \"6\"\n";
        let out = set_flat_key(cfg, "input_player1_a_btn", "\"0\"");
        assert!(out.contains("input_player1_a_btn = \"0\""));
        // other lines and the comment are byte-preserved
        assert!(out.contains("# header"));
        assert!(out.contains("input_player1_b_btn = \"6\""));
        assert!(out.ends_with('\n'));
        // only one occurrence of the key
        assert_eq!(out.matches("input_player1_a_btn").count(), 1);
    }

    #[test]
    fn set_flat_key_appends_when_absent() {
        let cfg = "input_player1_b_btn = \"6\"\n";
        let out = set_flat_key(cfg, "input_player1_a_btn", "\"0\"");
        assert!(out.contains("input_player1_b_btn = \"6\""));
        assert!(out.contains("input_player1_a_btn = \"0\""));
    }

    #[test]
    fn set_flat_key_preserves_indentation_and_no_trailing_newline() {
        let cfg = "  input_player1_a_btn = \"5\"";
        let out = set_flat_key(cfg, "input_player1_a_btn", "\"9\"");
        assert_eq!(out, "  input_player1_a_btn = \"9\"");
    }

    #[test]
    fn set_flat_key_does_not_match_key_prefix() {
        // `input_player1_a` must not match `input_player1_a_btn`'s line.
        let cfg = "input_player1_a_btn = \"5\"\n";
        let out = set_flat_key(cfg, "input_player1_a", "\"9\"");
        // original line untouched, new key appended
        assert!(out.contains("input_player1_a_btn = \"5\""));
        assert!(out.contains("input_player1_a = \"9\""));
    }

    // ── Mupen64Plus (Tier B) ────────────────────────────────────────────────

    /// The project's Bluetooth Xbox pad as SDL resolves it on this machine
    /// (matching the on-disk `mupen64plus.cfg` auto-config: Start=button(11),
    /// dpad=hat(0 …), C-buttons on the right stick, triggers on axes 5/4).
    fn device_mapping() -> PadMapping {
        let mut m = PadMapping::new();
        m.set_button(SDL_BTN_A, RawElement::Button(0));
        m.set_button(SDL_BTN_B, RawElement::Button(1));
        m.set_button(SDL_BTN_X, RawElement::Button(2));
        m.set_button(SDL_BTN_Y, RawElement::Button(3));
        m.set_button(SDL_BTN_BACK, RawElement::Button(10));
        m.set_button(SDL_BTN_GUIDE, RawElement::Button(8));
        m.set_button(SDL_BTN_START, RawElement::Button(11));
        m.set_button(SDL_BTN_LSHOULDER, RawElement::Button(4));
        m.set_button(SDL_BTN_RSHOULDER, RawElement::Button(5));
        m.set_button(SDL_BTN_DPAD_UP, RawElement::Hat(0, HatDir::Up));
        m.set_button(SDL_BTN_DPAD_DOWN, RawElement::Hat(0, HatDir::Down));
        m.set_button(SDL_BTN_DPAD_LEFT, RawElement::Hat(0, HatDir::Left));
        m.set_button(SDL_BTN_DPAD_RIGHT, RawElement::Hat(0, HatDir::Right));
        m.set_axis(SDL_AXIS_LEFTX, RawElement::Axis(0));
        m.set_axis(SDL_AXIS_LEFTY, RawElement::Axis(1));
        m.set_axis(SDL_AXIS_RIGHTX, RawElement::Axis(2));
        m.set_axis(SDL_AXIS_RIGHTY, RawElement::Axis(3));
        m.set_axis(SDL_AXIS_TRIGGERLEFT, RawElement::Axis(5));
        m.set_axis(SDL_AXIS_TRIGGERRIGHT, RawElement::Axis(4));
        m
    }

    #[test]
    fn mupen_start_uses_raw_joystick_index_not_normalized() {
        // The whole reason Mupen is Tier B: normalized Start is btn:9, but the
        // pad's raw joystick button is 11. A naive `button(9)` would bind wrong.
        let a = encode_mupen_digital("start", W3cInput::Button(9), &device_mapping()).unwrap();
        assert_eq!(a.key, "Start");
        assert_eq!(a.value, "\"button(11)\"");
    }

    #[test]
    fn mupen_face_buttons_map_directly() {
        let m = device_mapping();
        let a = encode_mupen_digital("a", W3cInput::Button(0), &m).unwrap();
        assert_eq!((a.key.as_str(), a.value.as_str()), ("A Button", "\"button(0)\""));
        let b = encode_mupen_digital("b", W3cInput::Button(1), &m).unwrap();
        assert_eq!((b.key.as_str(), b.value.as_str()), ("B Button", "\"button(1)\""));
    }

    #[test]
    fn mupen_dpad_resolves_to_hat() {
        let a = encode_mupen_digital("dpad_up", W3cInput::Button(12), &device_mapping()).unwrap();
        assert_eq!((a.key.as_str(), a.value.as_str()), ("DPad U", "\"hat(0 Up)\""));
    }

    #[test]
    fn mupen_c_buttons_from_right_stick_axes() {
        let m = device_mapping();
        let l = encode_mupen_digital("c_left", W3cInput::Axis(2, AxisSign::Neg), &m).unwrap();
        assert_eq!((l.key.as_str(), l.value.as_str()), ("C Button L", "\"axis(2-)\""));
        let u = encode_mupen_digital("c_up", W3cInput::Axis(3, AxisSign::Neg), &m).unwrap();
        assert_eq!((u.key.as_str(), u.value.as_str()), ("C Button U", "\"axis(3-)\""));
    }

    #[test]
    fn mupen_triggers_as_axes_and_as_buttons() {
        let m = device_mapping();
        // Captured as the analog trigger (W3C btn:6 = LT → SDL trigger axis 4 → raw axis 5).
        let trig = encode_mupen_digital("l", W3cInput::Button(6), &m).unwrap();
        assert_eq!((trig.key.as_str(), trig.value.as_str()), ("L Trig", "\"axis(5+)\""));
        // Captured as the shoulder button instead (W3C btn:4 = LB → raw button 4).
        let shoulder = encode_mupen_digital("l", W3cInput::Button(4), &m).unwrap();
        assert_eq!(shoulder.value, "\"button(4)\"");
    }

    #[test]
    fn mupen_analog_stick_folds_into_paired_axes() {
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("stick_left".into(), "axis:0-".into());
        b.insert("stick_right".into(), "axis:0+".into());
        b.insert("stick_up".into(), "axis:1-".into());
        b.insert("stick_down".into(), "axis:1+".into());
        let (out, unencoded) = materialize_mupen(&b, &m);
        assert!(unencoded.is_empty());
        let x = out.iter().find(|a| a.key == "X Axis").unwrap();
        assert_eq!(x.value, "\"axis(0-,0+)\"");
        let y = out.iter().find(|a| a.key == "Y Axis").unwrap();
        assert_eq!(y.value, "\"axis(1-,1+)\"");
    }

    #[test]
    fn mupen_unresolvable_input_is_unencoded_not_guessed() {
        // Empty mapping: nothing resolves, so everything is reported unencoded
        // rather than written as a wrong index.
        let mut b = BTreeMap::new();
        b.insert("start".into(), "btn:9".into());
        let (out, unencoded) = materialize_mupen(&b, &PadMapping::new());
        assert!(out.is_empty());
        assert_eq!(unencoded, vec!["start".to_string()]);
    }

    #[test]
    fn mupen_full_profile_matches_on_disk_shape() {
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("a".into(), "btn:0".into());
        b.insert("b".into(), "btn:1".into());
        b.insert("start".into(), "btn:9".into());
        b.insert("z".into(), "btn:7".into()); // RT button → raw axis 4? btn:7 is RT (axis)
        b.insert("dpad_up".into(), "btn:12".into());
        let (out, unencoded) = materialize_mupen(&b, &m);
        let find = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(find("A Button"), Some("\"button(0)\""));
        assert_eq!(find("Start"), Some("\"button(11)\""));
        assert_eq!(find("DPad U"), Some("\"hat(0 Up)\""));
        // z captured as RT (btn:7 → trigger axis) is legal; just assert it encoded.
        assert!(find("Z Trig").is_some());
        assert!(unencoded.is_empty());
    }

    #[test]
    fn set_ini_section_replaces_only_within_target_section() {
        let cfg = "\
[Input-SDL-Control1]
mode = 2
Start = \"button(11)\"

[Input-SDL-Control2]
mode = 2
Start = \"button(99)\"
";
        let out = set_ini_key_in_section(cfg, "Input-SDL-Control1", "mode", "0");
        // Control1 changed…
        assert!(out.contains("[Input-SDL-Control1]\nmode = 0"));
        // …Control2 untouched.
        assert!(out.contains("[Input-SDL-Control2]\nmode = 2"));
        assert!(out.contains("Start = \"button(99)\""));
    }

    #[test]
    fn set_ini_section_handles_keys_with_spaces() {
        let cfg = "[Input-SDL-Control1]\nA Button = \"button(0)\"\nB Button = \"button(1)\"\n";
        let out = set_ini_key_in_section(cfg, "Input-SDL-Control1", "A Button", "\"button(5)\"");
        assert!(out.contains("A Button = \"button(5)\""));
        assert!(out.contains("B Button = \"button(1)\""));
        assert_eq!(out.matches("A Button").count(), 1);
    }

    #[test]
    fn set_ini_section_inserts_missing_key_within_section() {
        let cfg = "[Input-SDL-Control1]\nmode = 0\n\n[Audio]\nx = 1\n";
        let out = set_ini_key_in_section(cfg, "Input-SDL-Control1", "Z Trig", "\"button(7)\"");
        // Inserted inside Control1, before the blank line / next section.
        let ctrl1 = out.split("[Audio]").next().unwrap();
        assert!(ctrl1.contains("Z Trig = \"button(7)\""));
        assert!(out.contains("[Audio]\nx = 1"));
    }

    // ── Mednafen ─────────────────────────────────────────────────────────────

    const GUID: &str = "0005045e028e11300009000f00000000";

    #[test]
    fn mednafen_faces_and_shoulders_embed_guid() {
        let m = device_mapping();
        let a = encode_mednafen("virtualboy", "a", W3cInput::Button(0), &m, GUID, None).unwrap();
        assert_eq!(a.key, "vb.input.builtin.gamepad.a");
        assert_eq!(a.value, format!("joystick 0x{GUID} button_0"));
        // VB "L" captured as the shoulder button (W3C btn:4 → raw button 4).
        let l = encode_mednafen("virtualboy", "l", W3cInput::Button(4), &m, GUID, None).unwrap();
        assert_eq!(l.key, "vb.input.builtin.gamepad.lt");
        assert_eq!(l.value, format!("joystick 0x{GUID} button_4"));
    }

    #[test]
    fn mednafen_right_dpad_uses_right_stick_axes() {
        let m = device_mapping();
        // Right d-pad up captured as right-stick Y negative (W3C axis:3- → raw axis 3).
        let up = encode_mednafen(
            "virtualboy",
            "r_dpad_up",
            W3cInput::Axis(3, AxisSign::Neg),
            &m,
            GUID,
            None,
        )
        .unwrap();
        assert_eq!(up.key, "vb.input.builtin.gamepad.up-r");
        assert_eq!(up.value, format!("joystick 0x{GUID} abs_3-"));
    }

    #[test]
    fn mednafen_left_dpad_uses_config_axis_convention_not_probe_hat() {
        // The probe resolves the d-pad to a hat (device_mapping has Hat(0,*)), but
        // Mednafen flattens it to axes. With the config's (x=6, y=7) convention,
        // the d-pad is encoded as abs halves — never the unaddressable hat.
        let m = device_mapping();
        let dpad = Some((6, 7));
        let up =
            encode_mednafen("virtualboy", "dpad_up", W3cInput::Button(12), &m, GUID, dpad).unwrap();
        assert_eq!(up.value, format!("joystick 0x{GUID} abs_7-"));
        let down =
            encode_mednafen("virtualboy", "dpad_down", W3cInput::Button(13), &m, GUID, dpad)
                .unwrap();
        assert_eq!(down.value, format!("joystick 0x{GUID} abs_7+"));
        let left =
            encode_mednafen("virtualboy", "dpad_left", W3cInput::Button(14), &m, GUID, dpad)
                .unwrap();
        assert_eq!(left.value, format!("joystick 0x{GUID} abs_6-"));
        let right =
            encode_mednafen("virtualboy", "dpad_right", W3cInput::Button(15), &m, GUID, dpad)
                .unwrap();
        assert_eq!(right.value, format!("joystick 0x{GUID} abs_6+"));
    }

    #[test]
    fn mednafen_dpad_unencoded_when_no_config_convention() {
        // No prior axis convention + probe sees a hat ⇒ never guess: report the
        // d-pad unencoded for Mednafen's in-game config (the escape hatch).
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("dpad_up".into(), "btn:12".into());
        b.insert("a".into(), "btn:0".into());
        let (out, unencoded) = materialize_mednafen("virtualboy", &b, &m, GUID, None);
        assert_eq!(unencoded, vec!["dpad_up".to_string()]);
        assert!(out.iter().any(|x| x.key == "vb.input.builtin.gamepad.a"));
    }

    #[test]
    fn mednafen_reads_guid_from_existing_config() {
        let cfg = format!(
            "vb.input.builtin.gamepad.a joystick 0x{GUID} button_0\n\
             vb.input.builtin.gamepad.left-l joystick 0x{GUID} abs_6-\n"
        );
        assert_eq!(mednafen_config_guid(&cfg, "virtualboy").as_deref(), Some(GUID));
        assert_eq!(mednafen_config_guid("", "virtualboy"), None);
    }

    #[test]
    fn mednafen_reads_dpad_axes_from_existing_config() {
        let cfg = format!(
            "vb.input.builtin.gamepad.up-l joystick 0x{GUID} abs_7-\n\
             vb.input.builtin.gamepad.down-l joystick 0x{GUID} abs_7+\n\
             vb.input.builtin.gamepad.left-l joystick 0x{GUID} abs_6-\n\
             vb.input.builtin.gamepad.right-l joystick 0x{GUID} abs_6+\n"
        );
        assert_eq!(mednafen_config_dpad_axes(&cfg, "virtualboy"), Some((6, 7)));
        // Missing the d-pad entirely ⇒ None (caller leaves d-pad to escape hatch).
        assert_eq!(mednafen_config_dpad_axes("", "virtualboy"), None);
    }

    fn ps_bindings() -> std::collections::BTreeMap<String, String> {
        // Standard W3C layout: South=0, East=1, West=2, North=3, Back=8, Start=9,
        // LB/RB=4/5, LT/RT=6/7, L3/R3=10/11, DPad=12..15, sticks axes 0..3.
        [
            ("cross", "btn:0"),
            ("circle", "btn:1"),
            ("square", "btn:2"),
            ("triangle", "btn:3"),
            ("l1", "btn:4"),
            ("r1", "btn:5"),
            ("l2", "btn:6"),
            ("r2", "btn:7"),
            ("select", "btn:8"),
            ("start", "btn:9"),
            ("l3", "btn:10"),
            ("r3", "btn:11"),
            ("dpad_up", "btn:12"),
            ("dpad_left", "btn:14"),
            ("l_stick_up", "axis:1-"),
            ("l_stick_right", "axis:0+"),
            ("r_stick_left", "axis:2-"),
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
    }

    fn find_val<'a>(a: &'a [Assignment], key: &str) -> Option<&'a str> {
        a.iter().find(|x| x.key == key).map(|x| x.value.as_str())
    }

    #[test]
    fn pcsx2_emits_positional_face_tokens_and_default_prefix() {
        let (a, un) = materialize_ps_named(&ps_bindings(), PsDialect::Pcsx2, "SDL-0");
        assert!(un.is_empty(), "all standard inputs encode: {un:?}");
        assert_eq!(find_val(&a, "Cross"), Some("SDL-0/FaceSouth"));
        assert_eq!(find_val(&a, "Triangle"), Some("SDL-0/FaceNorth"));
        assert_eq!(find_val(&a, "Select"), Some("SDL-0/Back"));
        assert_eq!(find_val(&a, "L2"), Some("SDL-0/+LeftTrigger"));
        assert_eq!(find_val(&a, "Up"), Some("SDL-0/DPadUp"));
        assert_eq!(find_val(&a, "LUp"), Some("SDL-0/-LeftY"));
        assert_eq!(find_val(&a, "LRight"), Some("SDL-0/+LeftX"));
        assert_eq!(find_val(&a, "RLeft"), Some("SDL-0/-RightX"));
    }

    #[test]
    fn duckstation_uses_sdl_letters_tilde_triggers_and_read_prefix() {
        let (a, _) = materialize_ps_named(&ps_bindings(), PsDialect::DuckStation, "SDL-1");
        assert_eq!(find_val(&a, "Cross"), Some("SDL-1/A"));
        assert_eq!(find_val(&a, "Circle"), Some("SDL-1/B"));
        assert_eq!(find_val(&a, "Square"), Some("SDL-1/X"));
        assert_eq!(find_val(&a, "Triangle"), Some("SDL-1/Y"));
        assert_eq!(find_val(&a, "L2"), Some("SDL-1/+LeftTrigger~"));
        assert_eq!(find_val(&a, "L1"), Some("SDL-1/LeftShoulder"));
    }

    #[test]
    fn ps_reads_device_prefix_from_existing_section() {
        let cfg = "[Pad1]\nType = DualShock2\nCross = SDL-2/FaceSouth\nUp = SDL-2/DPadUp\n";
        assert_eq!(ps_device_prefix(cfg, "Pad1").as_deref(), Some("SDL-2"));
        // Wrong section / unconfigured ⇒ None (caller defaults to SDL-0).
        assert_eq!(ps_device_prefix(cfg, "Pad2"), None);
        assert_eq!(ps_device_prefix("[Pad1]\nType = DualShock2\n", "Pad1"), None);
    }

    #[test]
    fn ps_unmappable_input_or_element_is_unencoded_not_guessed() {
        let b = [
            ("guide_extra", "btn:3"), // unknown console input id
            ("cross", "btn:99"),      // out-of-layout element
        ]
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
        let (a, un) = materialize_ps_named(&b, PsDialect::Pcsx2, "SDL-0");
        assert!(a.is_empty());
        assert_eq!(un.len(), 2);
    }

    #[test]
    fn parses_mednafen_calculated_joystick_id_from_startup_log() {
        // Real Mednafen 1.32.1 startup enumeration (indented two spaces). The
        // calculated ID here is the evdev-derived hardware ID, NOT the SDL GUID.
        let log = "\
 Initializing joysticks...\n\
  ID: 0x0005045e028e11300009000f00000000 - Xbox Wireless Controller\n\
 Loading \"game.vb\"...\n";
        assert_eq!(
            parse_mednafen_joystick_id(log).as_deref(),
            Some("0005045e028e11300009000f00000000")
        );
        // No joystick line ⇒ None; malformed/short hex ⇒ None.
        assert_eq!(parse_mednafen_joystick_id("Initializing joysticks...\n"), None);
        assert_eq!(parse_mednafen_joystick_id("  ID: 0xdeadbeef - Short\n"), None);
    }

    #[test]
    fn set_mednafen_setting_replaces_in_place_and_appends() {
        let cfg = "vb.input.builtin.gamepad.a joystick 0x0 button_0\nfoo.bar 1\n";
        let out = set_mednafen_setting(
            cfg,
            "vb.input.builtin.gamepad.a",
            &format!("joystick 0x{GUID} button_3"),
        );
        assert!(out.contains(&format!(
            "vb.input.builtin.gamepad.a joystick 0x{GUID} button_3"
        )));
        assert!(out.contains("foo.bar 1")); // untouched
        assert_eq!(out.matches("vb.input.builtin.gamepad.a").count(), 1);
        // Absent key is appended.
        let out2 = set_mednafen_setting(&out, "vb.input.builtin.gamepad.b", "joystick 0x0 button_1");
        assert!(out2.contains("vb.input.builtin.gamepad.b joystick 0x0 button_1"));
    }

    // ── Snes9x (Tier B) ──────────────────────────────────────────────────────

    #[test]
    fn snes9x_face_and_trigger_use_raw_indices() {
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("a".into(), "btn:0".into()); // South → raw button 0
        b.insert("l".into(), "btn:6".into()); // LT button → SDL trigger axis 4 → raw axis 5
        let (out, un) = materialize_snes9x(&b, &m, 1);
        let find = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(find("A"), Some("Joystick 1 Button 0"));
        assert_eq!(find("L"), Some("Joystick 1 Axis 5 + 40%"));
        assert!(un.is_empty());
    }

    #[test]
    fn snes9x_honours_device_number() {
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("b".into(), "btn:1".into());
        let (out, _) = materialize_snes9x(&b, &m, 3);
        assert_eq!(out[0].value, "Joystick 3 Button 1");
    }

    #[test]
    fn snes9x_dpad_hat_is_unencoded_not_guessed() {
        // The pad reports its d-pad as a hat; snes9x has no hat token, so the
        // direction must not be written as a synthesized axis.
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("dpad_up".into(), "btn:12".into());
        let (out, un) = materialize_snes9x(&b, &m, 1);
        assert!(out.is_empty());
        assert_eq!(un, vec!["dpad_up".to_string()]);
    }

    #[test]
    fn snes9x_reads_device_number_and_existing_binding() {
        let cfg = "\
[Joypad 0]
A        = Joystick 1 Button 0
Up       = Joystick 1 Axis 7 + 40%
Down     = Unset

[Joypad 1]
A        = Joystick 2 Button 0
";
        assert_eq!(snes9x_joystick_number(cfg, "Joypad 0"), Some(1));
        assert_eq!(snes9x_joystick_number(cfg, "Joypad 1"), Some(2));
        // Up has a real binding (so we won't nag); Down is Unset.
        assert!(snes9x_has_binding(cfg, "Joypad 0", "dpad_up"));
        assert!(!snes9x_has_binding(cfg, "Joypad 0", "dpad_down"));
        // A key not present in the section.
        assert!(!snes9x_has_binding(cfg, "Joypad 0", "start"));
    }

    #[test]
    fn snes9x_unknown_input_is_unencoded() {
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("c_up".into(), "btn:3".into()); // not a SNES pad input
        let (out, un) = materialize_snes9x(&b, &m, 1);
        assert!(out.is_empty());
        assert_eq!(un, vec!["c_up".to_string()]);
    }

    // ── mGBA (Tier B) ─────────────────────────────────────────────────────────

    #[test]
    fn mgba_section_per_platform() {
        assert_eq!(mgba_section("gba").as_deref(), Some("gba.input.SDLB"));
        assert_eq!(mgba_section("gb").as_deref(), Some("gb.input.SDLB"));
        assert_eq!(mgba_section("gbc").as_deref(), Some("gb.input.SDLB"));
        assert_eq!(mgba_section("nes"), None);
    }

    #[test]
    fn mgba_profile_section_keys_by_joystick_name() {
        // The Qt frontend loads (and re-saves) this section last, overriding SDLB.
        assert_eq!(
            mgba_profile_section("gba", "Xbox Wireless Controller").as_deref(),
            Some("gba.input-profile.Xbox Wireless Controller")
        );
        assert_eq!(
            mgba_profile_section("gbc", "Xbox Wireless Controller").as_deref(),
            Some("gb.input-profile.Xbox Wireless Controller")
        );
        assert_eq!(mgba_profile_section("nes", "Any Pad"), None);
    }

    #[test]
    fn mgba_button_uses_raw_index() {
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("a".into(), "btn:0".into()); // South → raw button 0
        b.insert("start".into(), "btn:9".into()); // Start → raw button 11 on this pad
        let (out, un) = materialize_mgba(&b, &m);
        let find = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(find("keyA"), Some("0"));
        assert_eq!(find("keyStart"), Some("11"));
        assert!(un.is_empty());
    }

    #[test]
    fn mgba_dpad_hat_uses_inverted_gamekey_value() {
        // The pad's d-pad probes as a hat; mGBA addresses hats natively, storing
        // the GAME KEY enum as the value (Up=6, Down=7, Left=5, Right=4).
        let m = device_mapping();
        let mut b = BTreeMap::new();
        b.insert("dpad_up".into(), "btn:12".into());
        b.insert("dpad_left".into(), "btn:14".into());
        let (out, un) = materialize_mgba(&b, &m);
        let find = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(find("hat0Up"), Some("6"));
        assert_eq!(find("hat0Left"), Some("5"));
        assert!(un.is_empty());
    }

    #[test]
    fn mgba_analog_trigger_emits_axis_and_threshold() {
        let m = device_mapping();
        let mut b = BTreeMap::new();
        // L captured as the analog left trigger: W3C btn:6 → SDL trigger axis 4 →
        // raw axis 5 (positive half on this pad).
        b.insert("l".into(), "btn:6".into());
        let (out, un) = materialize_mgba(&b, &m);
        let find = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(find("axisLAxis"), Some("+5"));
        assert_eq!(find("axisLValue"), Some("16384"));
        assert!(un.is_empty());
    }

    #[test]
    fn mgba_negative_axis_threshold_is_signed() {
        let m = device_mapping();
        let mut b = BTreeMap::new();
        // Contrived: bind L to the left-stick negative half to check the sign.
        b.insert("l".into(), "axis:0-".into());
        let (out, _) = materialize_mgba(&b, &m);
        let find = |k: &str| out.iter().find(|a| a.key == k).map(|a| a.value.as_str());
        assert_eq!(find("axisLAxis"), Some("-0"));
        assert_eq!(find("axisLValue"), Some("-16384"));
    }

    #[test]
    fn mgba_unknown_input_and_unresolvable_are_unencoded() {
        let mut b = BTreeMap::new();
        b.insert("c_up".into(), "btn:3".into()); // not a GB/GBA input
        b.insert("a".into(), "btn:0".into()); // resolvable id, but empty mapping
        let (out, un) = materialize_mgba(&b, &PadMapping::new());
        assert!(out.is_empty());
        assert_eq!(un.len(), 2);
        assert!(un.contains(&"c_up".to_string()));
        assert!(un.contains(&"a".to_string()));
    }

    #[test]
    fn set_ini_compact_writes_without_spaces() {
        let cfg = "[gba.input.SDLB]\nkeyA=0\nkeyB=2\n";
        let out = set_ini_key_in_section_compact(cfg, "gba.input.SDLB", "keyA", "5");
        assert!(out.contains("keyA=5"));
        assert!(!out.contains("keyA = 5"));
        assert!(out.contains("keyB=2")); // untouched
        // Absent key is inserted compactly within the section.
        let out2 = set_ini_key_in_section_compact(&out, "gba.input.SDLB", "hat0Up", "6");
        assert!(out2.contains("hat0Up=6"));
    }
}
