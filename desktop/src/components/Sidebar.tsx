import { Focusable } from "./Focusable";
import { useStore, type View } from "../store/useStore";

const ITEMS: { view: View; label: string; icon: string }[] = [
  { view: "home", label: "Home", icon: "◈" },
  { view: "library", label: "Library", icon: "▦" },
  { view: "recent", label: "Recent", icon: "↺" },
  { view: "favorites", label: "Favorites", icon: "☆" },
  { view: "collections", label: "Collections", icon: "❏" },
  { view: "screenshots", label: "Screenshots", icon: "▢" },
  { view: "achievements", label: "Achievements", icon: "★" },
  { view: "controller", label: "Controller", icon: "✛" },
  { view: "plugins", label: "Plugins", icon: "⧉" },
  { view: "stats", label: "Stats", icon: "▲" },
  { view: "settings", label: "Settings", icon: "⚙" },
];

export function Sidebar() {
  const view = useStore((s) => s.view);
  const setView = useStore((s) => s.setView);
  const progress = useStore((s) => s.progress);
  const active = view === "game" ? "library" : view;

  return (
    <nav className="flex h-full w-[210px] shrink-0 flex-col gap-1 border-r border-primary/10 p-3">
      <div className="mb-4 px-2">
        <div className="font-display text-2xl font-black tracking-widest text-primary text-glow">
          ARCADIA
        </div>
        <div className="text-[10px] uppercase tracking-[0.3em] text-ink-dim">
          Dreamshell
        </div>
      </div>

      {ITEMS.map((item) => (
        <Focusable
          key={item.view}
          ariaLabel={item.label}
          onActivate={() => setView(item.view)}
          className={`rounded-xl px-3 py-2.5 ${
            active === item.view
              ? "bg-primary/15 text-primary"
              : "text-ink-dim hover:text-ink"
          }`}
        >
          <div className="flex items-center gap-3">
            <span className="w-5 text-center text-lg">{item.icon}</span>
            <span className="text-sm font-semibold">{item.label}</span>
          </div>
        </Focusable>
      ))}

      {progress && <ProgressCard progress={progress} />}

      <div className="mt-auto px-2 text-[10px] leading-relaxed text-ink-dim/70">
        Powered by DreamVault
        <br />
        v0.1 · Console Mode
      </div>
    </nav>
  );
}

function ProgressCard({
  progress,
}: {
  progress: NonNullable<ReturnType<typeof useStore.getState>["progress"]>;
}) {
  const { label, done, total } = progress;
  const indeterminate = total === 0;
  const pct = indeterminate ? 0 : Math.min(100, Math.round((done / total) * 100));

  return (
    <div className="mt-3 rounded-xl border border-primary/15 bg-primary/5 px-3 py-2.5">
      <div className="mb-1.5 flex items-center justify-between gap-2">
        <span className="truncate text-xs font-semibold text-primary">{label}</span>
        {!indeterminate && (
          <span className="shrink-0 text-[10px] tabular-nums text-ink-dim">
            {done}/{total}
          </span>
        )}
      </div>
      <div className="h-1.5 overflow-hidden rounded-full bg-surface-2">
        {indeterminate ? (
          <div className="h-full w-1/3 animate-progress-indeterminate rounded-full bg-primary" />
        ) : (
          <div
            className="h-full rounded-full bg-primary transition-[width] duration-300"
            style={{ width: `${pct}%` }}
          />
        )}
      </div>
    </div>
  );
}
