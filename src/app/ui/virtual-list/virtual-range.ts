export interface VisibleRange {
  /** Index of the first rendered item. */
  start: number;
  /** Index after the last rendered item. */
  end: number;
  /** Height of the empty space above the rendered items. */
  offsetTop: number;
  /** Height of the empty space below the rendered items. */
  offsetBottom: number;
}

/**
 * Which slice of a long list of equally tall rows to render for the current scroll position,
 * with `overscan` extra rows above and below to avoid flicker while scrolling.
 */
export function visibleRange(
  scrollTop: number,
  viewportHeight: number,
  rowHeight: number,
  total: number,
  overscan = 8,
): VisibleRange {
  if (total <= 0 || rowHeight <= 0) return { start: 0, end: 0, offsetTop: 0, offsetBottom: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const visibleCount = Math.ceil(Math.max(0, viewportHeight) / rowHeight) + 1;
  const start = Math.min(total, Math.max(0, first - overscan));
  const end = Math.min(total, first + visibleCount + overscan);
  return {
    start,
    end,
    offsetTop: start * rowHeight,
    offsetBottom: (total - end) * rowHeight,
  };
}

/** True when the list is scrolled to (or within a row of) its end. */
export function isAtBottom(
  scrollTop: number,
  viewportHeight: number,
  contentHeight: number,
  slack = 24,
): boolean {
  return scrollTop + viewportHeight >= contentHeight - slack;
}
