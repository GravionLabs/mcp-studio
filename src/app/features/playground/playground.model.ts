import { JsonSchema, prune, validate } from "./schema-form.model";

/** Pretty JSON for the raw editor; `undefined` becomes an empty object. */
export function toRawText(value: unknown): string {
  return JSON.stringify(value ?? {}, null, 2);
}

export type ParsedRaw = { ok: true; value: unknown } | { ok: false; error: string };

/** Parses the raw editor text; an empty editor means "no arguments". */
export function parseRaw(text: string): ParsedRaw {
  if (text.trim() === "") return { ok: true, value: {} };
  try {
    return { ok: true, value: JSON.parse(text) };
  } catch (error) {
    return { ok: false, error: error instanceof Error ? error.message : String(error) };
  }
}

/** Arguments as they are sent: empty optional values removed. */
export function toArguments(schema: JsonSchema, value: unknown): unknown {
  const pruned = prune(schema, value ?? {}, schema);
  return pruned ?? {};
}

export interface Readiness {
  ready: boolean;
  problems: string[];
}

/** Whether the current input may be sent, with the reasons if not. */
export function readiness(schema: JsonSchema, value: unknown, rawError: string | null): Readiness {
  const problems = rawError ? [`Invalid JSON: ${rawError}`] : validate(schema, value ?? {}, schema);
  return { ready: problems.length === 0, problems };
}
