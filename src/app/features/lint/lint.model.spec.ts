import { describe, expect, it } from "vitest";
import type { LintFinding, LintReport } from "../../core/bindings";
import { countBySeverity, groupByTool, ruleTitle, summarize, worst } from "./lint.model";

const finding = (overrides: Partial<LintFinding> = {}): LintFinding => ({
  tool: "a",
  rule: "vagueDescription",
  severity: "warning",
  message: "m",
  related: [],
  ...overrides,
});

const report = (findings: LintFinding[], toolsChecked = 3): LintReport => ({
  findings,
  toolsChecked,
  totalTokens: 100,
});

describe("groupByTool", () => {
  it("groups findings by tool and keeps the order, server first", () => {
    const groups = groupByTool(
      report([
        finding({ tool: null, rule: "oversizedServer" }),
        finding({ tool: "a", severity: "error", rule: "missingDescription" }),
        finding({ tool: "a", severity: "info", rule: "undescribedParameter" }),
        finding({ tool: "b" }),
      ]),
    );
    expect(groups.map((g) => [g.tool, g.findings.length, g.severity])).toEqual([
      [null, 1, "warning"],
      ["a", 2, "error"],
      ["b", 1, "warning"],
    ]);
  });

  it("is empty without findings", () => {
    expect(groupByTool(report([]))).toEqual([]);
  });
});

describe("severity", () => {
  it("finds the worst severity", () => {
    expect(worst([finding({ severity: "info" }), finding({ severity: "warning" })])).toBe(
      "warning",
    );
    expect(worst([finding({ severity: "error" }), finding({ severity: "info" })])).toBe("error");
    expect(worst([])).toBe("info");
  });

  it("counts by severity", () => {
    expect(
      countBySeverity(
        report([
          finding({ severity: "error" }),
          finding({ severity: "warning" }),
          finding({ severity: "warning" }),
        ]),
      ),
    ).toEqual({ error: 1, warning: 2, info: 0 });
  });
});

describe("summarize", () => {
  it("says when nothing was found", () => {
    expect(summarize(report([], 1))).toBe("1 tool checked: no problems found");
  });

  it("counts what was found", () => {
    expect(
      summarize(
        report([
          finding({ severity: "error" }),
          finding({ severity: "warning" }),
          finding({ severity: "warning" }),
          finding({ severity: "info" }),
        ]),
      ),
    ).toBe("3 tools checked: 1 error, 2 warnings, 1 hint");
  });
});

describe("ruleTitle", () => {
  it("has a title for every rule", () => {
    expect(ruleTitle("overlappingTools")).toBe("Overlapping tools");
    expect(ruleTitle("missingRequired")).toBe("No required parameters");
  });
});
