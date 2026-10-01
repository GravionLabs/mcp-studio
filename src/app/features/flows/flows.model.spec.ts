import { describe, expect, it } from "vitest";
import type { Flow } from "../../core/bindings";
import { STEP_TYPES, describeFlow, newFlow } from "./flows.model";

const flow = (steps: Flow["steps"]): Flow => ({ version: 1, name: "f", steps });

describe("describeFlow", () => {
  it("counts steps", () => {
    expect(describeFlow(flow([]))).toBe("0 steps");
    expect(describeFlow(flow([{ id: "o", type: "output", outputs: {} }]))).toBe("1 step");
  });

  it("mentions tool and LLM steps", () => {
    const steps: Flow["steps"] = [
      { id: "a", type: "tool", server: "s", tool: "t", arguments: {} },
      { id: "b", type: "tool", server: "s", tool: "u", arguments: {} },
      { id: "c", type: "llm", model: "m", prompt: "p", system: null, tools: [] },
    ];
    expect(describeFlow(flow(steps))).toBe("3 steps · 2 tools, 1 LLM");
  });
});

describe("newFlow", () => {
  it("starts with an input and an output step that use the shorthand ids", () => {
    const created = newFlow("My flow");
    expect(created.name).toBe("My flow");
    expect(created.steps.map((s) => [s.id, s.type])).toEqual([
      ["inputs", "input"],
      ["outputs", "output"],
    ]);
  });
});

describe("STEP_TYPES", () => {
  it("lists every step type of the format once", () => {
    expect(new Set(STEP_TYPES).size).toBe(6);
    expect(STEP_TYPES).toContain("condition");
  });
});
