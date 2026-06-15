import { forwardRef, type ReactNode } from "react";

interface Props {
  children: ReactNode;
  onActivate?: () => void;
  className?: string;
  /** Render as this many grid columns worth of width (passes through styling). */
  as?: "div" | "button";
  ariaLabel?: string;
}

// Any element that should be reachable by keyboard / controller renders through
// here. It carries `data-focusable` (picked up by the spatial nav manager) and
// activates on click — which is also what Enter / the A button trigger.
export const Focusable = forwardRef<HTMLDivElement, Props>(function Focusable(
  { children, onActivate, className = "", ariaLabel },
  ref,
) {
  return (
    <div
      ref={ref}
      data-focusable="true"
      data-focused="false"
      role="button"
      aria-label={ariaLabel}
      tabIndex={-1}
      onClick={onActivate}
      onMouseEnter={(e) => {
        // Pointer hover moves the focus ring so mouse + controller agree.
        for (const el of document.querySelectorAll('[data-focusable][data-focused="true"]'))
          el.setAttribute("data-focused", "false");
        e.currentTarget.setAttribute("data-focused", "true");
      }}
      className={`focusable cursor-pointer ${className}`}
    >
      {children}
    </div>
  );
});
