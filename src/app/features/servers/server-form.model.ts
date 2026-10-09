import type { ServerInput, TransportKind } from "../../core/bindings";
import type { KeyValueRow } from "../../ui/key-value-editor/key-value-row";
import { SecretWrite, recordToRows, rowsToRecord } from "../../ui/key-value-editor/key-value-rows";

export type { KeyValueRow, SecretWrite };

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
  /** The HTTP server needs OAuth 2.1 sign-in in the browser. */
  oauth: boolean;
  /** Client ID registered by hand (Microsoft Entra ID has no dynamic registration). */
  oauthClientId: string;
  /** Space-separated scopes; empty uses what the server advertises. */
  oauthScopes: string;
  /** Fixed loopback redirect port as text; empty picks a free one. */
  oauthCallbackPort: string;
  /** The HTTP server (Microsoft Entra ID) takes its token from the user's Azure login. */
  azureCredentials: boolean;
}

/** The address of Azure DevOps MCP servers; the organization name is appended. */
export const AZURE_DEVOPS_URL = "https://mcp.dev.azure.com/";

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
    oauth: false,
    oauthClientId: "",
    oauthScopes: "",
    oauthCallbackPort: "",
    azureCredentials: false,
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

/** Converts the form to a server input plus the secret writes needed before saving it. */
export function formToInputWithSecrets(
  form: ServerFormState,
  newSecretName: () => string = () => crypto.randomUUID(),
): { input: ServerInput; writes: SecretWrite[] } {
  const writes: SecretWrite[] = [];
  return { input: buildInput(form, newSecretName, writes), writes };
}

export function formToInput(form: ServerFormState): ServerInput {
  return formToInputWithSecrets(form).input;
}

function buildInput(
  form: ServerFormState,
  newSecretName: () => string,
  writes: SecretWrite[],
): ServerInput {
  const oauth = form.transport === "http" && form.oauth;
  const azure = form.transport === "http" && form.azureCredentials && !oauth;
  return {
    name: form.name.trim(),
    transport: form.transport,
    command: form.command.trim() || null,
    args: parseArgs(form.args),
    env: rowsToRecord(form.env, newSecretName, writes),
    cwd: form.cwd.trim() || null,
    url: form.url.trim() || null,
    headers: rowsToRecord(form.headers, newSecretName, writes),
    tags: form.tags
      .split(",")
      .map((t) => t.trim())
      .filter((t) => t !== ""),
    oauth: form.transport === "http" && form.oauth,
    oauthClientId: oauth ? form.oauthClientId.trim() || null : null,
    oauthScopes: oauth || azure ? form.oauthScopes.trim() || null : null,
    oauthCallbackPort: oauth ? parsePort(form.oauthCallbackPort) : null,
    azureCredentials: azure,
  };
}

export function inputToForm(input: ServerInput): ServerFormState {
  return {
    name: input.name,
    transport: input.transport,
    command: input.command ?? "",
    args: formatArgs(input.args),
    cwd: input.cwd ?? "",
    env: recordToRows(input.env),
    url: input.url ?? "",
    headers: recordToRows(input.headers),
    tags: input.tags.join(", "),
    oauth: input.oauth ?? false,
    oauthClientId: input.oauthClientId ?? "",
    oauthScopes: input.oauthScopes ?? "",
    oauthCallbackPort: input.oauthCallbackPort?.toString() ?? "",
    azureCredentials: input.azureCredentials ?? false,
  };
}

/** A TCP port from text, or `null` when empty or not a valid port. */
export function parsePort(text: string): number | null {
  const value = Number(text.trim());
  return Number.isInteger(value) && value >= 1 && value <= 65535 ? value : null;
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
  if (form.transport === "http" && form.oauth && form.azureCredentials) {
    problems.push("OAuth sign-in and the Azure login cannot be combined.");
  }
  if (form.transport === "http" && form.oauth) {
    const port = form.oauthCallbackPort.trim();
    if (port !== "" && parsePort(port) === null) {
      problems.push("The callback port must be a number between 1 and 65535.");
    }
  }
  return problems;
}
