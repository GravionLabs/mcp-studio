import { describe, expect, it } from "vitest";
import type { ConfirmRequest, Flow, RunEvent } from "../../core/bindings";
import {
  applyEvent,
  describeArguments,
  describeCall,
  formatDuration,
  inputFields,
  parseInputs,
  startView,
} from "./run.model";

const flow: Flow = {
  version: 1,
  name: "f",
  steps: [
    {
      id: "inputs",
      type: "input",
      inputs: {
        repo: { type: "string", description: "owner/name" },
        limit: { type: "integer" },
        ratio: { type: "number" },
        debug: { type: "boolean" },
        labels: { type: "array" },
        filter: { type: "object" },
      },
    },
    { id: "o", type: "output", outputs: {} },
  ],
};

describe("inputFields", () => {
  it("lists the declared inputs alphabetically with their types", () => {
    const fields = inputFields(flow);
    expect(fields.map((f) => f.name)).toEqual([
      "debug",
      "filter",
      "labels",
      "limit",
      "ratio",
      "repo",
    ]);
    expect(fields.find((f) => f.name === "repo")).toEqual({
      name: "repo",
      kind: "string",
      description: "owner/name",
    });
  });

  it("is empty for a flow without an input step", () => {
    expect(inputFields({ version: 1, name: "x", steps: [] })).toEqual([]);
  });
});

describe("parseInputs", () => {
  const fields = inputFields(flow);

  it("converts what was typed to the declared types", () => {
    const { values, errors } = parseInputs(fields, {
      repo: "a/b",
      limit: " 5 ",
      ratio: "0.5",
      debug: true,
      labels: '["x","y"]',
      filter: '{"state":"open"}',
    });
    expect(errors).toEqual({});
    expect(values).toEqual({
      repo: "a/b",
      limit: 5,
      ratio: 0.5,
      debug: true,
      labels: ["x", "y"],
      filter: { state: "open" },
    });
  });

  it("treats a missing checkbox as false", () => {
    expect(parseInputs([{ name: "debug", kind: "boolean", description: null }], {}).values).toEqual(
      {
        debug: false,
      },
    );
  });

  it("reports what is missing or wrong, per input", () => {
    const { values, errors } = parseInputs(fields, {
      repo: "",
      limit: "1.5",
      ratio: "abc",
      labels: '{"a":1}',
      filter: "[1]",
    });
    expect(values).toEqual({ debug: false });
    expect(errors).toEqual({
      repo: "Enter a value",
      limit: "Enter a whole number",
      ratio: "Enter a number",
      labels: "Enter a JSON array",
      filter: "Enter a JSON object",
    });
  });

  it("rejects invalid JSON and empty numbers", () => {
    const { errors } = parseInputs(fields, { limit: "", labels: "[1,", filter: "null" });
    expect(errors["limit"]).toBe("Enter a number");
    expect(errors["labels"]).toBe("Enter a JSON array");
    expect(errors["filter"]).toBe("Enter a JSON object");
  });
});

describe("applyEvent", () => {
  const run = (events: RunEvent[]) => events.reduce(applyEvent, startView("r1", "f"));

  it("follows a run from start to finish", () => {
    const view = run([
      { type: "run_started", runId: "r1", flowName: "My flow" },
      { type: "step_started", runId: "r1", stepId: "ask", kind: "llm" },
      { type: "text_delta", runId: "r1", stepId: "ask", text: "Hel" },
      { type: "text_delta", runId: "r1", stepId: "ask", text: "lo" },
      { type: "step_finished", runId: "r1", stepId: "ask", status: "succeeded", error: null },
      { type: "run_finished", runId: "r1", status: "succeeded", error: null },
    ]);
    expect(view.flowName).toBe("My flow");
    expect(view.status).toBe("succeeded");
    expect(view.steps).toEqual([
      { stepId: "ask", kind: "llm", status: "succeeded", error: null, text: "Hello" },
    ]);
  });

  it("shows failures", () => {
    const view = run([
      { type: "step_started", runId: "r1", stepId: "t", kind: "tool" },
      { type: "step_finished", runId: "r1", stepId: "t", status: "failed", error: "boom" },
      { type: "run_finished", runId: "r1", status: "failed", error: "step `t` failed: boom" },
    ]);
    expect(view.steps[0]).toMatchObject({ status: "failed", error: "boom" });
    expect(view.status).toBe("failed");
    expect(view.error).toContain("boom");
  });

  it("lists steps that were jumped over without starting them", () => {
    const view = run([
      { type: "step_finished", runId: "r1", stepId: "small", status: "skipped", error: null },
    ]);
    expect(view.steps).toEqual([
      { stepId: "small", kind: "", status: "skipped", error: null, text: "" },
    ]);
  });

  it("ignores events of other runs", () => {
    const view = run([
      { type: "step_started", runId: "other", stepId: "x", kind: "tool" },
      { type: "run_finished", runId: "other", status: "failed", error: "no" },
    ]);
    expect(view).toEqual(startView("r1", "f"));
  });
});

describe("formatting", () => {
  it("formats durations", () => {
    expect(formatDuration(1000, 1250)).toBe("250 ms");
    expect(formatDuration(0, 1500)).toBe("1.50 s");
    expect(formatDuration(0, 12_000)).toBe("12.0 s");
    expect(formatDuration(0, null)).toBe("running");
  });

  it("describes a question", () => {
    const request: ConfirmRequest = {
      runId: "r",
      stepId: "s",
      server: "github",
      tool: "list_issues",
      arguments: { repo: "a/b" },
    };
    expect(describeCall(request)).toBe("github / list_issues");
    expect(describeArguments(request)).toBe('{\n  "repo": "a/b"\n}');
  });
});
