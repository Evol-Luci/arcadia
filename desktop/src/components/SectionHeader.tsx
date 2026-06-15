import type { ReactNode } from "react";

interface Props {
  title: string;
  subtitle?: string;
  /** Optional content pinned to the right of the title. */
  aside?: ReactNode;
}

// Shared sub-section heading: a small accent tick + display title, with room
// for a subtitle or right-aligned aside. Keeps hierarchy consistent app-wide.
export function SectionHeader({ title, subtitle, aside }: Props) {
  return (
    <div className="mb-3 flex items-end justify-between gap-3">
      <div className="min-w-0">
        <h2 className="flex items-center gap-2 font-display text-lg font-bold tracking-wide">
          <span className="h-4 w-1 shrink-0 rounded-full bg-primary/70" />
          {title}
        </h2>
        {subtitle && (
          <p className="mt-1 pl-3 text-xs leading-relaxed text-ink-dim">
            {subtitle}
          </p>
        )}
      </div>
      {aside && <div className="shrink-0">{aside}</div>}
    </div>
  );
}
