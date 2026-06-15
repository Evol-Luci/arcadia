import { Focusable } from "./Focusable";

interface Props {
  title: string;
  body: string;
  actionLabel?: string;
  onAction?: () => void;
}

export function EmptyState({ title, body, actionLabel, onAction }: Props) {
  return (
    <div className="glass mx-auto mt-16 max-w-md rounded-3xl p-8 text-center">
      <div className="mb-3 font-display text-2xl font-black text-primary text-glow">
        {title}
      </div>
      <p className="mb-6 text-sm leading-relaxed text-ink-dim">{body}</p>
      {actionLabel && onAction && (
        <Focusable
          onActivate={onAction}
          ariaLabel={actionLabel}
          className="inline-block rounded-xl bg-primary px-5 py-2.5 font-semibold text-black"
        >
          {actionLabel}
        </Focusable>
      )}
    </div>
  );
}
