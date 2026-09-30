import { describe, expect, it } from "vitest";
import { parseContent, parseToolResult, prettyJson } from "./result.model";

describe("parseContent", () => {
  it("detects JSON inside text content", () => {
    expect(parseContent({ type: "text", text: '{"a":1}' })).toEqual({
      kind: "text",
      text: '{"a":1}',
      json: true,
    });
    expect(parseContent({ type: "text", text: "plain {not json" })).toMatchObject({ json: false });
    expect(parseContent({ type: "text" })).toMatchObject({ text: "", json: false });
  });

  it("builds data URLs for images and audio", () => {
    expect(parseContent({ type: "image", mimeType: "image/png", data: "aGk=" })).toEqual({
      kind: "image",
      src: "data:image/png;base64,aGk=",
      mimeType: "image/png",
    });
    expect(parseContent({ type: "audio", mimeType: "audio/wav", data: "aGk=" })).toMatchObject({
      kind: "audio",
    });
  });

  it("refuses unsafe image data: wrong family, svg, or non-base64 payloads", () => {
    for (const item of [
      { type: "image", mimeType: "text/html", data: "aGk=" },
      { type: "image", mimeType: "image/svg+xml", data: "aGk=" },
      { type: "image", mimeType: "image/png", data: '"><script>' },
      { type: "image", mimeType: "image/png" },
    ]) {
      expect(parseContent(item).kind).toBe("unknown");
    }
  });

  it("reads embedded resources and resource links", () => {
    expect(
      parseContent({
        type: "resource",
        resource: { uri: "file:///a", mimeType: "text/plain", text: "hi" },
      }),
    ).toEqual({
      kind: "resource",
      uri: "file:///a",
      mimeType: "text/plain",
      text: "hi",
      blob: null,
    });
    expect(parseContent({ type: "resource_link", uri: "file:///b", name: "B" })).toEqual({
      kind: "resourceLink",
      uri: "file:///b",
      name: "B",
      description: null,
    });
  });

  it("keeps unknown items as they are", () => {
    expect(parseContent({ type: "hologram" })).toEqual({
      kind: "unknown",
      value: { type: "hologram" },
    });
    expect(parseContent(42)).toEqual({ kind: "unknown", value: 42 });
  });
});

describe("parseToolResult", () => {
  it("returns blocks, structured content, and the error flag", () => {
    const parsed = parseToolResult({
      content: [{ type: "text", text: "5" }],
      structuredContent: { sum: 5 },
      isError: true,
    });
    expect(parsed.blocks).toHaveLength(1);
    expect(parsed.structured).toEqual({ sum: 5 });
    expect(parsed.isError).toBe(true);
  });

  it("tolerates malformed results", () => {
    expect(parseToolResult(null)).toEqual({ blocks: [], structured: undefined, isError: false });
    expect(parseToolResult({ content: "nope" }).blocks).toEqual([]);
  });
});

describe("prettyJson", () => {
  it("formats valid JSON and keeps other text", () => {
    expect(prettyJson('{"a":[1,2]}')).toBe('{\n  "a": [\n    1,\n    2\n  ]\n}');
    expect(prettyJson("not json")).toBe("not json");
  });
});
