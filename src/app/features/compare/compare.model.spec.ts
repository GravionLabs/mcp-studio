import { describe, expect, it } from "vitest";
import type { CaseResult, VariantResult } from "../../core/bindings";
import {
  BASELINE,
  accuracyOf,
  callCount,
  describeVariant,
  failingOf,
  formatPercent,
  matrix,
  progressText,
  summarize,
  tokensOf,
} from "./compare.model";

const result = (id: string, passed: boolean, overrides: Partial<CaseResult> = {}): CaseResult => ({
  caseId: id,
  input: `input ${id}`,
  expectation: { kind: "tool", name: "t" },
  passed,
  calledTool: passed ? "t" : "other",
  answer: "",
  inputTokens: 100,
  outputTokens: 10,
  error: null,
  ...overrides,
});

const variantResult = (
  id: string,
  cases: CaseResult[],
  inputTokens = 100 * cases.length,
): VariantResult => ({
  variant: { ...BASELINE, id, label: id },
  results: cases,
  passed: cases.filter((c) => c.passed).length,
  total: cases.length,
  inputTokens,
  outputTokens: 10 * cases.length,
  definitionTokens: 500,
});

describe("numbers", () => {
  it("counts calls, accuracy, percent and tokens", () => {
    expect(callCount({ cases: [{}, {}, {}] as never }, [1, 2])).toBe(6);
    expect(accuracyOf({ passed: 1, total: 4 })).toBe(0.25);
    expect(accuracyOf({ passed: 0, total: 0 })).toBe(0);
    expect(formatPercent(0.666)).toBe("67 %");
    expect(tokensOf({ inputTokens: 5, outputTokens: 2 })).toBe(7);
    expect(progressText(3, 12)).toBe("3 of 12 cases done");
  });
});

describe("summarize", () => {
  it("marks the most accurate variant as best", () => {
    const rows = summarize([
      variantResult("a", [result("1", true), result("2", false)]),
      variantResult("b", [result("1", true), result("2", true)]),
    ]);
    expect(rows.map((r) => [r.id, r.accuracy, r.best])).toEqual([
      ["a", 0.5, false],
      ["b", 1, true],
    ]);
    expect(rows[1]?.definitionTokens).toBe(500);
  });

  it("prefers fewer tokens among equally accurate variants", () => {
    const rows = summarize([
      variantResult("a", [result("1", true)], 900),
      variantResult("b", [result("1", true)], 400),
    ]);
    expect(rows.map((r) => r.best)).toEqual([false, true]);
  });

  it("marks nothing when only one variant ran or nothing passed", () => {
    expect(summarize([variantResult("a", [result("1", true)])]).map((r) => r.best)).toEqual([
      false,
    ]);
    expect(
      summarize([
        variantResult("a", [result("1", false)]),
        variantResult("b", [result("1", false)]),
      ]).map((r) => r.best),
    ).toEqual([false, false]);
    expect(summarize([])).toEqual([]);
  });
});

describe("matrix", () => {
  it("puts cases in rows and variants in columns with a note of what the model did", () => {
    const rows = matrix([
      variantResult("a", [
        result("1", true),
        result("2", false, { calledTool: null, answer: "hi" }),
      ]),
      variantResult("b", [result("1", false, { error: "overloaded" })]),
    ]);
    expect(rows).toHaveLength(2);
    expect(rows[0]?.cells).toEqual([
      { passed: true, note: "called t" },
      { passed: false, note: "error: overloaded" },
    ]);
    expect(rows[1]?.cells[0]).toEqual({ passed: false, note: "answered" });
    // Variant b has no result for case 2.
    expect(rows[1]?.cells[1]).toBeNull();
    expect(
      matrix([variantResult("a", [result("1", false, { calledTool: null, answer: "" })])])[0]
        ?.cells[0]?.note,
    ).toBe("no tool, no text");
  });
});

describe("failingOf", () => {
  it("lists the cases that failed but not the cancelled ones", () => {
    const r = variantResult("a", [
      result("1", true),
      result("2", false),
      result("3", false, { error: "cancelled" }),
    ]);
    expect(failingOf(r).map((c) => c.caseId)).toEqual(["2"]);
  });
});

describe("describeVariant", () => {
  it("writes the prompt and the descriptions as Markdown", () => {
    expect(
      describeVariant({
        id: "v1",
        label: "Clearer",
        systemPrompt: "Use tools.",
        toolDescriptions: { list_issues: "Lists open issues", get_issue: "Gets one" },
      }),
    ).toBe(
      "## Clearer\n\n### System prompt\n\nUse tools.\n\n### Tool descriptions\n\n- `list_issues`: Lists open issues\n- `get_issue`: Gets one",
    );
    expect(describeVariant(BASELINE)).toBe("## Current");
  });
});
