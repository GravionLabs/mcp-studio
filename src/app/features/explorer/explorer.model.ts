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

/**
 * The variables of a resource template that only uses plain `{name}` expressions, in order of first
 * appearance. `null` when it has none or uses operators (`{+path}`, `{?query}`, `{list*}`) that the
 * per-variable form cannot fill in; those are edited as text.
 */
export function templateVariables(template: string): string[] | null {
  const names: string[] = [];
  for (const match of template.matchAll(/\{([^}]*)\}/g)) {
    const name = match[1] ?? "";
    if (!/^[A-Za-z0-9_.]+$/.test(name)) return null;
    if (!names.includes(name)) names.push(name);
  }
  return names.length > 0 ? names : null;
}

/** The URI of a template with its variables filled in (percent-encoded). */
export function fillTemplate(template: string, values: Record<string, string>): string {
  return template.replace(/\{([^}]*)\}/g, (_, name: string) =>
    encodeURIComponent(values[name] ?? ""),
  );
}
