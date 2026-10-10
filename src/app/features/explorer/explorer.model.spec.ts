import { describe, expect, it } from "vitest";
import { describeParameters, fillTemplate, matches, templateVariables } from "./explorer.model";

describe("describeParameters", () => {
  it("lists parameters with type, required flag, and description", () => {
    const rows = describeParameters({
      type: "object",
      required: ["query"],
      properties: {
        query: { type: "string", description: "What to search" },
        limit: { type: "integer" },
        mode: { enum: ["a", "b"] },
        tags: { type: "array", items: { type: "string" } },
        extra: {},
      },
    });
    expect(rows).toEqual([
      { name: "query", type: "string", required: true, description: "What to search" },
      { name: "limit", type: "integer", required: false, description: "" },
      { name: "mode", type: 'enum ("a", "b")', required: false, description: "" },
      { name: "tags", type: "array of string", required: false, description: "" },
      { name: "extra", type: "json", required: false, description: "" },
    ]);
  });

  it("resolves $ref and handles schemas without properties", () => {
    expect(
      describeParameters({
        $defs: { n: { type: "number" } },
        type: "object",
        properties: { x: { $ref: "#/$defs/n" } },
      }),
    ).toEqual([{ name: "x", type: "number", required: false, description: "" }]);
    expect(describeParameters({ type: "object" })).toEqual([]);
    expect(describeParameters(null)).toEqual([]);
    expect(describeParameters(undefined)).toEqual([]);
  });
});

describe("matches", () => {
  it("matches any field case-insensitively and accepts an empty query", () => {
    expect(matches("", "x")).toBe(true);
    expect(matches("ECH", "echo", null)).toBe(true);
    expect(matches("zzz", "echo", "Echo tool")).toBe(false);
    expect(matches("tool", undefined, "Echo tool")).toBe(true);
  });
});

describe("templateVariables", () => {
  it("lists plain variables once, in order", () => {
    expect(templateVariables("test://users/{name}/posts/{id}?x={name}")).toEqual(["name", "id"]);
  });

  it("gives up on operators and on templates without variables", () => {
    expect(templateVariables("file:///{+path}")).toBeNull();
    expect(templateVariables("test://x{?a,b}")).toBeNull();
    expect(templateVariables("test://fixed")).toBeNull();
  });
});

describe("fillTemplate", () => {
  it("fills and encodes the variables", () => {
    expect(fillTemplate("test://users/{name}", { name: "a b/c" })).toBe("test://users/a%20b%2Fc");
  });

  it("leaves unfilled variables empty", () => {
    expect(fillTemplate("test://{a}/{b}", { a: "x" })).toBe("test://x/");
  });
});
