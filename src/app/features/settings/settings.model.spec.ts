import { describe, expect, it } from "vitest";
import type { TableChanges, WorkspaceChanges } from "../../core/bindings";
import {
  describeSecret,
  describeStorage,
  describeTable,
  formatBytes,
  hasChanges,
  moreNames,
  parseRetention,
  tablesWithRows,
  toRetentionForm,
} from "./settings.model";

describe("parseRetention", () => {
  it("reads whole numbers", () => {
    expect(parseRetention({ maxAgeDays: " 7 ", maxMessages: "500" })).toEqual({
      policy: { maxAgeDays: 7, maxMessages: 500 },
      problems: [],
    });
  });

  it.each([
    ["", "10"],
    ["0", "10"],
    ["1.5", "10"],
    ["-3", "10"],
    ["4000", "10"],
    ["10", "0"],
    ["10", "abc"],
    ["10", "99999999"],
  ])("rejects %j days / %j messages", (maxAgeDays, maxMessages) => {
    const result = parseRetention({ maxAgeDays, maxMessages });
    expect(result.policy).toBeNull();
    expect(result.problems.length).toBeGreaterThan(0);
  });

  it("names both limits when both are wrong", () => {
    expect(parseRetention({ maxAgeDays: "0", maxMessages: "0" }).problems).toHaveLength(2);
  });

  it("shows the stored policy in the form", () => {
    expect(toRetentionForm({ maxAgeDays: 7, maxMessages: 500 })).toEqual({
      maxAgeDays: "7",
      maxMessages: "500",
    });
    expect(toRetentionForm({})).toEqual({ maxAgeDays: "30", maxMessages: "100000" });
  });
});

describe("formatBytes", () => {
  it("picks a readable unit", () => {
    expect(formatBytes(null)).toBe("unknown");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(20 * 1024 * 1024)).toBe("20 MB");
    expect(formatBytes(3 * 1024 ** 3)).toBe("3.0 GB");
  });
});

describe("describeStorage", () => {
  it("summarizes the database", () => {
    expect(describeStorage({ databaseBytes: 2048, messages: 1, historyEntries: 1200 })).toBe(
      "2.0 KB on disk · 1 message · 1,200 tool calls in the history",
    );
  });
});

function table(partial: Partial<TableChanges>): TableChanges {
  return {
    label: "Servers",
    added: [],
    replaced: [],
    addedCount: 0,
    replacedCount: 0,
    unchangedCount: 0,
    ...partial,
  };
}

describe("workspace import summary", () => {
  it("describes what happens to a table", () => {
    expect(describeTable(table({ addedCount: 2, replacedCount: 1, unchangedCount: 3 }))).toBe(
      "2 new, 1 replaced, 3 unchanged",
    );
    expect(describeTable(table({ unchangedCount: 4 }))).toBe("4 unchanged");
  });

  it("leaves out empty tables", () => {
    const changes: WorkspaceChanges = {
      tables: [table({ label: "Flows" }), table({ label: "Servers", addedCount: 1 })],
      missingSecrets: [],
    };
    expect(tablesWithRows(changes).map((t) => t.label)).toEqual(["Servers"]);
  });

  it("knows whether anything would change", () => {
    const none: WorkspaceChanges = { tables: [table({ unchangedCount: 3 })], missingSecrets: [] };
    const some: WorkspaceChanges = { tables: [table({ replacedCount: 1 })], missingSecrets: [] };
    expect(hasChanges(none)).toBe(false);
    expect(hasChanges(some)).toBe(true);
  });

  it("says how many names were cut off", () => {
    expect(moreNames(3, 3)).toBe("");
    expect(moreNames(60, 50)).toBe("and 10 more");
  });

  it("names where a missing secret is used", () => {
    expect(describeSecret({ name: "token", usedBy: ["server A", "environment B"] })).toBe(
      "token (server A, environment B)",
    );
  });
});
