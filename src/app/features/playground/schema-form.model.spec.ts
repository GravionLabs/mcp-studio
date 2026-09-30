import { describe, expect, it } from "vitest";
import { JsonSchema, defaultValue, kindOf, prune, resolve, validate } from "./schema-form.model";

const schema: JsonSchema = {
  type: "object",
  required: ["query"],
  properties: {
    query: { type: "string", minLength: 2, description: "Search text" },
    limit: { type: "integer", minimum: 1, maximum: 50, default: 10 },
    mode: { type: "string", enum: ["fast", "deep"] },
    verbose: { type: "boolean" },
    tags: { type: "array", items: { type: "string" } },
    filter: {
      type: "object",
      properties: { owner: { type: "string" } },
    },
  },
};

describe("kindOf", () => {
  it("classifies basic types", () => {
    expect(kindOf({ type: "string" })).toBe("string");
    expect(kindOf({ type: "integer" })).toBe("integer");
    expect(kindOf({ type: "number" })).toBe("number");
    expect(kindOf({ type: "boolean" })).toBe("boolean");
    expect(kindOf({ enum: ["a"] })).toBe("enum");
  });

  it("unwraps nullable types", () => {
    expect(kindOf({ type: ["string", "null"] })).toBe("string");
    expect(kindOf({ anyOf: [{ type: "null" }, { type: "number" }] })).toBe("number");
  });

  it("falls back to raw JSON for what forms cannot express", () => {
    expect(kindOf({})).toBe("json");
    expect(kindOf({ type: "object" })).toBe("json");
    expect(kindOf({ type: "array" })).toBe("json");
    expect(kindOf({ oneOf: [{ type: "string" }, { type: "number" }] })).toBe("json");
    expect(kindOf({ type: ["string", "number"] })).toBe("json");
  });
});

describe("resolve", () => {
  it("follows local refs and keeps sibling keywords", () => {
    const root: JsonSchema = {
      $defs: { name: { type: "string", minLength: 3 } },
      type: "object",
      properties: { n: { $ref: "#/$defs/name", description: "The name" } },
    };
    const resolved = resolve(root.properties?.["n"] as JsonSchema, root);
    expect(resolved).toMatchObject({ type: "string", minLength: 3, description: "The name" });
  });

  it("ignores unknown or remote refs and survives cycles", () => {
    expect(resolve({ $ref: "#/nope" }, {})).toEqual({});
    expect(resolve({ $ref: "http://x/y" }, {})).toEqual({});
    const cyclic: JsonSchema = { $defs: { a: { $ref: "#/$defs/a" } } };
    expect(() => resolve({ $ref: "#/$defs/a" }, cyclic)).not.toThrow();
  });
});

describe("defaultValue", () => {
  it("pre-fills required fields and defaults", () => {
    expect(defaultValue(schema)).toEqual({});
    expect(defaultValue({ ...schema, required: ["limit", "verbose"] })).toEqual({
      limit: 10,
      verbose: false,
    });
  });

  it("uses the first enum option and empty arrays", () => {
    expect(defaultValue({ enum: ["x", "y"] })).toBe("x");
    expect(defaultValue({ type: "array", items: { type: "string" } })).toEqual([]);
  });
});

describe("validate", () => {
  it("accepts a valid value", () => {
    expect(validate(schema, { query: "hello", limit: 5, mode: "fast" })).toEqual([]);
  });

  it("reports missing required fields", () => {
    expect(validate(schema, {})).toEqual(["/query: is required"]);
    expect(validate(schema, { query: "" })).toContain("/query: is required");
  });

  it("checks types, ranges, enums and lengths", () => {
    const errors = validate(schema, { query: "a", limit: 99, mode: "slow", verbose: "yes" });
    expect(errors).toEqual(
      expect.arrayContaining([
        "/query: must be at least 2 characters",
        "/limit: must be ≤ 50",
        '/mode: must be one of "fast", "deep"',
        "/verbose: must be true or false",
      ]),
    );
    expect(validate(schema, { query: "ok", limit: 1.5 })).toContain("/limit: must be an integer");
  });

  it("validates array items and nested objects", () => {
    expect(validate(schema, { query: "ok", tags: ["a", 3] })).toEqual([
      "/tags/1: must be a string",
    ]);
    expect(validate(schema, { query: "ok", filter: { owner: 5 } })).toEqual([
      "/filter/owner: must be a string",
    ]);
  });

  it("treats null as invalid unless the schema allows it", () => {
    expect(validate({ type: "string" }, null)).toEqual(["value: must not be null"]);
    expect(validate({ type: ["string", "null"] }, null)).toEqual([]);
  });

  it("ignores invalid regex patterns in the schema", () => {
    expect(validate({ type: "string", pattern: "(" }, "x")).toEqual([]);
  });
});

describe("prune", () => {
  it("drops empty optional values but keeps required and meaningful ones", () => {
    const value = { query: "q", mode: "", verbose: false, tags: [], filter: {}, limit: 0 };
    expect(prune(schema, value)).toEqual({ query: "q", verbose: false, tags: [], limit: 0 });
  });

  it("keeps required strings even when empty so validation can flag them", () => {
    expect(prune(schema, { query: "" })).toEqual({ query: "" });
  });
});
