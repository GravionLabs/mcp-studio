import type { MessageRecord } from "../../core/bindings";

export interface MessageFilterState {
  serverId: string;
  method: string;
  direction: "" | "out" | "in";
  errorsOnly: boolean;
  search: string;
}

export const NO_FILTER: MessageFilterState = {
  serverId: "",
  method: "",
  direction: "",
  errorsOnly: false,
  search: "",
};

/** Whether a message that just arrived belongs in the list for the given filter. */
export function matchesFilter(
  message: MessageRecord,
  filter: MessageFilterState,
  requestMethodOf: (jsonrpcId: string | null, sessionId: string) => string | undefined = () =>
    undefined,
): boolean {
  if (filter.serverId && message.serverId !== filter.serverId) return false;
  if (filter.direction && message.direction !== filter.direction) return false;
  if (filter.errorsOnly && !message.isError) return false;
  if (filter.method) {
    const method = message.method ?? requestMethodOf(message.jsonrpcId, message.sessionId);
    if (method !== filter.method) return false;
  }
  if (filter.search && !message.payload.toLowerCase().includes(filter.search.toLowerCase())) {
    return false;
  }
  return true;
}

/** `HH:MM:SS.mmm` in local time. */
export function formatClock(ts: number): string {
  const d = new Date(ts);
  const p = (n: number, width = 2) => String(n).padStart(width, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

export interface RowLabel {
  /** Short text for the timeline row. */
  text: string;
  kind: "request" | "response" | "notification" | "error";
}

/** Describes a message for the list: method for requests, `result`/`error` for responses. */
export function labelFor(message: MessageRecord, requestMethod?: string): RowLabel {
  let payload: Record<string, unknown> = {};
  try {
    const parsed: unknown = JSON.parse(message.payload);
    if (typeof parsed === "object" && parsed !== null) payload = parsed as Record<string, unknown>;
  } catch {
    // shown as-is below
  }
  if (message.method) {
    const detail =
      typeof payload["params"] === "object" && payload["params"] !== null
        ? nameOf(payload["params"] as Record<string, unknown>)
        : "";
    const text = detail ? `${message.method} · ${detail}` : message.method;
    return { text, kind: message.jsonrpcId === null ? "notification" : "request" };
  }
  const via = requestMethod ? ` · ${requestMethod}` : "";
  return message.isError
    ? { text: `error${via}`, kind: "error" }
    : { text: `result${via}`, kind: "response" };
}

function nameOf(params: Record<string, unknown>): string {
  for (const key of ["name", "uri"]) {
    const value = params[key];
    if (typeof value === "string") return value;
  }
  return "";
}

/** Pretty-printed payload, or the raw text if it is not JSON. */
export function prettyPayload(payload: string): string {
  try {
    return JSON.stringify(JSON.parse(payload), null, 2);
  } catch {
    return payload;
  }
}
