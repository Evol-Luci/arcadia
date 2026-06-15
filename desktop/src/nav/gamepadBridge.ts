// WebKitGTK's native Web Gamepad API does not reliably surface controllers on
// Linux, so the Rust side reads them via evdev and emits `arcadia://gamepad`
// snapshots. Here we cache the latest snapshot and override
// `navigator.getGamepads()` so the live tester and spatial nav work unchanged.
// Native gamepads (macOS / Windows / a working WebKit build) still take
// precedence when present.

import { listen } from "@tauri-apps/api/event";

interface PadButton {
  pressed: boolean;
  value: number;
}

interface PadSnapshot {
  index: number;
  id: string;
  buttons: PadButton[];
  axes: number[];
}

let snapshots: PadSnapshot[] = [];
let installed = false;

function toGamepad(s: PadSnapshot): Gamepad {
  return {
    id: s.id,
    index: s.index,
    connected: true,
    mapping: "standard",
    timestamp: performance.now(),
    axes: s.axes,
    buttons: s.buttons.map((b) => ({
      pressed: b.pressed,
      touched: b.pressed,
      value: b.value,
    })),
    vibrationActuator: null,
    hapticActuators: [],
  } as unknown as Gamepad;
}

/** Install the polyfill and start listening for Rust-side gamepad snapshots. */
export async function startGamepadBridge(): Promise<() => void> {
  if (!installed) {
    const native = navigator.getGamepads?.bind(navigator);
    navigator.getGamepads = function () {
      if (native) {
        const live = Array.from(native()).filter(Boolean);
        if (live.length > 0) return native();
      }
      return snapshots.map(toGamepad);
    } as typeof navigator.getGamepads;
    installed = true;
  }

  const unlisten = await listen<PadSnapshot[]>("arcadia://gamepad", (e) => {
    snapshots = e.payload ?? [];
  });
  return unlisten;
}
