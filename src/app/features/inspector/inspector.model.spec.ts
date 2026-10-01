import { describe, expect, it } from "vitest";
import type { MessageRecord } from "../../core/bindings";
import {
  NO_FILTER,
  formatBytes,
  formatClock,
  formatTokens,
  tokenSourceHint,
  labelFor,
  matchesFilter,
  prettyPayload,
} from "./inspector.model";

const message = (overrides: Partial<MessageRecord> = {}) => ({
  id: 1,
  sessionId: "sess",
  serverId: "srv",
  direction: "out" as const,
  jsonrpcId: "1",
  method: "tools/call" as string | null,
  payload: '{"id":1,"method":"tools/call","params":{"name":"echo"}}',
  bytes: 55,
  isError: false,
  ts: 0,
  durationMs: null,
  ...overrides,
});

describe("matchesFilter", () => {
  it("accepts everything without a filter", () => {
    expect(matchesFilter(message(), NO_FILTER)).toBe(true);
  });

  it("filters by server, direction, errors, and text", () => {
    expect(matchesFilter(message({ serverId: "a" }), { ...NO_FILTER, serverId: "b" })).toBe(false);
    expect(matchesFilter(message({ serverId: "a" }), { ...NO_FILTER, serverId: "a" })).toBe(true);
    expect(matchesFilter(message(), { ...NO_FILTER, direction: "in" })).toBe(false);
    expect(matchesFilter(message(), { ...NO_FILTER, errorsOnly: true })).toBe(false);
    expect(matchesFilter(message({ isError: true }), { ...NO_FILTER, errorsOnly: true })).toBe(
      true,
    );
    expect(matchesFilter(message(), { ...NO_FILTER, search: "ECHO" })).toBe(true);
    expect(matchesFilter(message(), { ...NO_FILTER, search: "nothing" })).toBe(false);
  });

  it("matches responses through their request's method", () => {
    const response = message({ method: null, direction: "in" });
    const lookup = () => "tools/call";
    expect(matchesFilter(response, { ...NO_FILTER, method: "tools/call" }, lookup)).toBe(true);
    expect(matchesFilter(response, { ...NO_FILTER, method: "tools/list" }, lookup)).toBe(false);
    expect(matchesFilter(response, { ...NO_FILTER, method: "tools/call" })).toBe(false);
  });
});

describe("labelFor", () => {
  it("names requests with their target", () => {
    expect(labelFor(message())).toEqual({ text: "tools/call · echo", kind: "request" });
  });

  it("marks notifications, results, and errors", () => {
    expect(
      labelFor(message({ jsonrpcId: null, method: "notifications/initialized", payload: "{}" })),
    ).toEqual({
      text: "notifications/initialized",
      kind: "notification",
    });
    expect(labelFor(message({ method: null, payload: "{}" }), "tools/list")).toEqual({
      text: "result · tools/list",
      kind: "response",
    });
    expect(labelFor(message({ method: null, isError: true, payload: "{}" }))).toEqual({
      text: "error",
      kind: "error",
    });
  });

  it("survives payloads that are not JSON", () => {
    expect(labelFor(message({ payload: "garbage" })).text).toBe("tools/call");
  });
});

describe("formatting", () => {
  it("formats the clock with milliseconds", () => {
    const ts = new Date(2026, 0, 1, 9, 5, 7, 42).getTime();
    expect(formatClock(ts)).toBe("09:05:07.042");
  });

  it("formats sizes", () => {
    expect(formatBytes(12)).toBe("12 B");
    expect(formatBytes(2048)).toBe("2.0 KiB");
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0 MiB");
  });

  it("pretty-prints JSON and keeps other text", () => {
    expect(prettyPayload('{"a":1}')).toBe('{\n  "a": 1\n}');
    expect(prettyPayload("nope")).toBe("nope");
  });
});

describe("formatTokens", () => {
  it("marks estimates with a tilde", () => {
    expect(formatTokens(42, "estimate")).toBe("~42 tokens");
    expect(formatTokens(0, "estimate")).toBe("~0 tokens");
  });

  it("shortens large counts", () => {
    expect(formatTokens(1234, "estimate")).toBe("~1.2k tokens");
  });

  it("shows exact counts without a tilde", () => {
    expect(formatTokens(1234, "exact")).toBe("1.2k tokens");
    expect(formatTokens(7, "exact")).toBe("7 tokens");
  });

  it("explains the source", () => {
    expect(tokenSourceHint("estimate")).toContain("Estimated");
    expect(tokenSourceHint("exact")).toContain("Exact");
  });
});
