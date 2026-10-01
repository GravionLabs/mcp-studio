import type { Flow, StepKind } from "../../core/bindings";

/** Step types in the order they appear in the editor palette. */
export const STEP_TYPES: StepKind["type"][] = [
  "input",
  "llm",
  "tool",
  "condition",
  "transform",
  "output",
];

/** One line that describes a flow in a list, for example `4 steps · 1 tool, 1 LLM`. */
export function describeFlow(flow: Flow): string {
  const count = flow.steps.length;
  const steps = `${count} ${count === 1 ? "step" : "steps"}`;
  const tools = flow.steps.filter((s) => s.type === "tool").length;
  const llms = flow.steps.filter((s) => s.type === "llm").length;
  const parts: string[] = [];
  if (tools > 0) parts.push(`${tools} ${tools === 1 ? "tool" : "tools"}`);
  if (llms > 0) parts.push(`${llms} LLM`);
  return parts.length > 0 ? `${steps} · ${parts.join(", ")}` : steps;
}

/** A starting point for a new flow: an input step and an output step. */
export function newFlow(name: string): Flow {
  return {
    version: 1,
    name,
    steps: [
      { id: "inputs", type: "input", inputs: {} },
      { id: "outputs", type: "output", outputs: {} },
    ],
  };
}
