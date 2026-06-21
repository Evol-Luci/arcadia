//! Per-console controller layouts — the target side of the input-setup module.
//!
//! There is no universal gamepad: an NES pad has two buttons, an N64 pad has an
//! analog stick plus four C-buttons and a Z trigger, a DualShock has two sticks
//! and four shoulder buttons. The setup UI maps the user's *physical* pad (a W3C
//! Standard Gamepad, captured live in the shell) onto *these* per-system targets,
//! one console at a time. This catalogue is the single source of truth for "what
//! inputs does this console have"; it is shared by the capture UI, the per-adapter
//! writers, and the stored [`crate::config::SystemControllerProfile`].
//!
//! Each entry's `id` is a stable, canonical token (the key used in a profile's
//! `bindings` map). Directional groups are flattened to one entry per discrete
//! capturable direction so every entry maps to exactly one W3C descriptor:
//! a d-pad is four `Dpad` entries, an analog stick is four `Stick` entries.

/// The capture/encode shape of a console input. Groups (`Dpad`, `Stick`) are
/// flattened in the catalogue — the kind tells the UI how to group and the
/// encoder how to translate (a `Stick` direction becomes an axis on pads that
/// report one, a button on pads that don't).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    Button,
    Dpad,
    Stick,
    Trigger,
}

/// One discrete, capturable input on a console's controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct ConsoleInput {
    /// Canonical token; the key in a profile's `bindings` map.
    pub id: &'static str,
    /// Human label for the capture UI ("A", "C ▲", "Left Stick ◀").
    pub label: &'static str,
    pub kind: InputKind,
}

/// The full input set of one console's controller.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct ConsolePad {
    /// Platform id, matching [`crate::platforms`] ("nes", "snes", "n64", "ps1").
    pub system: &'static str,
    pub inputs: &'static [ConsoleInput],
}

const fn btn(id: &'static str, label: &'static str) -> ConsoleInput {
    ConsoleInput { id, label, kind: InputKind::Button }
}
const fn trig(id: &'static str, label: &'static str) -> ConsoleInput {
    ConsoleInput { id, label, kind: InputKind::Trigger }
}

// A standard 4-way d-pad, shared verbatim by every console here.
const DPAD: [ConsoleInput; 4] = [
    ConsoleInput { id: "dpad_up", label: "D-Pad ▲", kind: InputKind::Dpad },
    ConsoleInput { id: "dpad_down", label: "D-Pad ▼", kind: InputKind::Dpad },
    ConsoleInput { id: "dpad_left", label: "D-Pad ◀", kind: InputKind::Dpad },
    ConsoleInput { id: "dpad_right", label: "D-Pad ▶", kind: InputKind::Dpad },
];

const NES_INPUTS: &[ConsoleInput] = &[
    DPAD[0], DPAD[1], DPAD[2], DPAD[3],
    btn("a", "A"),
    btn("b", "B"),
    btn("start", "Start"),
    btn("select", "Select"),
];

// Game Boy / Game Boy Color: d-pad, two face buttons, Start/Select. (mGBA shares
// one "gb" core for DMG and CGB, so both Arcadia systems use this layout.)
const GB_INPUTS: &[ConsoleInput] = &[
    DPAD[0], DPAD[1], DPAD[2], DPAD[3],
    btn("b", "B"),
    btn("a", "A"),
    btn("start", "Start"),
    btn("select", "Select"),
];

// Game Boy Advance: the GB set plus the L/R shoulder buttons.
const GBA_INPUTS: &[ConsoleInput] = &[
    DPAD[0], DPAD[1], DPAD[2], DPAD[3],
    btn("b", "B"),
    btn("a", "A"),
    btn("l", "L"),
    btn("r", "R"),
    btn("start", "Start"),
    btn("select", "Select"),
];

const SNES_INPUTS: &[ConsoleInput] = &[
    DPAD[0], DPAD[1], DPAD[2], DPAD[3],
    btn("b", "B"),
    btn("a", "A"),
    btn("y", "Y"),
    btn("x", "X"),
    btn("l", "L"),
    btn("r", "R"),
    btn("start", "Start"),
    btn("select", "Select"),
];

const N64_INPUTS: &[ConsoleInput] = &[
    DPAD[0], DPAD[1], DPAD[2], DPAD[3],
    ConsoleInput { id: "stick_up", label: "Analog ▲", kind: InputKind::Stick },
    ConsoleInput { id: "stick_down", label: "Analog ▼", kind: InputKind::Stick },
    ConsoleInput { id: "stick_left", label: "Analog ◀", kind: InputKind::Stick },
    ConsoleInput { id: "stick_right", label: "Analog ▶", kind: InputKind::Stick },
    btn("a", "A"),
    btn("b", "B"),
    btn("c_up", "C ▲"),
    btn("c_down", "C ▼"),
    btn("c_left", "C ◀"),
    btn("c_right", "C ▶"),
    btn("l", "L"),
    btn("r", "R"),
    btn("z", "Z"),
    btn("start", "Start"),
];

const PS1_INPUTS: &[ConsoleInput] = &[
    DPAD[0], DPAD[1], DPAD[2], DPAD[3],
    ConsoleInput { id: "l_stick_up", label: "Left Stick ▲", kind: InputKind::Stick },
    ConsoleInput { id: "l_stick_down", label: "Left Stick ▼", kind: InputKind::Stick },
    ConsoleInput { id: "l_stick_left", label: "Left Stick ◀", kind: InputKind::Stick },
    ConsoleInput { id: "l_stick_right", label: "Left Stick ▶", kind: InputKind::Stick },
    ConsoleInput { id: "r_stick_up", label: "Right Stick ▲", kind: InputKind::Stick },
    ConsoleInput { id: "r_stick_down", label: "Right Stick ▼", kind: InputKind::Stick },
    ConsoleInput { id: "r_stick_left", label: "Right Stick ◀", kind: InputKind::Stick },
    ConsoleInput { id: "r_stick_right", label: "Right Stick ▶", kind: InputKind::Stick },
    btn("cross", "✕"),
    btn("circle", "○"),
    btn("square", "□"),
    btn("triangle", "△"),
    btn("l1", "L1"),
    trig("l2", "L2"),
    btn("r1", "R1"),
    trig("r2", "R2"),
    btn("l3", "L3"),
    btn("r3", "R3"),
    btn("start", "Start"),
    btn("select", "Select"),
];

// Virtual Boy has TWO d-pads (left + right) plus A/B, the L/R shoulder buttons,
// Start/Select, and Mednafen's rapid-fire A/B variants. The platform id is
// `virtualboy` (matching platforms.rs); Mednafen's own config prefix is `vb`,
// which the encoder translates. The right d-pad is conventionally bound to the
// pad's right analog stick (as the user's working mednafen.cfg does), so it is
// catalogued as Stick inputs — the live tester then captures right-stick axes.
const VB_INPUTS: &[ConsoleInput] = &[
    DPAD[0], DPAD[1], DPAD[2], DPAD[3],
    ConsoleInput { id: "r_dpad_up", label: "Right ▲", kind: InputKind::Stick },
    ConsoleInput { id: "r_dpad_down", label: "Right ▼", kind: InputKind::Stick },
    ConsoleInput { id: "r_dpad_left", label: "Right ◀", kind: InputKind::Stick },
    ConsoleInput { id: "r_dpad_right", label: "Right ▶", kind: InputKind::Stick },
    btn("a", "A"),
    btn("b", "B"),
    btn("l", "L"),
    btn("r", "R"),
    btn("start", "Start"),
    btn("select", "Select"),
    btn("rapid_a", "Rapid A"),
    btn("rapid_b", "Rapid B"),
];

/// The catalogue. Headline systems first; the entries below span the variety
/// (2-button → shoulder buttons → analog + C-buttons → dual-stick + analog
/// triggers → dual d-pad) and prove the per-system model before the rest are
/// filled in.
pub const CONSOLE_PADS: &[ConsolePad] = &[
    ConsolePad { system: "nes", inputs: NES_INPUTS },
    ConsolePad { system: "snes", inputs: SNES_INPUTS },
    ConsolePad { system: "gb", inputs: GB_INPUTS },
    ConsolePad { system: "gbc", inputs: GB_INPUTS },
    ConsolePad { system: "gba", inputs: GBA_INPUTS },
    ConsolePad { system: "n64", inputs: N64_INPUTS },
    ConsolePad { system: "ps1", inputs: PS1_INPUTS },
    // PS2's DualShock2 shares PS1's (analog) DualShock control surface.
    ConsolePad { system: "ps2", inputs: PS1_INPUTS },
    ConsolePad { system: "virtualboy", inputs: VB_INPUTS },
];

/// The target layout for a system, or `None` if not yet catalogued (the setup
/// UI then offers nothing to map and the writers report `Unsupported`).
pub fn pad_for(system: &str) -> Option<&'static ConsolePad> {
    CONSOLE_PADS.iter().find(|p| p.system == system)
}

impl ConsolePad {
    /// Whether `input_id` is a real input on this console — used to validate a
    /// profile's keys before materializing it to emulator config.
    pub fn has_input(&self, input_id: &str) -> bool {
        self.inputs.iter().any(|i| i.id == input_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_pad_has_unique_input_ids() {
        for pad in CONSOLE_PADS {
            let mut seen = std::collections::HashSet::new();
            for input in pad.inputs {
                assert!(
                    seen.insert(input.id),
                    "duplicate input id {} in {}",
                    input.id,
                    pad.system
                );
            }
        }
    }

    #[test]
    fn pad_lookup_resolves_and_misses() {
        assert!(pad_for("n64").is_some());
        assert!(pad_for("nonexistent").is_none());
    }

    #[test]
    fn variety_is_represented() {
        // NES is minimal, N64 has an analog stick + C-buttons, PS1 has two sticks.
        assert_eq!(pad_for("nes").unwrap().inputs.len(), 8);
        let n64 = pad_for("n64").unwrap();
        assert!(n64.has_input("stick_up"));
        assert!(n64.has_input("c_left"));
        assert!(n64.has_input("z"));
        let ps1 = pad_for("ps1").unwrap();
        assert!(ps1.has_input("l_stick_up"));
        assert!(ps1.has_input("r_stick_right"));
        assert!(ps1.has_input("cross"));
    }

    #[test]
    fn dpad_is_present_everywhere() {
        for pad in CONSOLE_PADS {
            assert!(pad.has_input("dpad_up"), "{} missing dpad", pad.system);
        }
    }
}
