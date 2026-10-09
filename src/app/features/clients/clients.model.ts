import type { ClientEntry } from "../../core/bindings";

/** The entries of one configuration file. */
export interface ClientGroup {
  client: string;
  path: string;
  entries: ClientEntry[];
}

/** Entries grouped by file, in the order the files were found; the top level first, then projects, by name. */
export function groupEntries(entries: readonly ClientEntry[]): ClientGroup[] {
  const groups = new Map<string, ClientGroup>();
  for (const entry of entries) {
    const key = `${entry.client}\n${entry.path}`;
    let group = groups.get(key);
    if (!group) {
      group = { client: entry.client, path: entry.path, entries: [] };
      groups.set(key, group);
    }
    group.entries.push(entry);
  }
  const scopeRank = (origin: string) => (origin === "top level" ? 0 : 1);
  for (const group of groups.values()) {
    group.entries.sort(
      (a, b) =>
        scopeRank(a.origin) - scopeRank(b.origin) ||
        a.origin.localeCompare(b.origin) ||
        a.name.localeCompare(b.name),
    );
  }
  return [...groups.values()];
}

export type EntryStatus = "direct" | "routed" | "routed-by-hand" | "unsupported";

export function entryStatus(entry: ClientEntry): EntryStatus {
  if (entry.kind === "unsupported") return "unsupported";
  if (!entry.routed) return "direct";
  return entry.routeId ? "routed" : "routed-by-hand";
}

export function statusLabel(entry: ClientEntry): string {
  switch (entryStatus(entry)) {
    case "direct":
      return "Direct";
    case "routed":
      return "Through MCP Studio";
    case "routed-by-hand":
      return "Through MCP Studio (set up by hand)";
    case "unsupported":
      return entry.unsupported ?? "Not supported";
  }
}

export const canRoute = (entry: ClientEntry): boolean => entryStatus(entry) === "direct";

export const canRestore = (entry: ClientEntry): boolean => entryStatus(entry) === "routed";

/** The text of the question when an entry was changed after it was routed. */
export function forceQuestion(conflict: string): string {
  return `${conflict} Restore the original entry anyway? This replaces what is in the file now.`;
}
