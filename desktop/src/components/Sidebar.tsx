import { Focusable } from "./Focusable";
import { useStore, type View } from "../store/useStore";

const ITEMS: { view: View; label: string; icon: string }[] = [
  { view: "home", label: "Home", icon: "◈" },
  { view: "library", label: "Library", icon: "▦" },
  { view: "recent", label: "Recent", icon: "↺" },
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

      <div className="mt-auto px-2 text-[10px] leading-relaxed text-ink-dim/70">
        Powered by DreamVault
        <br />
        v0.1 · Console Mode
      </div>
    </nav>
  );
}
