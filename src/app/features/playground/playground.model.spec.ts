import { describe, expect, it } from "vitest";
import { parseRaw, readiness, toArguments, toRawText } from "./playground.model";
import type { JsonSchema } from "./schema-form.model";

const schema: JsonSchema = {
  type: "object",
  required: ["a"],
  properties: { a: { type: "integer" }, note: { type: "string" } },
};

describe("raw text", () => {
  it("prints undefined as an empty object", () => {
    expect(toRawText(undefined)).toBe("{}");
    expect(toRawText({ a: 1 })).toBe('{\n  "a": 1\n}');
  });

  it("parses valid JSON and treats blank as empty arguments", () => {
    expect(parseRaw('{"a":1}')).toEqual({ ok: true, value: { a: 1 } });
    expect(parseRaw("  ")).toEqual({ ok: true, value: {} });
  });

  it("reports parse errors", () => {
    const result = parseRaw("{a:");
    expect(result.ok).toBe(false);
  });
});

describe("toArguments", () => {
  it("drops empty optional values", () => {
    expect(toArguments(schema, { a: 1, note: "" })).toEqual({ a: 1 });
    expect(toArguments(schema, undefined)).toEqual({});
  });
});

describe("readiness", () => {
  it("is ready for valid input", () => {
    expect(readiness(schema, { a: 1 }, null)).toEqual({ ready: true, problems: [] });
  });

  it("lists validation problems", () => {
    expect(readiness(schema, {}, null)).toEqual({ ready: false, problems: ["/a: is required"] });
  });

  it("blocks on JSON syntax errors first", () => {
    const result = readiness(schema, { a: 1 }, "Unexpected token");
    expect(result.ready).toBe(false);
    expect(result.problems[0]).toContain("Invalid JSON");
  });
});
