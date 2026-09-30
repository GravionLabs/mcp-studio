import type { ServerInput, TransportKind } from "../../core/bindings";

/** Editable form state; text areas hold raw strings until submit. */
export interface ServerFormState {
  name: string;
  transport: TransportKind;
  command: string;
  args: string;
  cwd: string;
  env: KeyValueRow[];
  url: string;
  headers: KeyValueRow[];
  tags: string;
}

export interface KeyValueRow {
  key: string;
  value: string;
}

export function emptyForm(transport: TransportKind = "stdio"): ServerFormState {
  return {
    name: "",
    transport,
    command: "",
    args: "",
    cwd: "",
    env: [],
    url: "",
    headers: [],
    tags: "",
  };
}

/**
 * Splits a command line into arguments, honoring single quotes, double quotes, and backslash escapes.
 * Unterminated quotes take the rest of the line.
 */
export function parseArgs(line: string): string[] {
  const args: string[] = [];
  let current = "";
  let inToken = false;
  let quote: '"' | "'" | null = null;
  for (let i = 0; i < line.length; i++) {
    const char = line[i] as string;
    if (quote) {
      if (char === quote) quote = null;
      else if (char === "\\" && quote === '"' && i + 1 < line.length) current += line[++i];
      else current += char;
    } else if (char === '"' || char === "'") {
      quote = char;
      inToken = true;
    } else if (char === "\\" && i + 1 < line.length) {
      current += line[++i];
      inToken = true;
    } else if (/\s/.test(char)) {
      if (inToken) args.push(current);
      current = "";
      inToken = false;
    } else {
      current += char;
      inToken = true;
    }
  }
  if (inToken) args.push(current);
  return args;
}

/** Inverse of {@link parseArgs}: quotes arguments that need it. */
export function formatArgs(args: string[]): string {
  return args
    .map((arg) =>
      arg === "" || /[\s"'\\]/.test(arg) ? `"${arg.replace(/(["\\])/g, "\\$1")}"` : arg,
    )
    .join(" ");
}

function toRecord(rows: KeyValueRow[]): Record<string, string> {
  const record: Record<string, string> = {};
  for (const { key, value } of rows) {
    if (key.trim() !== "") record[key.trim()] = value;
  }
  return record;
}

function toRows(record: Record<string, string>): KeyValueRow[] {
  return Object.entries(record).map(([key, value]) => ({ key, value }));
}

export function formToInput(form: ServerFormState): ServerInput {
  return {
    name: form.name.trim(),
    transport: form.transport,
    command: form.command.trim() || null,
    args: parseArgs(form.args),
    env: toRecord(form.env),
    cwd: form.cwd.trim() || null,
    url: form.url.trim() || null,
    headers: toRecord(form.headers),
    tags: form.tags
      .split(",")
      .map((t) => t.trim())
      .filter((t) => t !== ""),
  };
}

export function inputToForm(input: ServerInput): ServerFormState {
  return {
    name: input.name,
    transport: input.transport,
    command: input.command ?? "",
    args: formatArgs(input.args),
    cwd: input.cwd ?? "",
    env: toRows(input.env),
    url: input.url ?? "",
    headers: toRows(input.headers),
    tags: input.tags.join(", "),
  };
}

/** Returns human-readable problems; an empty list means the form can be submitted. */
export function validateForm(form: ServerFormState): string[] {
  const problems: string[] = [];
  if (form.name.trim() === "") problems.push("Name is required.");
  if (form.transport === "stdio") {
    if (form.command.trim() === "") problems.push("Command is required for stdio servers.");
  } else {
    const url = form.url.trim();
    if (url === "") problems.push("URL is required for HTTP servers.");
    else if (!/^https?:\/\//i.test(url) || !URL.canParse(url)) {
      problems.push("URL must be a valid http:// or https:// address.");
    }
  }
  return problems;
}
