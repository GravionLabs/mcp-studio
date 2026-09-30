import { describe, expect, it } from "vitest";
import { diffJson, preview, summarizeDiff } from "./json-diff";

describe("diffJson", () => {
  it("returns nothing for equal values", () => {
    expect(diffJson({ a: [1, { b: 2 }] }, { a: [1, { b: 2 }] })).toEqual([]);
    expect(diffJson("x", "x")).toEqual([]);
    expect(diffJson(null, null)).toEqual([]);
  });

  it("reports added, removed, and changed keys with their paths", () => {
    const entries = diffJson(
      { name: "echo", params: { a: 1, gone: true } },
      { name: "echo", params: { a: 2, extra: "x" } },
    );
    expect(entries).toEqual([
      { path: "/params/a", kind: "changed", before: 1, after: 2 },
      { path: "/params/extra", kind: "added", after: "x" },
      { path: "/params/gone", kind: "removed", before: true },
    ]);
  });

  it("compares arrays by index", () => {
    expect(diffJson({ l: [1, 2, 3] }, { l: [1, 9] })).toEqual([
      { path: "/l/1", kind: "changed", before: 2, after: 9 },
      { path: "/l/2", kind: "removed", before: 3 },
    ]);
    expect(diffJson([1], [1, 2])).toEqual([{ path: "/1", kind: "added", after: 2 }]);
  });

  it("treats a change of type as one change", () => {
    expect(diffJson({ v: [1] }, { v: { a: 1 } })).toEqual([
      { path: "/v", kind: "changed", before: [1], after: { a: 1 } },
    ]);
    expect(diffJson(1, "1")).toEqual([{ path: "", kind: "changed", before: 1, after: "1" }]);
  });

  it("ignores top-level keys on request, but not nested ones", () => {
    const a = { id: 1, params: { id: 1 } };
    const b = { id: 2, params: { id: 2 } };
    expect(diffJson(a, b, { ignoreTopLevel: ["id"] })).toEqual([
      { path: "/params/id", kind: "changed", before: 1, after: 2 },
    ]);
  });

  it("escapes slashes and tildes in keys", () => {
    expect(diffJson({ "a/b": 1, "c~d": 1 }, { "a/b": 2, "c~d": 2 }).map((e) => e.path)).toEqual([
      "/a~1b",
      "/c~0d",
    ]);
  });
});

describe("helpers", () => {
  it("previews values and truncates long ones", () => {
    expect(preview({ a: 1 })).toBe('{"a":1}');
    expect(preview(undefined)).toBe("");
    expect(preview("x".repeat(200), 10)).toHaveLength(10);
  });

  it("summarizes entries", () => {
    expect(
      summarizeDiff([
        { path: "/a", kind: "added" },
        { path: "/b", kind: "removed" },
        { path: "/c", kind: "changed" },
        { path: "/d", kind: "changed" },
      ]),
    ).toEqual({ added: 1, removed: 1, changed: 2 });
  });
});
