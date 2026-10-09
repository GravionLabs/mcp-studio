import type { RetentionPolicy, StorageInfo } from "../../core/bindings";

export interface RetentionForm {
  maxAgeDays: string;
  maxMessages: string;
}

const MAX_DAYS = 3650;
const MAX_MESSAGES = 10_000_000;

export function toRetentionForm(policy: RetentionPolicy): RetentionForm {
  return {
    maxAgeDays: String(policy.maxAgeDays ?? 30),
    maxMessages: String(policy.maxMessages ?? 100_000),
  };
}

function wholeNumber(text: string): number | null {
  const trimmed = text.trim();
  return /^\d+$/.test(trimmed) ? Number(trimmed) : null;
}

/** The limits to store, or the reasons why the form cannot be saved. */
export function parseRetention(
  form: RetentionForm,
): { policy: RetentionPolicy; problems: [] } | { policy: null; problems: string[] } {
  const problems: string[] = [];
  const maxAgeDays = wholeNumber(form.maxAgeDays);
  const maxMessages = wholeNumber(form.maxMessages);
  if (maxAgeDays === null || maxAgeDays < 1 || maxAgeDays > MAX_DAYS) {
    problems.push(`Keep history for 1 to ${MAX_DAYS} days.`);
  }
  if (maxMessages === null || maxMessages < 1 || maxMessages > MAX_MESSAGES) {
    problems.push(`Keep 1 to ${MAX_MESSAGES.toLocaleString("en-US")} messages.`);
  }
  if (problems.length > 0 || maxAgeDays === null || maxMessages === null) {
    return { policy: null, problems };
  }
  return { policy: { maxAgeDays, maxMessages }, problems: [] };
}

export function formatBytes(bytes: number | null): string {
  if (bytes === null) return "unknown";
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = -1;
  do {
    value /= 1024;
    unit += 1;
  } while (value >= 1024 && unit < units.length - 1);
  return `${value >= 10 ? Math.round(value) : value.toFixed(1)} ${units[unit]}`;
}

export function describeStorage(info: StorageInfo): string {
  const messages =
    info.messages === 1 ? "1 message" : `${info.messages.toLocaleString("en-US")} messages`;
  const entries =
    info.historyEntries === 1
      ? "1 tool call"
      : `${info.historyEntries.toLocaleString("en-US")} tool calls`;
  return `${formatBytes(info.databaseBytes)} on disk · ${messages} · ${entries} in the history`;
}
