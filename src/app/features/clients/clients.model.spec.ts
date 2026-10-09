import { describe, expect, it } from "vitest";
import type { ClientEntry } from "../../core/bindings";
import {
  canRestore,
  canRoute,
  entryStatus,
  forceQuestion,
  groupEntries,
  statusLabel,
} from "./clients.model";

function entry(overrides: Partial<ClientEntry> = {}): ClientEntry {
  return {
    path: "/home/me/.claude.json",
    pointer: "/mcpServers",
    name: "files",
    client: "Claude Code (user)",
    origin: "top level",
    kind: "stdio",
    summary: "npx files",
    routed: false,
    routeId: null,
    unsupported: null,
    ...overrides,
  };
}

describe("entryStatus", () => {
  it("tells direct, routed and unsupported entries apart", () => {
    expect(entryStatus(entry())).toBe("direct");
    expect(entryStatus(entry({ routed: true, routeId: "r1" }))).toBe("routed");
    expect(entryStatus(entry({ routed: true }))).toBe("routed-by-hand");
    expect(entryStatus(entry({ kind: "unsupported", unsupported: "legacy SSE" }))).toBe(
      "unsupported",
    );
  });

  it("offers routing only for direct entries and restoring only for entries MCP Studio routed", () => {
    expect(canRoute(entry())).toBe(true);
    expect(canRestore(entry())).toBe(false);
    const routed = entry({ routed: true, routeId: "r1" });
    expect(canRoute(routed)).toBe(false);
    expect(canRestore(routed)).toBe(true);
    const byHand = entry({ routed: true });
    expect(canRoute(byHand)).toBe(false);
    expect(canRestore(byHand)).toBe(false);
    expect(canRoute(entry({ kind: "unsupported" }))).toBe(false);
  });

  it("labels each state", () => {
    expect(statusLabel(entry())).toBe("Direct");
    expect(statusLabel(entry({ routed: true, routeId: "r" }))).toBe("Through MCP Studio");
    expect(statusLabel(entry({ routed: true }))).toBe("Through MCP Studio (set up by hand)");
    expect(statusLabel(entry({ kind: "unsupported", unsupported: "legacy SSE" }))).toBe(
      "legacy SSE",
    );
  });
});

describe("groupEntries", () => {
  it("groups by file and sorts by scope and name", () => {
    const groups = groupEntries([
      entry({ name: "b" }),
      entry({ name: "z", origin: "project /a" }),
      entry({ name: "a" }),
      entry({ client: "Claude Desktop", path: "/desktop.json", name: "x" }),
    ]);
    expect(groups.map((g) => g.client)).toEqual(["Claude Code (user)", "Claude Desktop"]);
    expect(groups[0].entries.map((e) => e.name)).toEqual(["a", "b", "z"]);
  });

  it("is empty without entries", () => {
    expect(groupEntries([])).toEqual([]);
  });
});

describe("forceQuestion", () => {
  it("repeats the conflict and says what forcing does", () => {
    expect(forceQuestion("The entry was changed.")).toContain("The entry was changed.");
    expect(forceQuestion("x")).toContain("anyway");
  });
});
