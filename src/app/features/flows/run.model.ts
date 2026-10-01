import type {
  ConfirmRequest,
  Flow,
  FlowRun,
  RunEvent,
  RunStatus,
  StepStatus,
} from "../../core/bindings";

/** A declared input of a flow, as a field of the run form. */
export interface InputField {
  name: string;
  kind: string;
  description: string | null;
}

/** The inputs a flow declares in its `input` step, in alphabetical order. */
export function inputFields(flow: Flow): InputField[] {
  const fields: InputField[] = [];
  for (const step of flow.steps) {
    if (step.type !== "input") continue;
    for (const [name, decl] of Object.entries(step.inputs ?? {})) {
      fields.push({ name, kind: decl.type, description: decl.description ?? null });
    }
  }
  return fields.sort((a, b) => a.name.localeCompare(b.name));
}

export interface ParsedInputs {
  values: Record<string, unknown>;
  /** Problems by input name; empty when the form is fine. */
  errors: Record<string, string>;
}

/** Turns what was typed into the form into values of the declared types. */
export function parseInputs(
  fields: readonly InputField[],
  raw: Readonly<Record<string, string | boolean>>,
): ParsedInputs {
  const values: Record<string, unknown> = {};
  const errors: Record<string, string> = {};
  for (const field of fields) {
    const entry = raw[field.name];
    switch (field.kind) {
      case "boolean":
        values[field.name] = entry === true || entry === "true";
        break;
      case "number":
      case "integer": {
        const text = typeof entry === "string" ? entry.trim() : "";
        const number = Number(text);
        if (text === "" || !Number.isFinite(number)) errors[field.name] = "Enter a number";
        else if (field.kind === "integer" && !Number.isInteger(number))
          errors[field.name] = "Enter a whole number";
        else values[field.name] = number;
        break;
      }
      case "array":
      case "object": {
        const text = typeof entry === "string" ? entry.trim() : "";
        try {
          const parsed: unknown = JSON.parse(text);
          const isArray = Array.isArray(parsed);
          const ok =
            field.kind === "array"
              ? isArray
              : typeof parsed === "object" && parsed !== null && !isArray;
          if (ok) values[field.name] = parsed;
          else errors[field.name] = `Enter a JSON ${field.kind}`;
        } catch {
          errors[field.name] = `Enter a JSON ${field.kind}`;
        }
        break;
      }
      default: {
        const text = typeof entry === "string" ? entry : "";
        if (text === "") errors[field.name] = "Enter a value";
        else values[field.name] = text;
      }
    }
  }
  return { values, errors };
}

export interface StepView {
  stepId: string;
  kind: string;
  status: StepStatus;
  error: string | null;
  /** What an LLM step has written so far. */
  text: string;
}

/** What the run panel shows; built from events while a run is going and from a stored run later. */
export interface RunView {
  runId: string;
  flowName: string;
  status: RunStatus;
  error: string | null;
  steps: StepView[];
}

export function startView(runId: string, flowName: string): RunView {
  return { runId, flowName, status: "running", error: null, steps: [] };
}

/** Applies one progress event. Events of other runs are ignored. */
export function applyEvent(view: RunView, event: RunEvent): RunView {
  if (event.runId !== view.runId) return view;
  switch (event.type) {
    case "run_started":
      return { ...view, flowName: event.flowName };
    case "step_started":
      return {
        ...view,
        steps: [
          ...view.steps,
          { stepId: event.stepId, kind: event.kind, status: "running", error: null, text: "" },
        ],
      };
    case "step_finished": {
      const exists = view.steps.some((s) => s.stepId === event.stepId);
      const finished = (s: StepView): StepView => ({
        ...s,
        status: event.status,
        error: event.error,
      });
      return {
        ...view,
        steps: exists
          ? view.steps.map((s) => (s.stepId === event.stepId ? finished(s) : s))
          : // A step that was jumped over never started.
            [
              ...view.steps,
              finished({
                stepId: event.stepId,
                kind: "",
                status: "running",
                error: null,
                text: "",
              }),
            ],
      };
    }
    case "text_delta":
      return {
        ...view,
        steps: view.steps.map((s) =>
          s.stepId === event.stepId ? { ...s, text: s.text + event.text } : s,
        ),
      };
    case "run_finished":
      return { ...view, status: event.status, error: event.error };
  }
}

/** The view of a stored run. */
export function viewOf(run: FlowRun): RunView {
  return {
    runId: run.id,
    flowName: run.flow.name,
    status: run.status,
    error: run.error,
    steps: run.steps.map((s) => ({
      stepId: s.stepId,
      kind: s.kind,
      status: s.status,
      error: s.error,
      text: "",
    })),
  };
}

export function formatDuration(startedAt: number, endedAt: number | null): string {
  if (endedAt === null) return "running";
  const ms = endedAt - startedAt;
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(ms < 10_000 ? 2 : 1)} s`;
}

/** The tool of a question as `server / tool`. */
export function describeCall(request: ConfirmRequest): string {
  return `${request.server} / ${request.tool}`;
}

/** The arguments of a question as indented JSON. */
export function describeArguments(request: ConfirmRequest): string {
  return JSON.stringify(request.arguments, null, 2);
}
