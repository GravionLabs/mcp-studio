import { describe, expect, it } from "vitest";
import type { Span } from "../../core/bindings";
import { buildWaterfall, formatDuration } from "./waterfall.model";

const span = (overrides: Partial<Span> & Pick<Span, "id">): Span => ({
  traceId: "t",
  parentId: null,
  kind: "tool",
  name: overrides.id,
  startedAt: 0,
  endedAt: 0,
  status: "ok",
  attributes: {},
  tokens: null,
  ...overrides,
});

describe("buildWaterfall", () => {
  it("is empty without spans", () => {
    expect(buildWaterfall([], 0)).toEqual({ rows: [], totalMs: 0, errors: 0, cancelled: 0 });
  });

  it("orders parents before children and places bars relative to the trace", () => {
    const spans = [
      span({ id: "b", parentId: "root", startedAt: 600, endedAt: 800 }),
      span({ id: "root", kind: "session", startedAt: 0, endedAt: 1000 }),
      span({ id: "a", parentId: "root", startedAt: 100, endedAt: 300 }),
    ];
    const { rows, totalMs } = buildWaterfall(spans, 5000);
    expect(totalMs).toBe(1000);
    expect(rows.map((r) => [r.span.id, r.depth])).toEqual([
      ["root", 0],
      ["a", 1],
      ["b", 1],
    ]);
    expect(rows[1]).toMatchObject({ offsetPct: 10, widthPct: 20, durationMs: 200 });
    expect(rows[0]).toMatchObject({ offsetPct: 0, widthPct: 100, durationMs: 1000 });
  });

  it("nests deeper levels", () => {
    const spans = [
      span({ id: "r", startedAt: 0, endedAt: 10 }),
      span({ id: "c", parentId: "r", startedAt: 1, endedAt: 9 }),
      span({ id: "g", parentId: "c", startedAt: 2, endedAt: 8 }),
    ];
    expect(buildWaterfall(spans, 0).rows.map((r) => r.depth)).toEqual([0, 1, 2]);
  });

  it("keeps instant spans visible", () => {
    const spans = [
      span({ id: "r", startedAt: 0, endedAt: 1000 }),
      span({ id: "i", parentId: "r", startedAt: 500, endedAt: 500 }),
    ];
    const bar = buildWaterfall(spans, 0).rows[1];
    expect(bar?.widthPct).toBeGreaterThan(0);
    expect(bar?.durationMs).toBe(0);
  });

  it("lets open spans run until now and reports no duration", () => {
    const spans = [
      span({ id: "r", startedAt: 0, endedAt: null }),
      span({ id: "c", parentId: "r", startedAt: 200, endedAt: null }),
    ];
    const { rows, totalMs } = buildWaterfall(spans, 1000);
    expect(totalMs).toBe(1000);
    expect(rows[1]).toMatchObject({ durationMs: null, offsetPct: 20, widthPct: 80 });
  });

  it("shows a span with a missing parent as a root", () => {
    const rows = buildWaterfall([span({ id: "x", parentId: "gone" })], 0).rows;
    expect(rows[0]?.depth).toBe(0);
  });

  it("counts errors and cancelled spans", () => {
    const spans = [
      span({ id: "a", status: "error" }),
      span({ id: "b", status: "error" }),
      span({ id: "c", status: "cancelled" }),
      span({ id: "d" }),
    ];
    const result = buildWaterfall(spans, 0);
    expect(result.errors).toBe(2);
    expect(result.cancelled).toBe(1);
  });
});

describe("formatDuration", () => {
  it("formats milliseconds and seconds", () => {
    expect(formatDuration(0)).toBe("0 ms");
    expect(formatDuration(999)).toBe("999 ms");
    expect(formatDuration(1250)).toBe("1.25 s");
    expect(formatDuration(12_345)).toBe("12.3 s");
  });

  it("marks open spans", () => {
    expect(formatDuration(null)).toBe("running");
  });
});
