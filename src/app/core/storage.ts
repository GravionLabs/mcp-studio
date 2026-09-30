import { InjectionToken } from "@angular/core";

/** Minimal key-value storage; defaults to `localStorage` and never throws. */
export interface KeyValueStore {
  get(key: string): string | null;
  set(key: string, value: string): void;
}

export const KEY_VALUE_STORE = new InjectionToken<KeyValueStore>("KEY_VALUE_STORE", {
  providedIn: "root",
  factory: () => ({
    get: (key) => {
      try {
        return globalThis.localStorage?.getItem(key) ?? null;
      } catch {
        return null;
      }
    },
    set: (key, value) => {
      try {
        globalThis.localStorage?.setItem(key, value);
      } catch {
        // storage is a convenience only
      }
    },
  }),
});

/** In-memory store for tests. */
export function memoryStore(initial: Record<string, string> = {}): KeyValueStore {
  const data = new Map(Object.entries(initial));
  return { get: (key) => data.get(key) ?? null, set: (key, value) => void data.set(key, value) };
}
