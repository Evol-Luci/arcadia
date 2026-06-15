import { Fragment } from "react";
import { Focusable } from "./Focusable";
import { EmptyState } from "./EmptyState";
import {
  groupPlatforms,
  platformName,
  platformShort,
} from "../lib/platforms";
import type { PlatformUsage } from "../api/types";

interface ConsoleNavProps {
  usage: PlatformUsage[];
  total: number;
  active: string | null;
  onSelect: (platform: string | null) => void;
}

// Compact, manufacturer-grouped console switcher shown above a loaded library.
// Designed as a clean tab strip rather than a wall of boxes: chips are text-only
// until focused or active, manufacturers are marked by a slim accent tick, and
// the active console reads as a single solid primary pill.
export function ConsoleRail({ usage, total, active, onSelect }: ConsoleNavProps) {
  const groups = groupPlatforms(usage);
  // The glass panel stays solid; only the inner scroll track is masked so its
  // content dissolves at both edges — a quiet hint that the rail scrolls past
  // what's visible. py-3 leaves room for the focus glow not to clip.
  return (
    <div className="glass mb-5 rounded-2xl">
      <div className="flex flex-nowrap items-center gap-x-1 overflow-x-auto px-4 py-3 [-webkit-overflow-scrolling:touch] [mask-image:linear-gradient(to_right,transparent,#000_28px,#000_calc(100%-28px),transparent)] [scrollbar-width:none] [-webkit-mask-image:linear-gradient(to_right,transparent,#000_28px,#000_calc(100%-28px),transparent)] [&::-webkit-scrollbar]:hidden">
        <Chip
          label="All"
          count={total}
          active={active === null}
          onSelect={() => onSelect(null)}
        />
        {groups.map((g) => (
          <Fragment key={g.manufacturer}>
            <span className="mx-1.5 flex shrink-0 select-none items-center gap-2">
              <span aria-hidden className="h-4 w-px bg-primary/25" />
              <span className="text-[10px] font-bold uppercase tracking-[0.2em] text-primary/50">
                {g.manufacturer}
              </span>
            </span>
            {g.items.map((u) => (
              <Chip
                key={u.platform}
                label={platformShort(u.platform)}
                count={u.game_count}
                active={active === u.platform}
                onSelect={() => onSelect(u.platform)}
              />
            ))}
          </Fragment>
        ))}
      </div>
    </div>
  );
}

function Chip({
  label,
  count,
  active,
  onSelect,
}: {
  label: string;
  count: number;
  active: boolean;
  onSelect: () => void;
}) {
  return (
    <Focusable
      ariaLabel={`${label}, ${count} games`}
      onActivate={onSelect}
      className={`group flex shrink-0 items-center gap-1.5 rounded-full py-1.5 pl-3 pr-1.5 text-xs font-semibold transition-colors ${
        active ? "bg-primary shadow-[0_0_20px_rgb(var(--arc-primary)/0.5)]" : ""
      }`}
    >
      <span
        className={
          active
            ? "text-black"
            : "text-ink group-data-[focused=true]:text-primary"
        }
      >
        {label}
      </span>
      <span
        className={`rounded-full px-1.5 py-0.5 text-[10px] font-bold tabular-nums ${
          active ? "bg-black/20 text-black/70" : "bg-ink/10 text-ink-dim"
        }`}
      >
        {count.toLocaleString()}
      </span>
    </Focusable>
  );
}

interface PickerProps extends Omit<ConsoleNavProps, "active"> {
  onOpenSettings: () => void;
  loading: boolean;
}

// Landing screen for the Library: pick a console (or All) before any games are
// loaded, so a large library is never fetched just to browse a small one.
export function ConsolePicker({
  usage,
  total,
  onSelect,
  onOpenSettings,
  loading,
}: PickerProps) {
  if (!loading && total === 0) {
    return (
      <EmptyState
        title="No games indexed"
        body="Add a ROM folder and scan your library from Settings."
        actionLabel="Open Settings"
        onAction={onOpenSettings}
      />
    );
  }

  const groups = groupPlatforms(usage);
  const systemCount = usage.length;
  // Running index drives a staggered entrance cascade across all rows.
  let step = 0;
  const delay = () => ({ animationDelay: `${step++ * 45}ms` });

  return (
    <div>
      <header
        className="mb-8 animate-fade-up"
        style={delay()}
      >
        <div className="text-[11px] font-bold uppercase tracking-[0.3em] text-secondary">
          Select a system
        </div>
        <h1 className="mt-1 font-display text-4xl font-black tracking-wide text-glow">
          Library
        </h1>
        <p className="mt-1.5 text-sm text-ink-dim">
          {total.toLocaleString()} games across {systemCount}{" "}
          {systemCount === 1 ? "console" : "consoles"}.
        </p>
      </header>

      <div className="mb-9 animate-fade-up" style={delay()}>
        <Focusable
          ariaLabel={`All games, ${total} games`}
          onActivate={() => onSelect(null)}
          className="group glass relative flex items-center justify-between overflow-hidden rounded-3xl border-primary/40 px-7 py-6 shadow-[0_0_34px_rgb(var(--arc-primary)/0.15)]"
        >
          <span
            aria-hidden
            className="pointer-events-none absolute inset-0 bg-[radial-gradient(130%_120%_at_0%_0%,rgb(var(--arc-primary)/0.18),transparent_55%)]"
          />
          <span
            aria-hidden
            className="pointer-events-none absolute -right-6 -top-8 font-display text-[8rem] font-black leading-none text-primary/[0.06] transition-colors duration-300 group-data-[focused=true]:text-primary/[0.12]"
          >
            ∞
          </span>
          <div className="relative">
            <div className="text-[11px] font-bold uppercase tracking-[0.3em] text-primary/80">
              Everything
            </div>
            <div className="mt-0.5 font-display text-3xl font-black text-glow">
              All Games
            </div>
            <div className="mt-1 text-xs text-ink-dim">
              Browse your full collection
            </div>
          </div>
          <div className="relative text-right">
            <div className="font-display text-5xl font-black text-primary drop-shadow-[0_0_18px_rgb(var(--arc-primary)/0.5)]">
              {total.toLocaleString()}
            </div>
            <div className="text-[10px] uppercase tracking-[0.25em] text-ink-dim">
              titles
            </div>
          </div>
        </Focusable>
      </div>

      {groups.map((g) => {
        const games = g.items.reduce((n, u) => n + u.game_count, 0);
        return (
          <section
            key={g.manufacturer}
            className="mb-7 animate-fade-up"
            style={delay()}
          >
            <div className="mb-3.5 flex items-center gap-3">
              <h2 className="font-display text-sm font-black uppercase tracking-[0.25em] text-ink">
                {g.manufacturer}
              </h2>
              <span className="text-[10px] font-semibold uppercase tracking-wider text-ink-dim">
                {g.items.length} {g.items.length === 1 ? "system" : "systems"} ·{" "}
                {games.toLocaleString()} games
              </span>
              <span
                aria-hidden
                className="h-px flex-1 bg-gradient-to-r from-primary/30 via-primary/10 to-transparent"
              />
            </div>
            <div className="grid grid-cols-[repeat(auto-fill,minmax(190px,1fr))] gap-3">
              {g.items.map((u) => (
                <ConsoleTile
                  key={u.platform}
                  name={platformName(u.platform)}
                  short={platformShort(u.platform)}
                  count={u.game_count}
                  onSelect={() => onSelect(u.platform)}
                />
              ))}
            </div>
          </section>
        );
      })}
    </div>
  );
}

function ConsoleTile({
  name,
  short,
  count,
  onSelect,
}: {
  name: string;
  short: string;
  count: number;
  onSelect: () => void;
}) {
  return (
    <Focusable
      ariaLabel={`${name}, ${count} games`}
      onActivate={onSelect}
      className="group glass relative overflow-hidden rounded-2xl px-5 py-4"
    >
      <span
        aria-hidden
        className="pointer-events-none absolute -right-3 -top-4 font-display text-6xl font-black leading-none text-primary/[0.07] transition-colors duration-300 group-data-[focused=true]:text-primary/[0.16]"
      >
        {short}
      </span>
      <span
        aria-hidden
        className="pointer-events-none absolute left-0 top-0 h-full w-1 bg-gradient-to-b from-primary/70 to-secondary/40"
      />
      <div className="relative">
        <div className="truncate pr-8 text-sm font-semibold">{name}</div>
        <div className="mt-2 flex items-baseline gap-1.5">
          <span className="font-display text-2xl font-black text-primary">
            {count.toLocaleString()}
          </span>
          <span className="text-[10px] uppercase tracking-wider text-ink-dim">
            games
          </span>
        </div>
      </div>
    </Focusable>
  );
}
