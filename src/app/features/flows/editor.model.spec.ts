import { describe, expect, it } from "vitest";
import type { Flow } from "../../core/bindings";
import {
  addStep,
  connectJump,
  connectionsOf,
  inConnector,
  jumpTargets,
  layoutOf,
  moveStep,
  outConnector,
  parseConnector,
  placeAfter,
  removeStep,
  renameStep,
  summarize,
  uniqueStepId,
  updateStep,
} from "./editor.model";

const flow = (): Flow => ({
  version: 1,
  name: "f",
  steps: [
    { id: "inputs", type: "input", inputs: { repo: { type: "string" } } },
    {
      id: "issues",
      type: "tool",
      server: "github",
      tool: "list",
      arguments: { repo: "{{ inputs.repo }}" },
    },
    {
      id: "check",
      type: "condition",
      expression: "len(steps.issues.result) > 0",
      then: "summary",
      else: null,
    },
    { id: "empty", type: "transform", values: { text: "none from {{ steps.issues.result }}" } },
    {
      id: "summary",
      type: "llm",
      model: "claude-sonnet-5-5",
      prompt: "Summarize {{ steps.issues.result }}",
      system: null,
      tools: [],
    },
    { id: "outputs", type: "output", outputs: { text: "{{ steps.summary.text }}" } },
  ],
});

const ids = (f: Flow) => f.steps.map((s) => s.id);

describe("adding steps", () => {
  it("adds a step of every type with sensible defaults", () => {
    let f: Flow = { version: 1, name: "n", steps: [] };
    for (const type of ["input", "llm", "tool", "condition", "transform", "output"] as const) {
      f = addStep(f, type);
    }
    expect(ids(f)).toEqual(["inputs", "llm", "tool", "condition", "transform", "outputs"]);
    expect(f.steps[3]).toMatchObject({ type: "condition", expression: "", then: null, else: null });
    expect(f.steps[1]).toMatchObject({ type: "llm", prompt: "", tools: [] });
  });

  it("numbers ids that are taken", () => {
    let f = addStep(flow(), "tool");
    f = addStep(f, "tool");
    expect(ids(f).filter((i) => i.startsWith("tool"))).toEqual(["tool", "tool1"]);
    expect(uniqueStepId(f, "tool")).toBe("tool2");
  });

  it("inserts after a given step", () => {
    const f = addStep(flow(), "transform", "issues");
    expect(ids(f).slice(0, 3)).toEqual(["inputs", "issues", "transform"]);
  });
});

describe("removing and moving steps", () => {
  it("removes a step and clears jumps to it", () => {
    const f = removeStep(flow(), "summary");
    expect(ids(f)).not.toContain("summary");
    expect(f.steps.find((s) => s.id === "check")).toMatchObject({ then: null });
  });

  it("moves a step up and down and stays within the list", () => {
    expect(ids(moveStep(flow(), "empty", -1)).slice(2, 4)).toEqual(["empty", "check"]);
    expect(ids(moveStep(flow(), "empty", 1)).slice(3, 5)).toEqual(["summary", "empty"]);
    const f = flow();
    expect(moveStep(f, "inputs", -1)).toBe(f);
    expect(moveStep(f, "outputs", 1)).toBe(f);
    expect(moveStep(f, "ghost", 1)).toBe(f);
  });

  it("places a step right after another", () => {
    expect(ids(placeAfter(flow(), "summary", "issues")).slice(0, 4)).toEqual([
      "inputs",
      "issues",
      "summary",
      "check",
    ]);
    const f = flow();
    expect(placeAfter(f, "issues", "issues")).toBe(f);
    expect(placeAfter(f, "ghost", "issues")).toBe(f);
    expect(placeAfter(f, "issues", "ghost")).toBe(f);
  });
});

describe("updating steps", () => {
  it("changes the fields of one step only", () => {
    const f = updateStep(flow(), "summary", { prompt: "Be brief" });
    expect(f.steps.find((s) => s.id === "summary")).toMatchObject({
      prompt: "Be brief",
      model: "claude-sonnet-5-5",
    });
    expect(f.steps.find((s) => s.id === "issues")).toEqual(flow().steps[1]);
  });
});

describe("renaming steps", () => {
  it("follows the new id in jumps and in every template", () => {
    const f = renameStep(flow(), "issues", "fetch");
    expect(ids(f)[1]).toBe("fetch");
    const text = JSON.stringify(f);
    expect(text).not.toContain("steps.issues");
    expect(text).toContain("steps.fetch.result");
    const g = renameStep(flow(), "summary", "answer");
    expect(g.steps.find((s) => s.id === "check")).toMatchObject({ then: "answer" });
    expect(JSON.stringify(g)).toContain("{{ steps.answer.text }}");
  });

  it("does not touch longer ids that start with the old one", () => {
    const f: Flow = {
      version: 1,
      name: "x",
      steps: [
        { id: "a", type: "transform", values: {} },
        { id: "ab", type: "transform", values: {} },
        { id: "o", type: "output", outputs: { x: "{{ steps.a.x }} {{ steps.ab.x }} steps.a2" } },
      ],
    };
    const renamed = renameStep(f, "a", "first");
    expect(JSON.stringify(renamed)).toContain("{{ steps.first.x }} {{ steps.ab.x }} steps.a2");
  });

  it("refuses ids that are invalid or taken", () => {
    const f = flow();
    expect(renameStep(f, "issues", "has space")).toBe(f);
    expect(renameStep(f, "issues", "1abc")).toBe(f);
    expect(renameStep(f, "issues", "summary")).toBe(f);
    expect(renameStep(f, "issues", "issues")).toBe(f);
    expect(renameStep(f, "issues", "")).toBe(f);
  });
});

describe("connecting steps", () => {
  it("sets a jump on one branch of a condition", () => {
    const f = connectJump(flow(), "check", "else", "outputs");
    expect(f?.steps.find((s) => s.id === "check")).toMatchObject({
      then: "summary",
      else: "outputs",
    });
    const cleared = connectJump(flow(), "check", "then", null);
    expect(cleared?.steps.find((s) => s.id === "check")).toMatchObject({ then: null });
  });

  it("only jumps forward and only from conditions", () => {
    expect(connectJump(flow(), "check", "then", "inputs")).toBeNull();
    expect(connectJump(flow(), "check", "then", "check")).toBeNull();
    expect(connectJump(flow(), "issues", "then", "outputs")).toBeNull();
    expect(connectJump(flow(), "ghost", "then", "outputs")).toBeNull();
  });

  it("lists the steps a condition can jump to", () => {
    expect(jumpTargets(flow(), "check")).toEqual(["empty", "summary", "outputs"]);
    expect(jumpTargets(flow(), "ghost")).toEqual([]);
  });
});

describe("the picture", () => {
  it("connects every step to the next and a condition to both of its branches", () => {
    const connections = connectionsOf(flow());
    const find = (id: string) => connections.find((c) => c.id === id);
    expect(find("inputs-next")).toEqual({
      id: "inputs-next",
      source: outConnector("inputs"),
      target: inConnector("issues"),
      kind: "next",
    });
    // then jumps to summary; else has no jump, so it falls through to the next step.
    expect(find("check-then")?.target).toBe(inConnector("summary"));
    expect(find("check-else")?.target).toBe(inConnector("empty"));
    expect(connections.some((c) => c.id === "outputs-next")).toBe(false);
    expect(connections).toHaveLength(6);
  });

  it("has no connections without steps or with a single step", () => {
    expect(connectionsOf({ version: 1, name: "x", steps: [] })).toEqual([]);
    expect(connectionsOf({ version: 1, name: "x", steps: [flow().steps[0]!] })).toEqual([]);
  });

  it("lays the steps out in one column in running order", () => {
    const layout = layoutOf(flow());
    expect(layout.get("inputs")).toEqual({ x: 0, y: 0 });
    expect(layout.get("issues")?.y).toBeGreaterThan(0);
    expect([...layout.keys()]).toEqual(ids(flow()));
  });

  it("reads connector ids back", () => {
    expect(parseConnector("in-issues")).toEqual({
      direction: "in",
      stepId: "issues",
      branch: null,
    });
    expect(parseConnector("out-issues")).toEqual({
      direction: "out",
      stepId: "issues",
      branch: null,
    });
    expect(parseConnector("out-check-then")).toEqual({
      direction: "out",
      stepId: "check",
      branch: "then",
    });
    expect(parseConnector("out-check-else")).toEqual({
      direction: "out",
      stepId: "check",
      branch: "else",
    });
    // `in-` connectors have no branch, even if the id ends like one.
    expect(parseConnector("in-a-then")).toEqual({
      direction: "in",
      stepId: "a-then",
      branch: null,
    });
    expect(parseConnector("nonsense")).toBeNull();
  });

  it("summarizes steps for the nodes", () => {
    const steps = flow().steps;
    expect(summarize(steps[0]!)).toBe("1 inputs");
    expect(summarize(steps[1]!)).toBe("github / list");
    expect(summarize(steps[2]!)).toContain("len(");
    expect(summarize(steps[4]!)).toBe("claude-sonnet-5-5");
    expect(summarize({ id: "t", type: "tool", server: "", tool: "", arguments: {} })).toBe(
      "no tool chosen",
    );
  });
});
