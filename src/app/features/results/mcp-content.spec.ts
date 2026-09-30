import { describe, expect, it } from "vitest";
import { missingArguments, promptMessages, resourceBlocks } from "./mcp-content";

describe("resourceBlocks", () => {
  it("maps text and blob contents", () => {
    const blocks = resourceBlocks({
      contents: [
        { uri: "file:///a", mimeType: "text/plain", text: "hello" },
        { uri: "file:///b", blob: "aGk=" },
      ],
    });
    expect(blocks).toEqual([
      { kind: "resource", uri: "file:///a", mimeType: "text/plain", text: "hello", blob: null },
      { kind: "resource", uri: "file:///b", mimeType: null, text: null, blob: "aGk=" },
    ]);
  });

  it("returns nothing for malformed input", () => {
    expect(resourceBlocks(null)).toEqual([]);
    expect(resourceBlocks({ contents: "x" })).toEqual([]);
  });
});

describe("promptMessages", () => {
  it("returns description and role-tagged blocks", () => {
    const view = promptMessages({
      description: "A greeting",
      messages: [{ role: "user", content: { type: "text", text: "Hello, Ada!" } }],
    });
    expect(view.description).toBe("A greeting");
    expect(view.messages).toEqual([
      { role: "user", block: { kind: "text", text: "Hello, Ada!", json: false } },
    ]);
  });

  it("tolerates missing roles and content", () => {
    const view = promptMessages({ messages: [{}, 3] });
    expect(view.messages.map((m) => m.role)).toEqual(["unknown", "unknown"]);
    expect(view.messages[0]?.block.kind).toBe("unknown");
    expect(promptMessages(undefined)).toEqual({ description: null, messages: [] });
  });
});

describe("missingArguments", () => {
  it("lists required arguments without a value", () => {
    const definitions = [
      { name: "a", required: true },
      { name: "b" },
      { name: "c", required: true },
    ];
    expect(missingArguments(definitions, { a: "x", b: "" })).toEqual(["c"]);
    expect(missingArguments(definitions, { a: " ", c: "1" })).toEqual(["a"]);
    expect(missingArguments([], {})).toEqual([]);
  });
});
