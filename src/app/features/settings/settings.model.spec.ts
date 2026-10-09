import { describe, expect, it } from "vitest";
import { describeStorage, formatBytes, parseRetention, toRetentionForm } from "./settings.model";

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
