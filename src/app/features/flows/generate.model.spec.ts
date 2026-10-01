import { describe, expect, it } from "vitest";
import { MAX_GOAL_CHARS, blocker, issueLines, outcome, toggled } from "./generate.model";

describe("blocker", () => {
  it("explains what is missing", () => {
    expect(blocker("  ", ["a"])).toMatch(/Describe/);
    expect(blocker("x".repeat(MAX_GOAL_CHARS + 1), ["a"])).toMatch(/longer/);
    expect(blocker("do it", [])).toMatch(/server/);
    expect(blocker("do it", ["a"])).toBeNull();
  });
});

describe("toggled", () => {
  it("adds and removes", () => {
    expect(toggled(["a"], "b")).toEqual(["a", "b"]);
    expect(toggled(["a", "b"], "a")).toEqual(["b"]);
  });
});

describe("issueLines", () => {
  it("names the step when there is one", () => {
    expect(
      issueLines([
        { stepId: "s1", message: "unknown tool" },
        { stepId: null, message: "needs a name" },
      ]),
    ).toEqual(["s1: unknown tool", "needs a name"]);
  });
});

describe("outcome", () => {
  it("says whether the flow may be opened", () => {
    expect(outcome({ issues: [], attempts: 1 })).toBe("Generated a valid flow.");
    expect(outcome({ issues: [], attempts: 2 })).toBe("Generated a valid flow after one repair.");
    expect(outcome({ issues: [1], attempts: 2 })).toBe(
      "The model's flow has 1 problem after one repair, so it was not opened.",
    );
    expect(outcome({ issues: [1, 2], attempts: 1 })).toContain("2 problems");
  });
});
