import { describe, expect, it } from "vitest";
import type { ClientRequest } from "../../core/bindings";
import {
  dequeue,
  elicitationProblems,
  elicitationView,
  enqueue,
  samplingBlocker,
  samplingView,
} from "./client-requests.model";

const request = (id: string): ClientRequest => ({
  id,
  serverId: "s1",
  kind: "sampling",
  params: {},
  ts: 1,
});

describe("samplingView", () => {
  it("shows messages, system prompt, limits and preferences", () => {
    const view = samplingView({
      messages: [
        { role: "user", content: { type: "text", text: "Hi" } },
        {
          role: "assistant",
          content: [
            { type: "text", text: "Hello" },
            { type: "image", data: "AAAA" },
          ],
        },
      ],
      systemPrompt: "Be brief",
      maxTokens: 100,
      temperature: 0.2,
      modelPreferences: { hints: [{ name: "claude" }], speedPriority: 0.8 },
    });
    expect(view.messages).toEqual([
      { role: "user", text: "Hi" },
      { role: "assistant", text: "Hello\n[image content]" },
    ]);
    expect(view.system).toBe("Be brief");
    expect(view.maxTokens).toBe(100);
    expect(view.temperature).toBe(0.2);
    expect(view.preferences).toBe("hints: claude · speed 0.8");
  });

  it("copes with missing parts", () => {
    expect(samplingView(undefined)).toEqual({
      system: null,
      messages: [],
      preferences: null,
      maxTokens: null,
      temperature: null,
    });
  });
});

describe("elicitationView", () => {
  it("returns the message and an object schema", () => {
    const view = elicitationView({
      mode: "form",
      message: "Name?",
      requestedSchema: { properties: { name: { type: "string" } }, required: ["name"] },
    });
    expect(view.message).toBe("Name?");
    expect(view.schema.type).toBe("object");
    expect(view.schema.required).toEqual(["name"]);
  });
});

describe("elicitationProblems", () => {
  const schema = {
    type: "object",
    properties: { name: { type: "string" } },
    required: ["name"],
  };

  it("reports a missing required field", () => {
    expect(elicitationProblems(schema, {})).not.toEqual([]);
  });

  it("accepts a complete form", () => {
    expect(elicitationProblems(schema, { name: "Ada" })).toEqual([]);
  });
});

describe("samplingBlocker", () => {
  it("asks for an answer", () => {
    expect(samplingBlocker("  ")).not.toBeNull();
    expect(samplingBlocker("ok")).toBeNull();
  });
});

describe("queue", () => {
  it("does not queue a request twice and removes answered ones", () => {
    const queue = enqueue(enqueue([], request("a")), request("a"));
    expect(queue).toHaveLength(1);
    expect(dequeue(enqueue(queue, request("b")), "a").map((q) => q.id)).toEqual(["b"]);
  });
});
