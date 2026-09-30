import type { KeyValueRow } from "./key-value-row";

export const REFERENCE_PREFIX = "keyring:";

/** A secret value that must be written to the keyring before its owner is saved. */
export interface SecretWrite {
  /** Name inside the reference, i.e. the part after `keyring:`. */
  name: string;
  value: string;
}

/**
 * Converts editor rows to a name/value record. Secret rows with a new value are appended to `writes`
 * and stored as `keyring:` references; secret rows left empty keep their existing reference.
 */
export function rowsToRecord(
  rows: KeyValueRow[],
  newSecretName: () => string,
  writes: SecretWrite[],
): Record<string, string> {
  const record: Record<string, string> = {};
  for (const row of rows) {
    const key = row.key.trim();
    if (key === "") continue;
    if (!row.secret) {
      record[key] = row.value;
    } else if (row.value !== "") {
      const name = row.stored ? row.stored.slice(REFERENCE_PREFIX.length) : newSecretName();
      writes.push({ name, value: row.value });
      record[key] = REFERENCE_PREFIX + name;
    } else if (row.stored) {
      record[key] = row.stored;
    }
  }
  return record;
}

/** Inverse of {@link rowsToRecord}: references become masked secret rows. */
export function recordToRows(record: Record<string, string>): KeyValueRow[] {
  return Object.entries(record).map(([key, value]) =>
    value.startsWith(REFERENCE_PREFIX)
      ? { key, value: "", secret: true, stored: value }
      : { key, value },
  );
}
