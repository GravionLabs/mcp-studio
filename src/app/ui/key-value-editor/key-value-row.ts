/** One name/value pair. Secret rows keep their value in the OS keyring, never in the database. */
export interface KeyValueRow {
  key: string;
  value: string;
  /** Mask the value and store it in the keyring on save. */
  secret?: boolean;
  /** The existing `keyring:` reference when the secret is already stored. */
  stored?: string;
}
