import { describe, expect, it } from "vitest";
import type { TestSuite } from "../../core/bindings";
import { describeExpectation, draftOf, newCase, newSuite, problems, toInput } from "./suites.model";

const suite: TestSuite = {
  id: "s1",
  serverId: "srv",
  name: "Smoke",
  systemPrompt: "Be brief.",
  updatedAt: 1,
  cases: [
    {
      id: "c1",
      input: "Open issues?",
      expectation: { kind: "tool", name: "list_issues" },
      notes: null,
    },
    { id: "c2", input: "Hello", expectation: { kind: "noTool" }, notes: "greeting" },
    {
      id: "c3",
      input: "Which repo?",
      expectation: { kind: "answer", contains: "a/b" },
      notes: null,
    },
  ],
};

describe("draftOf and toInput", () => {
  it("round-trips a suite through the form", () => {
    const draft = draftOf(suite);
    expect(draft.cases.map((c) => [c.kind, c.value])).toEqual([
      ["tool", "list_issues"],
      ["noTool", ""],
      ["answer", "a/b"],
    ]);
    expect(toInput("srv", draft)).toEqual({
      serverId: "srv",
      name: "Smoke",
      systemPrompt: "Be brief.",
      cases: [
        {
          id: "c1",
          input: "Open issues?",
          expectation: { kind: "tool", name: "list_issues" },
          notes: null,
        },
        { id: "c2", input: "Hello", expectation: { kind: "noTool" }, notes: "greeting" },
        {
          id: "c3",
          input: "Which repo?",
          expectation: { kind: "answer", contains: "a/b" },
          notes: null,
        },
      ],
    });
  });

  it("trims text and turns blank optional fields into null", () => {
    const draft = newSuite();
    draft.name = "  S  ";
    draft.systemPrompt = "   ";
    const first = draft.cases[0]!;
    first.input = "  Hi  ";
    first.value = " tool ";
    first.notes = " ";
    expect(toInput("srv", draft)).toMatchObject({
      name: "S",
      systemPrompt: null,
      cases: [{ id: null, input: "Hi", expectation: { kind: "tool", name: "tool" }, notes: null }],
    });
  });

  it("gives every row its own key", () => {
    const keys = draftOf(suite).cases.map((c) => c.key);
    expect(new Set(keys).size).toBe(3);
    expect(newCase().key).not.toBe(newCase().key);
  });
});

describe("problems", () => {
  it("accepts a complete suite and a suite without cases", () => {
    expect(problems(draftOf(suite))).toEqual([]);
    expect(problems({ id: null, name: "x", systemPrompt: "", cases: [] })).toEqual([]);
  });

  it("lists what is missing, by case", () => {
    const draft = newSuite();
    draft.cases.push(newCase("answer"), newCase("noTool"));
    draft.cases[2]!.input = "x";
    expect(problems(draft)).toEqual([
      "The suite needs a name.",
      "Case 1 has no input.",
      "Case 1 names no tool.",
      "Case 2 has no input.",
      "Case 2 says nothing about the answer.",
    ]);
  });
});

describe("describeExpectation", () => {
  it("says it in words", () => {
    expect(describeExpectation({ kind: "tool", name: "t" })).toBe("calls t");
    expect(describeExpectation({ kind: "noTool" })).toBe("calls no tool");
    expect(describeExpectation({ kind: "answer", contains: "a/b" })).toBe("answers with “a/b”");
  });
});
