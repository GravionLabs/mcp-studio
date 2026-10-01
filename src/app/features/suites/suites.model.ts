import type { Expectation, TestSuite, TestSuiteInput } from "../../core/bindings";

export type ExpectationKind = "tool" | "noTool" | "answer";

export interface CaseDraft {
  /** Identifies the row in the form. */
  key: number;
  /** The id of a saved case; new cases have none. */
  id: string | null;
  input: string;
  kind: ExpectationKind;
  /** The tool name or the text the answer should contain; not used for `noTool`. */
  value: string;
  notes: string;
}

export interface SuiteDraft {
  id: string | null;
  name: string;
  systemPrompt: string;
  cases: CaseDraft[];
}

let nextKey = 1;

export function newCase(kind: ExpectationKind = "tool"): CaseDraft {
  return { key: nextKey++, id: null, input: "", kind, value: "", notes: "" };
}

export function newSuite(): SuiteDraft {
  return { id: null, name: "", systemPrompt: "", cases: [newCase()] };
}

function kindOf(expectation: Expectation): { kind: ExpectationKind; value: string } {
  switch (expectation.kind) {
    case "tool":
      return { kind: "tool", value: expectation.name };
    case "answer":
      return { kind: "answer", value: expectation.contains };
    case "noTool":
      return { kind: "noTool", value: "" };
  }
}

export function draftOf(suite: TestSuite): SuiteDraft {
  return {
    id: suite.id,
    name: suite.name,
    systemPrompt: suite.systemPrompt ?? "",
    cases: suite.cases.map((c) => ({
      key: nextKey++,
      id: c.id,
      input: c.input,
      ...kindOf(c.expectation),
      notes: c.notes ?? "",
    })),
  };
}

function expectationOf(draft: CaseDraft): Expectation {
  switch (draft.kind) {
    case "tool":
      return { kind: "tool", name: draft.value.trim() };
    case "answer":
      return { kind: "answer", contains: draft.value.trim() };
    case "noTool":
      return { kind: "noTool" };
  }
}

export function toInput(serverId: string, draft: SuiteDraft): TestSuiteInput {
  const text = (value: string) => (value.trim() === "" ? null : value.trim());
  return {
    serverId,
    name: draft.name.trim(),
    systemPrompt: text(draft.systemPrompt),
    cases: draft.cases.map((c) => ({
      id: c.id,
      input: c.input.trim(),
      expectation: expectationOf(c),
      notes: text(c.notes),
    })),
  };
}

/** What stops a suite from being saved; the same rules as the backend, to tell the user early. */
export function problems(draft: SuiteDraft): string[] {
  const found: string[] = [];
  if (draft.name.trim() === "") found.push("The suite needs a name.");
  draft.cases.forEach((c, index) => {
    const number = index + 1;
    if (c.input.trim() === "") found.push(`Case ${number} has no input.`);
    if (c.kind === "tool" && c.value.trim() === "") found.push(`Case ${number} names no tool.`);
    if (c.kind === "answer" && c.value.trim() === "") {
      found.push(`Case ${number} says nothing about the answer.`);
    }
  });
  return found;
}

export function describeExpectation(expectation: Expectation): string {
  switch (expectation.kind) {
    case "tool":
      return `calls ${expectation.name}`;
    case "noTool":
      return "calls no tool";
    case "answer":
      return `answers with “${expectation.contains}”`;
  }
}
