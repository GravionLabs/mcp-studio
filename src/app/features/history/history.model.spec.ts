import { describe, expect, it } from "vitest";
import type { HistoryEntry } from "../../core/bindings";
import { formatDuration, formatTimestamp, promptArguments, summarize } from "./history.model";

const entry = (overrides: Partial<HistoryEntry>): HistoryEntry => ({
  id: 1,
  serverId: "s",
  method: "tools/call",
  target: "echo",
  arguments: {},
  isError: false,
  cancelled: false,
  durationMs: 5,
  result: null,
  error: null,
  ts: 0,
  ...overrides,
});

describe("summarize", () => {
  it("classifies methods", () => {
    expect(summarize(entry({}))).toEqual({ title: "echo", kind: "tool", status: "ok" });
    expect(summarize(entry({ method: "resources/read", target: "file:///a" })).kind).toBe(
      "resource",
    );
    expect(summarize(entry({ method: "prompts/get", target: "greet" })).kind).toBe("prompt");
    expect(summarize(entry({ method: "x/y", target: "z" }))).toMatchObject({
      title: "x/y z",
      kind: "other",
    });
  });

  it("prefers cancelled over error", () => {
    expect(summarize(entry({ isError: true })).status).toBe("error");
    expect(summarize(entry({ isError: true, cancelled: true })).status).toBe("cancelled");
  });
});

describe("promptArguments", () => {
  it("stringifies values and rejects non-objects", () => {
    expect(promptArguments(entry({ arguments: { name: "Ada", n: 3 } }))).toEqual({
      name: "Ada",
      n: "3",
    });
    expect(promptArguments(entry({ arguments: [1] }))).toEqual({});
    expect(promptArguments(entry({ arguments: null }))).toEqual({});
  });
});

describe("formatting", () => {
  it("formats durations", () => {
    expect(formatDuration(12)).toBe("12 ms");
    expect(formatDuration(1400)).toBe("1.4 s");
    expect(formatDuration(null)).toBe("");
  });

  it("formats local timestamps with zero padding", () => {
    const ts = new Date(2026, 0, 5, 3, 4, 9).getTime();
    expect(formatTimestamp(ts)).toBe("2026-01-05 03:04:09");
  });
});
