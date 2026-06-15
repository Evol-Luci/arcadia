//! Gamepad bridge.
//!
//! WebKitGTK's native Web Gamepad API does not reliably surface controllers on
//! Linux, so the webview's `navigator.getGamepads()` comes back empty even when
//! a pad is connected and emitting evdev events. We instead read controllers
//! in-process with gilrs (evdev) and push standard-layout snapshots to the
//! webview, where a small polyfill feeds them back into `getGamepads()`.

use gilrs::{Axis, Button, Gilrs};
use serde::Serialize;
use std::time::Duration;
use tauri::{AppHandle, Manager};

#[derive(Serialize, Clone)]
struct PadButton {
    pressed: bool,
    value: f32,
}

#[derive(Serialize, Clone)]
struct PadSnapshot {
    index: usize,
    id: String,
    buttons: Vec<PadButton>,
    axes: Vec<f32>,
}

// W3C "standard" gamepad button order. The frontend (live tester and spatial
// nav) indexes buttons by these positions, so the mapping must match exactly.
const BUTTON_ORDER: [Button; 17] = [
    Button::South,         // 0  A
    Button::East,          // 1  B
    Button::West,          // 2  X
    Button::North,         // 3  Y
    Button::LeftTrigger,   // 4  LB
    Button::RightTrigger,  // 5  RB
    Button::LeftTrigger2,  // 6  LT
    Button::RightTrigger2, // 7  RT
    Button::Select,        // 8  Select / View
    Button::Start,         // 9  Start / Menu
    Button::LeftThumb,     // 10 L3
    Button::RightThumb,    // 11 R3
    Button::DPadUp,        // 12
    Button::DPadDown,      // 13
    Button::DPadLeft,      // 14
    Button::DPadRight,     // 15
    Button::Mode,          // 16 Guide
];

/// Start the background gamepad poller. Runs for the life of the process on its
/// own thread; `Gilrs` is constructed inside the thread so its non-Send evdev
/// handles never cross a thread boundary.
pub fn spawn(handle: AppHandle) {
    std::thread::spawn(move || {
        let mut gilrs = match Gilrs::new() {
            Ok(g) => g,
            Err(e) => {
                tracing::warn!(error = %e, "gamepad bridge disabled: gilrs init failed");
                return;
            }
        };
        tracing::info!("gamepad bridge active (gilrs/evdev)");

        // Avoid spamming the webview with empty snapshots when nothing is
        // connected: emit the empty state once, then stay quiet until a pad
        // shows up again.
        let mut last_empty = false;

        loop {
            // Pump pending events so gilrs refreshes its cached button/axis
            // state before we read it.
            while gilrs.next_event().is_some() {}

            let pads: Vec<PadSnapshot> = gilrs
                .gamepads()
                .enumerate()
                .map(|(index, (_, pad))| {
                    let buttons = BUTTON_ORDER
                        .iter()
                        .map(|&b| {
                            let pressed = pad.is_pressed(b);
                            let value = pad
                                .button_data(b)
                                .map(|d| d.value())
                                .unwrap_or(if pressed { 1.0 } else { 0.0 });
                            PadButton { pressed, value }
                        })
                        .collect();
                    // Web convention: stick +Y points down. gilrs reports +Y up,
                    // so invert the vertical axes.
                    let axes = vec![
                        pad.value(Axis::LeftStickX),
                        -pad.value(Axis::LeftStickY),
                        pad.value(Axis::RightStickX),
                        -pad.value(Axis::RightStickY),
                    ];
                    PadSnapshot {
                        index,
                        id: pad.name().to_string(),
                        buttons,
                        axes,
                    }
                })
                .collect();

            if pads.is_empty() {
                if !last_empty {
                    let _ = handle.emit_all("arcadia://gamepad", &pads);
                    last_empty = true;
                }
            } else {
                last_empty = false;
                let _ = handle.emit_all("arcadia://gamepad", &pads);
            }

            std::thread::sleep(Duration::from_millis(16));
        }
    });
}
