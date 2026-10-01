import type { Flow, Step, StepKind } from "../../core/bindings";

export type StepType = StepKind["type"];

/** The pieces of a step that the form edits. */
export type StepPatch = Partial<Omit<Step, "id" | "type">> & Record<string, unknown>;

const SEQUENCE_STEP_HEIGHT = 140;

function emptyKind(type: StepType): StepKind {
  switch (type) {
    case "input":
      return { type, inputs: {} };
    case "llm":
      return { type, model: "claude-sonnet-5-5", prompt: "", system: null, tools: [] };
    case "tool":
      return { type, server: "", tool: "", arguments: {} };
    case "condition":
      return { type, expression: "", then: null, else: null };
    case "transform":
      return { type, values: {} };
    case "output":
      return { type, outputs: {} };
  }
}

/** An id like `tool1` that no step of the flow uses. */
export function uniqueStepId(flow: Flow, base: string): string {
  const taken = new Set(flow.steps.map((s) => s.id));
  if (!taken.has(base)) return base;
  let n = 1;
  while (taken.has(`${base}${n}`)) n += 1;
  return `${base}${n}`;
}

/**
 * Adds a step of `type` after the step `afterId`, or at the end. The shorthand ids `inputs` and
 * `outputs` are used for the first input and output step, so a new flow reads like the spec.
 */
export function addStep(flow: Flow, type: StepType, afterId: string | null = null): Flow {
  const base = type === "input" ? "inputs" : type === "output" ? "outputs" : type;
  const step = { id: uniqueStepId(flow, base), ...emptyKind(type) } as Step;
  const at =
    afterId === null ? flow.steps.length : flow.steps.findIndex((s) => s.id === afterId) + 1;
  const steps = [...flow.steps];
  steps.splice(at < 0 ? steps.length : at, 0, step);
  return { ...flow, steps };
}

/** Removes a step; conditions that jumped to it fall through instead. */
export function removeStep(flow: Flow, id: string): Flow {
  const steps = flow.steps
    .filter((s) => s.id !== id)
    .map((s) =>
      s.type === "condition"
        ? { ...s, then: s.then === id ? null : s.then, else: s.else === id ? null : s.else }
        : s,
    );
  return { ...flow, steps };
}

/** Moves a step up (`-1`) or down (`1`) in the order. */
export function moveStep(flow: Flow, id: string, delta: -1 | 1): Flow {
  const index = flow.steps.findIndex((s) => s.id === id);
  const target = index + delta;
  if (index < 0 || target < 0 || target >= flow.steps.length) return flow;
  const steps = [...flow.steps];
  const [moved] = steps.splice(index, 1);
  if (moved) steps.splice(target, 0, moved);
  return { ...flow, steps };
}

/** Moves `id` to directly after `afterId`, so that one runs right after the other. */
export function placeAfter(flow: Flow, id: string, afterId: string): Flow {
  if (id === afterId) return flow;
  const moved = flow.steps.find((s) => s.id === id);
  if (!moved) return flow;
  const rest = flow.steps.filter((s) => s.id !== id);
  const at = rest.findIndex((s) => s.id === afterId) + 1;
  if (at === 0) return flow;
  rest.splice(at, 0, moved);
  return { ...flow, steps: rest };
}

/** Changes fields of a step (not its id or type). */
export function updateStep(flow: Flow, id: string, patch: StepPatch): Flow {
  return {
    ...flow,
    steps: flow.steps.map((s) => (s.id === id ? ({ ...s, ...patch } as Step) : s)),
  };
}

const escapeRegExp = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** Rewrites `steps.<old>` references in text to `steps.<next>`. */
function renameReferences(text: string, from: string, to: string): string {
  return text.replace(
    new RegExp(`\\bsteps\\.${escapeRegExp(from)}(?![A-Za-z0-9_])`, "g"),
    `steps.${to}`,
  );
}

function mapStrings(value: unknown, change: (text: string) => string): unknown {
  if (typeof value === "string") return change(value);
  if (Array.isArray(value)) return value.map((v) => mapStrings(v, change));
  if (value !== null && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value as Record<string, unknown>).map(([k, v]) => [k, mapStrings(v, change)]),
    );
  }
  return value;
}

/**
 * Gives a step a new id and follows it everywhere: jumps of conditions and `steps.<id>` in every
 * template and expression. Returns the flow unchanged when `next` is empty, not a valid id, or
 * already used.
 */
export function renameStep(flow: Flow, id: string, next: string): Flow {
  if (next === id || !/^[A-Za-z_][A-Za-z0-9_]*$/.test(next)) return flow;
  if (flow.steps.some((s) => s.id === next)) return flow;
  const steps = flow.steps.map((step) => {
    const { id: stepId, ...rest } = step;
    const rewritten = mapStrings(rest, (t) => renameReferences(t, id, next)) as Record<
      string,
      unknown
    >;
    if (step.type === "condition") {
      if (step.then === id) rewritten["then"] = next;
      if (step.else === id) rewritten["else"] = next;
    }
    return { id: stepId === id ? next : stepId, ...rewritten } as Step;
  });
  return { ...flow, steps };
}

export type Branch = "then" | "else";

/**
 * Makes a condition jump to `targetId` on one branch. Jumps only go forward, so the target must
 * come after the condition; returns `null` otherwise.
 */
export function connectJump(
  flow: Flow,
  conditionId: string,
  branch: Branch,
  targetId: string | null,
): Flow | null {
  const from = flow.steps.findIndex((s) => s.id === conditionId);
  const condition = flow.steps[from];
  if (condition?.type !== "condition") return null;
  if (targetId !== null) {
    const to = flow.steps.findIndex((s) => s.id === targetId);
    if (to <= from) return null;
  }
  return updateStep(flow, conditionId, { [branch]: targetId });
}

/** The steps a condition can jump to: the ones after it, except the next one (that is a no-op). */
export function jumpTargets(flow: Flow, conditionId: string): string[] {
  const from = flow.steps.findIndex((s) => s.id === conditionId);
  return from < 0 ? [] : flow.steps.slice(from + 1).map((s) => s.id);
}

// ---- the picture --------------------------------------------------------------------------------

export const inConnector = (stepId: string): string => `in-${stepId}`;
export const outConnector = (stepId: string, branch?: Branch): string =>
  branch ? `out-${stepId}-${branch}` : `out-${stepId}`;

export interface EditorConnection {
  id: string;
  /** Connector id of the source (an `out-…` connector). */
  source: string;
  /** Connector id of the target (an `in-…` connector). */
  target: string;
  /** Which branch of a condition this is, or `next` for the order of the steps. */
  kind: Branch | "next";
}

/**
 * The connections of a flow: every step leads to the next one; a condition has a `then` and an
 * `else` connection, to the step it jumps to or to the next one when it has no jump.
 */
export function connectionsOf(flow: Flow): EditorConnection[] {
  const connections: EditorConnection[] = [];
  flow.steps.forEach((step, index) => {
    const next = flow.steps[index + 1];
    if (step.type === "condition") {
      for (const branch of ["then", "else"] as const) {
        const target = step[branch] ?? next?.id ?? null;
        if (target !== null) {
          connections.push({
            id: `${step.id}-${branch}`,
            source: outConnector(step.id, branch),
            target: inConnector(target),
            kind: branch,
          });
        }
      }
    } else if (next) {
      connections.push({
        id: `${step.id}-next`,
        source: outConnector(step.id),
        target: inConnector(next.id),
        kind: "next",
      });
    }
  });
  return connections;
}

/** Where to draw each step when the user has not moved it: one column, in running order. */
export function layoutOf(flow: Flow): Map<string, { x: number; y: number }> {
  return new Map(flow.steps.map((s, i) => [s.id, { x: 0, y: i * SEQUENCE_STEP_HEIGHT }]));
}

/** Resolves a connector id such as `out-c1-then` to its step and branch. */
export function parseConnector(
  id: string,
): { direction: "in" | "out"; stepId: string; branch: Branch | null } | null {
  const match = /^(in|out)-(.+?)(?:-(then|else))?$/.exec(id);
  if (!match) return null;
  const [, direction, stepId, branch] = match;
  if (!stepId || (direction !== "in" && direction !== "out")) return null;
  // `in-` connectors never carry a branch; a step id may itself end in `-then`.
  return direction === "in"
    ? { direction, stepId: branch ? `${stepId}-${branch}` : stepId, branch: null }
    : { direction, stepId, branch: (branch as Branch | undefined) ?? null };
}

/** One line under the title of a node. */
export function summarize(step: Step): string {
  switch (step.type) {
    case "input":
      return `${Object.keys(step.inputs ?? {}).length} inputs`;
    case "llm":
      return step.model + (step.tools?.length ? ` · ${step.tools.length} tools` : "");
    case "tool":
      return step.server || step.tool ? `${step.server} / ${step.tool}` : "no tool chosen";
    case "condition":
      return step.expression || "no expression";
    case "transform":
      return `${Object.keys(step.values ?? {}).length} values`;
    case "output":
      return `${Object.keys(step.outputs ?? {}).length} results`;
  }
}
