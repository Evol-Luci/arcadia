import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "../api/commands";
import { useStore } from "../store/useStore";
import { Focusable } from "../components/Focusable";
import { setGamepadCapture } from "../nav/spatialNav";
import type { ControllerProfile, HidapiWorkaround } from "../api/types";

// Standard-mapping button indices → readable names, for the live tester.
const BUTTON_NAMES: Record<number, string> = {
  0: "A",
  1: "B",
  2: "X",
  3: "Y",
  4: "LB",
  5: "RB",
  6: "LT",
  7: "RT",
  8: "Select",
  9: "Start",
  10: "L3",
  11: "R3",
  12: "D-Up",
  13: "D-Down",
  14: "D-Left",
  15: "D-Right",
  16: "Guide",
};

// Indices the gamepad schematic places by hand, so the "other buttons" row only
// shows pads that report extra, non-standard inputs.
const PLACED_BUTTONS = new Set([
  0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
]);

// Logical actions a profile can remap. The engine just stores the map; the
// emulator is what ultimately consumes a binding, so this is advisory metadata.
const ACTIONS = [
  "confirm",
  "cancel",
  "menu",
  "quick_save",
  "quick_load",
  "screenshot",
  "fast_forward",
  "exit",
];

interface PadState {
  id: string;
  pressed: number[];
  values: number[];
  axes: number[];
}

export function Controller() {
  const [pad, setPad] = useState<PadState | null>(null);
  const [testing, setTesting] = useState(false);
  const raf = useRef(0);
  const testingRef = useRef(false);
  const prevStart = useRef(false);

  // Capturing locks the gamepad to the tester so it stops driving app
  // navigation. Always released when leaving the view.
  useEffect(() => {
    testingRef.current = testing;
    setGamepadCapture(testing);
  }, [testing]);
  useEffect(() => () => setGamepadCapture(false), []);

  useEffect(() => {
    const poll = () => {
      const pads = navigator.getGamepads?.() ?? [];
      const live = Array.from(pads).find((p) => p);
      if (live) {
        setPad({
          id: live.id,
          pressed: live.buttons.flatMap((b, i) => (b.pressed ? [i] : [])),
          values: live.buttons.map((b) => Math.round(b.value * 100) / 100),
          axes: live.axes.map((a) => Math.round(a * 100) / 100),
        });
        // While capturing, app nav is paused, so let the Start button toggle
        // the tester back off (rising edge so a held press fires once).
        const startPressed = !!live.buttons[9]?.pressed;
        if (startPressed && !prevStart.current && testingRef.current) {
          setTesting(false);
        }
        prevStart.current = startPressed;
      } else {
        setPad(null);
        prevStart.current = false;
      }
      raf.current = requestAnimationFrame(poll);
    };
    raf.current = requestAnimationFrame(poll);
    return () => cancelAnimationFrame(raf.current);
  }, []);

  return (
    <div className="animate-fade-up max-w-3xl">
      <h1 className="mb-6 font-display text-3xl font-black tracking-wide text-glow">
        Controller
      </h1>

      <section className="mb-6">
        <h2 className="font-display text-lg font-bold">Live Tester</h2>
        <p className="mb-3 text-xs text-ink-dim">
          Press buttons on a connected gamepad to verify the OS sees it. Arcadia
          itself navigates with the d-pad, A and B, and the bumpers — start
          testing to lock the controller to the tester so it stops moving the
          menus.
        </p>

        <div className="mb-3 flex items-center gap-3">
          <Focusable
            onActivate={() => setTesting((t) => !t)}
            ariaLabel={testing ? "Stop controller test" : "Start controller test"}
            className={`rounded-lg px-4 py-1.5 text-sm font-semibold ${
              testing ? "bg-secondary text-black" : "bg-primary text-black"
            }`}
          >
            {testing ? "Stop testing" : "Start testing"}
          </Focusable>
          {testing && (
            <span className="text-xs text-ink-dim">
              Capturing — app navigation paused. Press{" "}
              <span className="font-semibold text-ink">Start</span> on the pad,
              or activate this button, to stop.
            </span>
          )}
        </div>

        {!pad ? (
          <div className="glass flex items-center gap-3 rounded-2xl p-5 text-sm text-ink-dim">
            <span className="h-2.5 w-2.5 shrink-0 animate-pulse rounded-full bg-ink-dim/60" />
            No gamepad detected. Connect one and press any button to wake it.
          </div>
        ) : (
          <Gamepad pad={pad} />
        )}
      </section>

      <CompatibilityPanel />
      <ProfilesPanel />
    </div>
  );
}

// Controller compatibility: exposes the SDL-HIDAPI vs xpadneo workaround as a
// user setting. A Bluetooth Xbox pad on the xpadneo driver is seen but delivers
// no input to SDL-input emulators (PCSX2, DuckStation, RPCS3, Mupen64Plus);
// forcing SDL's evdev backend fixes it. "Auto" applies it only when such a pad
// is detected, so it stays out of the way of controllers that work fine.
const HIDAPI_OPTIONS: { value: HidapiWorkaround; label: string; help: string }[] = [
  {
    value: "auto",
    label: "Automatic",
    help: "Apply the fix only when a controller that needs it is connected. Recommended.",
  },
  {
    value: "force",
    label: "Always on",
    help: "Always force SDL's evdev input backend for these emulators.",
  },
  {
    value: "off",
    label: "Off",
    help: "Never change SDL input behaviour.",
  },
];

function CompatibilityPanel() {
  const qc = useQueryClient();
  const setToast = useStore((s) => s.setToast);

  const status = useQuery({
    queryKey: ["hidapi-status"],
    queryFn: () => api.hidapiStatus(),
  });

  const setPolicy = useMutation({
    mutationFn: (p: HidapiWorkaround) => api.setHidapiWorkaround(p),
    onSuccess: () => {
      setToast("Controller compatibility updated.");
      qc.invalidateQueries({ queryKey: ["hidapi-status"] });
      qc.invalidateQueries({ queryKey: ["controller-config"] });
    },
    onError: (e: unknown) => setToast(`Couldn't update: ${String(e)}`),
  });

  const policy = status.data?.policy ?? "auto";
  const effective = status.data?.effective ?? false;
  const detected = status.data?.xpadneo_present ?? false;

  return (
    <section className="mb-6">
      <h2 className="font-display text-lg font-bold">Compatibility</h2>
      <p className="mb-3 text-xs text-ink-dim">
        Some Bluetooth Xbox controllers are detected by emulators but send no
        input until SDL falls back to its evdev backend. This applies that fix
        when launching SDL-input emulators (PCSX2, DuckStation, RPCS3,
        Mupen64Plus).
      </p>

      <div className="glass max-w-sm rounded-2xl p-3">
        <div className="mb-2 flex flex-col gap-2">
          {HIDAPI_OPTIONS.map((opt) => {
            const selected = policy === opt.value;
            return (
              <Focusable
                key={opt.value}
                onActivate={() => !selected && setPolicy.mutate(opt.value)}
                ariaLabel={`Set controller compatibility to ${opt.label}`}
                className={`rounded-lg px-3 py-2 text-left ${
                  selected ? "bg-primary/15 ring-1 ring-primary/50" : ""
                }`}
              >
                <div className="flex items-center gap-2">
                  <span
                    className={`h-2.5 w-2.5 shrink-0 rounded-full ${
                      selected ? "bg-primary" : "bg-ink-dim/40"
                    }`}
                  />
                  <span className="text-sm font-semibold">{opt.label}</span>
                </div>
                <p className="mt-0.5 pl-[18px] text-[11px] leading-snug text-ink-dim">
                  {opt.help}
                </p>
              </Focusable>
            );
          })}
        </div>

        <div className="mt-1 border-t border-ink-dim/10 pt-2 text-[11px] text-ink-dim">
          {detected ? (
            <span>
              A controller needing this fix is{" "}
              <span className="font-semibold text-ink">connected</span>.
            </span>
          ) : (
            <span>No controller needing this fix is currently detected.</span>
          )}{" "}
          {effective ? (
            <span className="font-semibold text-primary">Fix active.</span>
          ) : (
            <span>Fix not being applied.</span>
          )}
        </div>
      </div>
    </section>
  );
}

// Live gamepad schematic — lays inputs out the way they sit on a real pad so a
// lit element tells you *which* physical control fired, not just that one did.
function Gamepad({ pad }: { pad: PadState }) {
  const down = (i: number) => pad.pressed.includes(i);
  const axis = (i: number) => pad.axes[i] ?? 0;
  const value = (i: number) => pad.values[i] ?? 0;

  const extras = pad.pressed.filter((i) => !PLACED_BUTTONS.has(i));

  return (
    <div className="glass overflow-hidden rounded-2xl">
      <div className="flex items-center gap-2 border-b border-primary/10 px-5 py-3">
        <span className="h-2.5 w-2.5 shrink-0 rounded-full bg-primary shadow-glow" />
        <span className="truncate font-mono text-[11px] text-ink-dim">
          {pad.id}
        </span>
      </div>

      <div className="p-6">
        {/* Shoulders + triggers ride above the deck, mirrored left/right. */}
        <div className="mb-7 flex items-start justify-between gap-6">
          <div className="flex w-32 flex-col gap-2">
            <Trigger label="LT" value={value(6)} active={down(6)} />
            <Pip label="LB" active={down(4)} shape="pill" />
          </div>
          <div className="flex w-32 flex-col gap-2">
            <Trigger label="RT" value={value(7)} active={down(7)} />
            <Pip label="RB" active={down(5)} shape="pill" />
          </div>
        </div>

        {/* Main deck: d-pad · center cluster · face diamond. */}
        <div className="grid grid-cols-3 items-center gap-4">
          <Cross
            up={down(12)}
            down={down(13)}
            left={down(14)}
            right={down(15)}
          />

          <div className="flex flex-col items-center gap-2">
            <Pip label="Guide" active={down(16)} shape="pill" />
            <div className="flex gap-2">
              <Pip label="Select" active={down(8)} shape="pill" />
              <Pip label="Start" active={down(9)} shape="pill" />
            </div>
          </div>

          <Diamond top={down(3)} left={down(2)} right={down(1)} bottom={down(0)} />
        </div>

        {/* Analog sticks, drawn as wells with a live dot. */}
        <div className="mt-7 flex justify-center gap-10 border-t border-primary/10 pt-7">
          <Stick label="L3" x={axis(0)} y={axis(1)} active={down(10)} />
          <Stick label="R3" x={axis(2)} y={axis(3)} active={down(11)} />
        </div>

        {extras.length > 0 && (
          <div className="mt-6 flex flex-wrap items-center gap-2 border-t border-primary/10 pt-5">
            <span className="text-[10px] uppercase tracking-wider text-ink-dim">
              Other
            </span>
            {extras.map((i) => (
              <span
                key={i}
                className="rounded-md bg-primary px-2 py-0.5 text-[11px] font-semibold text-black"
              >
                {BUTTON_NAMES[i] ?? `#${i}`}
              </span>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

function Pip({
  label,
  active,
  shape = "round",
}: {
  label: string;
  active: boolean;
  shape?: "round" | "round-lg" | "pill";
}) {
  const base =
    shape === "round"
      ? "h-11 w-11 rounded-full text-sm"
      : shape === "round-lg"
        ? "h-12 w-12 rounded-full text-sm"
        : "h-7 rounded-full px-3 text-[11px]";
  return (
    <span
      className={`flex items-center justify-center font-display font-bold tracking-wide transition-all duration-75 ${base} ${
        active
          ? "scale-105 bg-primary text-black shadow-glow"
          : "bg-surface-2 text-ink-dim ring-1 ring-primary/10"
      }`}
    >
      {label}
    </span>
  );
}

// Vertical trigger fill bar driven by the analog button value (0–1).
function Trigger({
  label,
  value,
  active,
}: {
  label: string;
  value: number;
  active: boolean;
}) {
  return (
    <div className="flex items-center gap-2">
      <span
        className={`w-6 font-display text-[11px] font-bold tracking-wide transition-colors ${
          active || value > 0.02 ? "text-primary" : "text-ink-dim"
        }`}
      >
        {label}
      </span>
      <div className="relative h-2.5 flex-1 overflow-hidden rounded-full bg-surface-2 ring-1 ring-primary/10">
        <div
          className="absolute inset-y-0 left-0 rounded-full bg-primary transition-[width] duration-75"
          style={{ width: `${Math.min(1, Math.max(0, value)) * 100}%` }}
        />
      </div>
    </div>
  );
}

// D-pad cross, laid out on a 3×3 grid.
function Cross({
  up,
  down,
  left,
  right,
}: {
  up: boolean;
  down: boolean;
  left: boolean;
  right: boolean;
}) {
  return (
    <div className="mx-auto grid grid-cols-3 grid-rows-3 gap-1.5">
      <span />
      <DirCap active={up} dir="up" />
      <span />
      <DirCap active={left} dir="left" />
      <span className="h-9 w-9 rounded-md bg-surface-2 ring-1 ring-primary/10" />
      <DirCap active={right} dir="right" />
      <span />
      <DirCap active={down} dir="down" />
      <span />
    </div>
  );
}

function DirCap({
  active,
  dir,
}: {
  active: boolean;
  dir: "up" | "down" | "left" | "right";
}) {
  const glyph = { up: "▲", down: "▼", left: "◀", right: "▶" }[dir];
  return (
    <span
      className={`flex h-9 w-9 items-center justify-center rounded-md text-[10px] transition-all duration-75 ${
        active
          ? "scale-105 bg-primary text-black shadow-glow"
          : "bg-surface-2 text-ink-dim ring-1 ring-primary/10"
      }`}
    >
      {glyph}
    </span>
  );
}

// Face-button diamond (Xbox layout: Y top, A bottom, X left, B right).
function Diamond({
  top,
  left,
  right,
  bottom,
}: {
  top: boolean;
  left: boolean;
  right: boolean;
  bottom: boolean;
}) {
  return (
    <div className="mx-auto grid grid-cols-3 grid-rows-3 place-items-center gap-1">
      <span />
      <Pip label="Y" active={top} shape="round-lg" />
      <span />
      <Pip label="X" active={left} shape="round-lg" />
      <span />
      <Pip label="B" active={right} shape="round-lg" />
      <span />
      <Pip label="A" active={bottom} shape="round-lg" />
      <span />
    </div>
  );
}

// Analog stick well: a dot offset from center by the (x, y) axis pair.
function Stick({
  label,
  x,
  y,
  active,
}: {
  label: string;
  x: number;
  y: number;
  active: boolean;
}) {
  const r = 30;
  const live = active || Math.abs(x) > 0.12 || Math.abs(y) > 0.12;
  return (
    <div className="flex flex-col items-center gap-2.5">
      <div
        className={`relative h-24 w-24 rounded-full bg-surface-2/70 transition-shadow duration-75 ${
          active
            ? "ring-2 ring-primary shadow-glow"
            : "ring-1 ring-primary/15"
        }`}
      >
        <span className="absolute left-1/2 top-1/2 h-px w-full -translate-x-1/2 -translate-y-1/2 bg-primary/10" />
        <span className="absolute left-1/2 top-1/2 h-full w-px -translate-x-1/2 -translate-y-1/2 bg-primary/10" />
        <span
          className={`absolute left-1/2 top-1/2 h-7 w-7 rounded-full transition-[transform,background-color] duration-75 ${
            live ? "bg-primary shadow-glow" : "bg-ink-dim/40"
          }`}
          style={{
            transform: `translate(calc(-50% + ${x * r}px), calc(-50% + ${y * r}px))`,
          }}
        />
      </div>
      <span
        className={`font-display text-[11px] font-bold tracking-wide transition-colors ${
          active ? "text-primary" : "text-ink-dim"
        }`}
      >
        {label}
      </span>
    </div>
  );
}

function ProfilesPanel() {
  const qc = useQueryClient();
  const setToast = useStore((s) => s.setToast);
  const [name, setName] = useState("");
  const [editing, setEditing] = useState<ControllerProfile | null>(null);

  const config = useQuery({
    queryKey: ["controller-config"],
    queryFn: () => api.controllerConfig(),
  });

  const invalidate = () =>
    qc.invalidateQueries({ queryKey: ["controller-config"] });

  const create = useMutation({
    mutationFn: (n: string) =>
      api.saveControllerProfile({ id: "", name: n, bindings: {} }),
    onSuccess: (p) => {
      setName("");
      setToast(`Profile "${p.name}" created.`);
      invalidate();
    },
  });

  const save = useMutation({
    mutationFn: (p: ControllerProfile) => api.saveControllerProfile(p),
    onSuccess: () => {
      setEditing(null);
      setToast("Bindings saved.");
      invalidate();
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => api.deleteControllerProfile(id),
    onSuccess: invalidate,
  });

  const activate = useMutation({
    mutationFn: (id: string | null) => api.setActiveControllerProfile(id),
    onSuccess: invalidate,
  });

  const profiles = config.data?.profiles ?? [];
  const active = config.data?.active_profile ?? null;

  return (
    <section className="mb-6">
      <h2 className="font-display text-lg font-bold">Mapping Profiles</h2>
      <p className="mb-3 text-xs text-ink-dim">
        Named binding sets you can switch between. Bindings are stored by Arcadia
        and passed through to emulators that accept them.
      </p>

      <div className="glass mb-4 flex max-w-sm gap-2 rounded-2xl p-3">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && name.trim()) create.mutate(name.trim());
          }}
          placeholder="New profile name…"
          className="flex-1 bg-transparent px-2 text-sm outline-none placeholder:text-ink-dim"
        />
        <Focusable
          onActivate={() => name.trim() && create.mutate(name.trim())}
          ariaLabel="Create controller profile"
          className="rounded-lg bg-primary px-4 py-1.5 text-sm font-semibold text-black"
        >
          {create.isPending ? "Creating…" : "Create"}
        </Focusable>
      </div>

      {profiles.length === 0 ? (
        <p className="text-sm text-ink-dim">No profiles yet.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {profiles.map((p) => (
            <li key={p.id} className="glass rounded-2xl p-3">
              <div className="flex items-center gap-3">
                <span className="flex-1 text-sm font-semibold">{p.name}</span>
                {active === p.id ? (
                  <span className="rounded-md bg-primary/15 px-2 py-0.5 text-[10px] font-semibold text-primary">
                    active
                  </span>
                ) : (
                  <Focusable
                    onActivate={() => activate.mutate(p.id)}
                    ariaLabel={`Activate ${p.name}`}
                    className="rounded-lg px-2 py-1 text-xs text-primary"
                  >
                    Set active
                  </Focusable>
                )}
                <Focusable
                  onActivate={() => setEditing(editing?.id === p.id ? null : p)}
                  ariaLabel={`Edit ${p.name}`}
                  className="rounded-lg px-2 py-1 text-xs"
                >
                  {editing?.id === p.id ? "Done" : "Edit"}
                </Focusable>
                <Focusable
                  onActivate={() => remove.mutate(p.id)}
                  ariaLabel={`Delete ${p.name}`}
                  className="rounded-lg px-2 py-1 text-xs text-secondary"
                >
                  Delete
                </Focusable>
              </div>

              {editing?.id === p.id && (
                <BindingEditor
                  profile={editing}
                  onChange={setEditing}
                  onSave={() => save.mutate(editing)}
                  saving={save.isPending}
                />
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

function BindingEditor({
  profile,
  onChange,
  onSave,
  saving,
}: {
  profile: ControllerProfile;
  onChange: (p: ControllerProfile) => void;
  onSave: () => void;
  saving: boolean;
}) {
  return (
    <div className="mt-3 border-t border-primary/10 pt-3">
      <div className="grid grid-cols-2 gap-2">
        {ACTIONS.map((action) => (
          <label key={action} className="flex flex-col gap-1 text-xs">
            <span className="text-ink-dim">{action.replace(/_/g, " ")}</span>
            <input
              value={profile.bindings[action] ?? ""}
              onChange={(e) =>
                onChange({
                  ...profile,
                  bindings: { ...profile.bindings, [action]: e.target.value },
                })
              }
              placeholder="e.g. A, Start, LB…"
              className="glass rounded-lg bg-surface-2 px-3 py-2 text-sm outline-none focus:border-primary"
            />
          </label>
        ))}
      </div>
      <Focusable
        onActivate={onSave}
        ariaLabel="Save bindings"
        className="mt-3 inline-block rounded-xl bg-primary px-4 py-2 text-sm font-semibold text-black"
      >
        {saving ? "Saving…" : "Save bindings"}
      </Focusable>
    </div>
  );
}
