import type { Span } from "../../core/bindings";

export interface WaterfallRow {
  span: Span;
  depth: number;
  /** Where the bar starts, 0–100 (percent of the trace). */
  offsetPct: number;
  /** Bar width, 0–100; never zero so instant spans stay visible. */
  widthPct: number;
  /** `null` while the span is still open. */
  durationMs: number | null;
}

export interface Waterfall {
  rows: WaterfallRow[];
  /** Length of the whole trace. */
  totalMs: number;
  errors: number;
  cancelled: number;
}

const MIN_WIDTH_PCT = 0.6;

/**
 * Lays out the spans of one trace for a waterfall: parents before their children, children ordered
 * by start time, bars placed relative to the start of the trace. Open spans end at `now`.
 */
export function buildWaterfall(spans: readonly Span[], now: number): Waterfall {
  if (spans.length === 0) return { rows: [], totalMs: 0, errors: 0, cancelled: 0 };

  const ids = new Set(spans.map((s) => s.id));
  const children = new Map<string | null, Span[]>();
  for (const span of spans) {
    // A span whose parent is missing (for example removed by retention) is shown as a root.
    const parent = span.parentId !== null && ids.has(span.parentId) ? span.parentId : null;
    children.set(parent, [...(children.get(parent) ?? []), span]);
  }
  for (const list of children.values()) list.sort((a, b) => a.startedAt - b.startedAt);

  const start = Math.min(...spans.map((s) => s.startedAt));
  const end = Math.max(...spans.map((s) => s.endedAt ?? Math.max(now, s.startedAt)));
  const total = Math.max(end - start, 1);

  const rows: WaterfallRow[] = [];
  const visit = (parent: string | null, depth: number): void => {
    for (const span of children.get(parent) ?? []) {
      const spanEnd = span.endedAt ?? Math.max(now, span.startedAt);
      const offsetPct = ((span.startedAt - start) / total) * 100;
      const widthPct = Math.min(
        100 - offsetPct,
        Math.max(((spanEnd - span.startedAt) / total) * 100, MIN_WIDTH_PCT),
      );
      rows.push({
        span,
        depth,
        offsetPct,
        widthPct: Math.max(widthPct, 0),
        durationMs: span.endedAt === null ? null : span.endedAt - span.startedAt,
      });
      visit(span.id, depth + 1);
    }
  };
  visit(null, 0);

  return {
    rows,
    totalMs: total,
    errors: spans.filter((s) => s.status === "error").length,
    cancelled: spans.filter((s) => s.status === "cancelled").length,
  };
}

export function formatDuration(ms: number | null): string {
  if (ms === null) return "running";
  if (ms < 1000) return `${ms} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}
