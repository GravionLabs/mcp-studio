import { JsonSchema, kindOf, resolve } from "../playground/schema-form.model";

export interface ParameterRow {
  name: string;
  type: string;
  required: boolean;
  description: string;
}

/** Flattens a tool's `inputSchema` into a table of top-level parameters. */
export function describeParameters(schema: unknown): ParameterRow[] {
  if (typeof schema !== "object" || schema === null) return [];
  const root = schema as JsonSchema;
  const resolved = resolve(root, root);
  const required = new Set(resolved.required ?? []);
  return Object.entries(resolved.properties ?? {}).map(([name, property]) => {
    const p = resolve(property, root);
    const kind = kindOf(property, root);
    let type: string = kind;
    if (kind === "enum") type = `enum (${(p.enum ?? []).map((v) => JSON.stringify(v)).join(", ")})`;
    else if (kind === "array") type = `array of ${p.items ? kindOf(p.items, root) : "any"}`;
    else if (kind === "json") type = "json";
    return { name, type, required: required.has(name), description: p.description ?? "" };
  });
}

/** Case-insensitive filter over the given text fields. */
export function matches(query: string, ...fields: (string | null | undefined)[]): boolean {
  const q = query.trim().toLowerCase();
  return q === "" || fields.some((f) => f?.toLowerCase().includes(q));
}
