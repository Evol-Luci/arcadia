import type { ReactNode } from "react";

interface Props {
  title: string;
  subtitle?: string;
  /** Optional content pinned to the right of the title (counts, actions). */
  aside?: ReactNode;
}

// Shared page title treatment so every view opens the same way: a glowing
// gradient accent bar, the Orbitron display title, and an optional subtitle.
export function PageHeader({ title, subtitle, aside }: Props) {
  return (
    <header className="mb-7 flex items-end justify-between gap-4">
      <div className="min-w-0">
        <div className="flex items-center gap-3">
          <span className="h-7 w-1.5 shrink-0 rounded-full bg-gradient-to-b from-primary to-secondary shadow-glow" />
          <h1 className="truncate font-display text-3xl font-black tracking-wide text-glow">
            {title}
          </h1>
        </div>
        {subtitle && (
          <p className="mt-1.5 pl-[18px] text-sm text-ink-dim">{subtitle}</p>
        )}
      </div>
      {aside && <div className="shrink-0">{aside}</div>}
    </header>
  );
}
