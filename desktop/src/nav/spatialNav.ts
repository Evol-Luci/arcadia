// Spatial navigation for Mouse / Keyboard / Controller / Touch.
//
// "Controller from day one" — every screen is operable without a pointer.
// Focusable elements opt in by rendering `data-focusable` (the `focusable`
// helper in components does this). The manager picks the nearest element in
// the pressed direction by geometry, so it works with any dynamic layout.

type Dir = "up" | "down" | "left" | "right";

function focusables(): HTMLElement[] {
  return Array.from(
    document.querySelectorAll<HTMLElement>("[data-focusable]"),
  ).filter((el) => el.offsetParent !== null);
}

function current(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[data-focusable][data-focused="true"]');
}

function setFocus(el: HTMLElement | null) {
  for (const e of focusables()) e.setAttribute("data-focused", "false");
  if (el) {
    el.setAttribute("data-focused", "true");
    el.scrollIntoView({ block: "nearest", inline: "nearest", behavior: "smooth" });
  }
}

function center(el: HTMLElement) {
  const r = el.getBoundingClientRect();
  return { x: r.left + r.width / 2, y: r.top + r.height / 2 };
}

function move(dir: Dir) {
  const els = focusables();
  if (els.length === 0) return;
  const cur = current();
  if (!cur) {
    setFocus(els[0]);
    return;
  }
  const c = center(cur);
  let best: HTMLElement | null = null;
  let bestScore = Infinity;

  for (const el of els) {
    if (el === cur) continue;
    const p = center(el);
    const dx = p.x - c.x;
    const dy = p.y - c.y;

    const inDir =
      (dir === "up" && dy < -1) ||
      (dir === "down" && dy > 1) ||
      (dir === "left" && dx < -1) ||
      (dir === "right" && dx > 1);
    if (!inDir) continue;

    // Distance along the travel axis dominates; cross-axis drift is penalised.
    const primary = dir === "up" || dir === "down" ? Math.abs(dy) : Math.abs(dx);
    const cross = dir === "up" || dir === "down" ? Math.abs(dx) : Math.abs(dy);
    const score = primary + cross * 2;
    if (score < bestScore) {
      bestScore = score;
      best = el;
    }
  }
  if (best) setFocus(best);
}

function activate() {
  current()?.click();
}

function back() {
  window.dispatchEvent(new CustomEvent("arcadia:back"));
}

function tab(delta: number) {
  window.dispatchEvent(new CustomEvent("arcadia:tab", { detail: delta }));
}

// ---- Gamepad polling ------------------------------------------------------

const REPEAT_MS = 180;
let last: Record<string, number> = {};

// When the controller live tester is capturing, the gamepad must not also drive
// app navigation. Keyboard/mouse stay active so the tester can still be toggled
// off by those inputs.
let captureGamepad = false;
export function setGamepadCapture(on: boolean) {
  captureGamepad = on;
}

// While a game is running, the emulator owns the controller. Our evdev bridge
// reads the pad globally regardless of window focus, so without this gate the
// background dreamshell would keep moving its focus from in-game button
// presses. We track sessions by id (rather than a bare boolean) so overlapping
// launches don't clear the gate early — nav stays suspended until the last
// session ends.
const activeSessions = new Set<string>();
function navSuspended(): boolean {
  return activeSessions.size > 0;
}

/** Suspend gamepad navigation for a launched game session. */
export function noteGameLaunched(sessionId: string) {
  activeSessions.add(sessionId);
}

/** Resume gamepad navigation when a game session ends. */
export function noteGameEnded(sessionId: string) {
  activeSessions.delete(sessionId);
}

function gate(key: string): boolean {
  const now = performance.now();
  if (now - (last[key] ?? 0) < REPEAT_MS) return false;
  last[key] = now;
  return true;
}

function pollGamepads() {
  if (captureGamepad || navSuspended()) return;
  const pads = navigator.getGamepads?.() ?? [];
  for (const pad of pads) {
    if (!pad) continue;
    const b = pad.buttons;
    const ax = pad.axes;

    const up = b[12]?.pressed || (ax[1] ?? 0) < -0.5;
    const down = b[13]?.pressed || (ax[1] ?? 0) > 0.5;
    const left = b[14]?.pressed || (ax[0] ?? 0) < -0.5;
    const right = b[15]?.pressed || (ax[0] ?? 0) > 0.5;

    if (up && gate("up")) move("up");
    if (down && gate("down")) move("down");
    if (left && gate("left")) move("left");
    if (right && gate("right")) move("right");

    if (b[0]?.pressed && gate("a")) activate(); // A / cross
    if (b[1]?.pressed && gate("b")) back(); // B / circle
    if (b[4]?.pressed && gate("lb")) tab(-1);
    if (b[5]?.pressed && gate("rb")) tab(1);
  }
}

let raf = 0;
function loop() {
  pollGamepads();
  raf = requestAnimationFrame(loop);
}

function onKeyDown(e: KeyboardEvent) {
  const target = e.target as HTMLElement | null;
  // Don't hijack typing in inputs.
  if (target && (target.tagName === "INPUT" || target.tagName === "TEXTAREA")) {
    if (e.key === "Escape") (target as HTMLInputElement).blur();
    return;
  }
  switch (e.key) {
    case "ArrowUp":
      e.preventDefault();
      move("up");
      break;
    case "ArrowDown":
      e.preventDefault();
      move("down");
      break;
    case "ArrowLeft":
      move("left");
      break;
    case "ArrowRight":
      move("right");
      break;
    case "Enter":
    case " ":
      e.preventDefault();
      activate();
      break;
    case "Backspace":
    case "Escape":
      back();
      break;
    case "Tab":
      e.preventDefault();
      tab(e.shiftKey ? -1 : 1);
      break;
  }
}

/** Start global navigation. Returns a teardown function. */
export function startSpatialNav(): () => void {
  window.addEventListener("keydown", onKeyDown);
  raf = requestAnimationFrame(loop);
  // Focus the first element once the first render settles.
  setTimeout(() => {
    if (!current()) setFocus(focusables()[0] ?? null);
  }, 80);
  return () => {
    window.removeEventListener("keydown", onKeyDown);
    cancelAnimationFrame(raf);
    last = {};
  };
}

/** Re-seed focus to the first focusable (call on view change). */
export function resetFocus() {
  last = {};
  setTimeout(() => setFocus(focusables()[0] ?? null), 40);
}
