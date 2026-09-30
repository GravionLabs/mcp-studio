export type DiffKind = "added" | "removed" | "changed";

export interface DiffEntry {
  /** JSON pointer-like path, e.g. `/params/arguments/message`; empty for the root. */
  path: string;
  kind: DiffKind;
  before?: unknown;
  after?: unknown;
}

export interface DiffOptions {
  /** Top-level keys to skip, e.g. the JSON-RPC `id` which differs between any two requests. */
  ignoreTopLevel?: string[];
}

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/**
 * Structural difference between two JSON values. Objects are compared by key, arrays by index;
 * values of different type or primitives that differ are reported as one `changed` entry.
 */
export function diffJson(before: unknown, after: unknown, options: DiffOptions = {}): DiffEntry[] {
  const out: DiffEntry[] = [];
  walk(before, after, "", out, new Set(options.ignoreTopLevel ?? []), true);
  return out;
}

function walk(
  before: unknown,
  after: unknown,
  path: string,
  out: DiffEntry[],
  ignore: Set<string>,
  top: boolean,
): void {
  if (isObject(before) && isObject(after)) {
    const keys = [...new Set([...Object.keys(before), ...Object.keys(after)])].sort();
    for (const key of keys) {
      if (top && ignore.has(key)) continue;
      const child = `${path}/${escapeKey(key)}`;
      if (!(key in after)) out.push({ path: child, kind: "removed", before: before[key] });
      else if (!(key in before)) out.push({ path: child, kind: "added", after: after[key] });
      else walk(before[key], after[key], child, out, ignore, false);
    }
    return;
  }
  if (Array.isArray(before) && Array.isArray(after)) {
    const length = Math.max(before.length, after.length);
    for (let i = 0; i < length; i++) {
      const child = `${path}/${i}`;
      if (i >= after.length) out.push({ path: child, kind: "removed", before: before[i] });
      else if (i >= before.length) out.push({ path: child, kind: "added", after: after[i] });
      else walk(before[i], after[i], child, out, ignore, false);
    }
    return;
  }
  if (!Object.is(before, after) && JSON.stringify(before) !== JSON.stringify(after)) {
    out.push({ path, kind: "changed", before, after });
  }
}

function escapeKey(key: string): string {
  return key.replace(/~/g, "~0").replace(/\//g, "~1");
}

/** Short one-line rendering of a value for the diff table. */
export function preview(value: unknown, max = 120): string {
  const text =
    value === undefined
      ? ""
      : typeof value === "string"
        ? JSON.stringify(value)
        : JSON.stringify(value);
  return text.length > max ? `${text.slice(0, max - 1)}…` : text;
}

export interface DiffSummary {
  added: number;
  removed: number;
  changed: number;
}

export function summarizeDiff(entries: DiffEntry[]): DiffSummary {
  return {
    added: entries.filter((e) => e.kind === "added").length,
    removed: entries.filter((e) => e.kind === "removed").length,
    changed: entries.filter((e) => e.kind === "changed").length,
  };
}
