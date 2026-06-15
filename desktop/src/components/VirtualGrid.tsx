import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

interface Props<T> {
  items: T[];
  getKey: (item: T) => string;
  renderItem: (item: T) => ReactNode;
  /** Fixed card width in px (cards are uniform). */
  cardWidth: number;
  /** Estimated card height in px; auto-corrected after first paint. */
  cardHeight: number;
  /** Gap between cards in px (both axes). */
  gap?: number;
  /** Extra rows rendered above/below the viewport. Keep >= 2 so the spatial
   *  nav always finds the adjacent row when stepping to the grid edge. */
  overscanRows?: number;
  className?: string;
}

function findScrollParent(el: HTMLElement | null): HTMLElement | null {
  let p = el?.parentElement ?? null;
  while (p) {
    const oy = getComputedStyle(p).overflowY;
    if (oy === "auto" || oy === "scroll") return p;
    p = p.parentElement;
  }
  return null;
}

interface Metrics {
  width: number;
  viewportH: number;
  scrollTop: number;
  gridTop: number;
  cardH: number;
}

// Windowed grid: only the cards in (or near) the viewport are mounted. Cards are
// absolutely positioned at their exact row/column so the geometry-based spatial
// nav (which reads getBoundingClientRect on [data-focusable]) keeps working
// unchanged — every single-step move lands on a card that overscan guarantees is
// rendered. This keeps the DOM at ~a few hundred nodes regardless of library
// size, which is what makes large libraries render and navigate smoothly.
export function VirtualGrid<T>({
  items,
  getKey,
  renderItem,
  cardWidth,
  cardHeight,
  gap = 16,
  overscanRows = 3,
  className,
}: Props<T>) {
  const rootRef = useRef<HTMLDivElement>(null);
  const scrollParentRef = useRef<HTMLElement | null>(null);
  const [m, setM] = useState<Metrics>({
    width: 0,
    viewportH: 0,
    scrollTop: 0,
    gridTop: 0,
    cardH: cardHeight,
  });

  // Re-measure container width, viewport height, scroll offset, and the grid's
  // position within the scroll content. Cheap (a few rect reads); coalesced via
  // rAF on scroll. Only commits state when something actually changed so we
  // don't re-render every scroll frame for free.
  const measure = () => {
    const parent = scrollParentRef.current;
    const root = rootRef.current;
    if (!parent || !root) return;
    const pr = parent.getBoundingClientRect();
    const rr = root.getBoundingClientRect();
    const cell = root.querySelector<HTMLElement>("[data-vg-cell]");
    const cardH = cell?.offsetHeight || cardHeight;
    const next: Metrics = {
      width: root.clientWidth,
      viewportH: parent.clientHeight,
      scrollTop: parent.scrollTop,
      gridTop: rr.top - pr.top + parent.scrollTop,
      cardH,
    };
    setM((prev) =>
      prev.width === next.width &&
      prev.viewportH === next.viewportH &&
      prev.scrollTop === next.scrollTop &&
      prev.gridTop === next.gridTop &&
      prev.cardH === next.cardH
        ? prev
        : next,
    );
  };

  useLayoutEffect(() => {
    scrollParentRef.current = findScrollParent(rootRef.current);
    measure();
    const parent = scrollParentRef.current;
    if (!parent) return;

    let raf = 0;
    const onScroll = () => {
      if (raf) return;
      raf = requestAnimationFrame(() => {
        raf = 0;
        measure();
      });
    };
    parent.addEventListener("scroll", onScroll, { passive: true });

    const ro = new ResizeObserver(() => measure());
    ro.observe(parent);
    if (rootRef.current) ro.observe(rootRef.current);

    return () => {
      parent.removeEventListener("scroll", onScroll);
      ro.disconnect();
      if (raf) cancelAnimationFrame(raf);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // Content above the grid (filter chips that wrap, etc.) can shift gridTop when
  // the item set changes; re-measure when the list length changes.
  useEffect(() => {
    measure();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [items.length]);

  const cols = Math.max(1, Math.floor((m.width + gap) / (cardWidth + gap)));
  const rowH = m.cardH + gap;
  const rowCount = Math.ceil(items.length / cols);
  const totalH = Math.max(0, rowCount * rowH - gap);

  const firstRow = Math.max(
    0,
    Math.floor((m.scrollTop - m.gridTop) / rowH) - overscanRows,
  );
  const lastRow = Math.min(
    rowCount - 1,
    Math.ceil((m.scrollTop - m.gridTop + m.viewportH) / rowH) + overscanRows,
  );

  const start = firstRow * cols;
  const end = Math.min(items.length, (lastRow + 1) * cols);

  const cells: ReactNode[] = [];
  for (let i = start; i < end; i++) {
    const item = items[i];
    const row = Math.floor(i / cols);
    const col = i % cols;
    cells.push(
      <div
        key={getKey(item)}
        data-vg-cell={i === start ? "true" : undefined}
        style={{
          position: "absolute",
          top: row * rowH,
          left: col * (cardWidth + gap),
          width: cardWidth,
        }}
      >
        {renderItem(item)}
      </div>,
    );
  }

  return (
    <div
      ref={rootRef}
      className={className}
      style={{ position: "relative", height: totalH }}
    >
      {cells}
    </div>
  );
}
