import type { HistoryEntry } from "../../core/bindings";

export interface EntrySummary {
  title: string;
  kind: "tool" | "resource" | "prompt" | "other";
  status: "ok" | "error" | "cancelled";
}

/** Short description of a history entry for the list. */
export function summarize(entry: HistoryEntry): EntrySummary {
  const status = entry.cancelled ? "cancelled" : entry.isError ? "error" : "ok";
  switch (entry.method) {
    case "tools/call":
      return { title: entry.target, kind: "tool", status };
    case "resources/read":
      return { title: entry.target, kind: "resource", status };
    case "prompts/get":
      return { title: entry.target, kind: "prompt", status };
    default:
      return { title: `${entry.method} ${entry.target}`, kind: "other", status };
  }
}

/** Arguments of a `prompts/get` entry as the string map the backend expects. */
export function promptArguments(entry: HistoryEntry): Record<string, string> {
  const raw = entry.arguments;
  if (typeof raw !== "object" || raw === null || Array.isArray(raw)) return {};
  return Object.fromEntries(
    Object.entries(raw as Record<string, unknown>).map(([key, value]) => [key, String(value)]),
  );
}

/** `12 ms`, `1.4 s`, or an empty string when unknown. */
export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined) return "";
  return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} s`;
}

/** Local date and time, `2026-09-30 14:03:12`. */
export function formatTimestamp(ts: number): string {
  const d = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
}
