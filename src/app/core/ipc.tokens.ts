import { InjectionToken } from "@angular/core";

/** Calls a Tauri command by name. */
export type InvokeFn = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

/** Subscribes to a Tauri event; resolves to an unsubscribe function. */
export type ListenFn = <T>(event: string, handler: (payload: T) => void) => Promise<() => void>;

export const IPC_INVOKE = new InjectionToken<InvokeFn>("IPC_INVOKE", {
  providedIn: "root",
  factory:
    () =>
    async <T>(command: string, args?: Record<string, unknown>) => {
      const { invoke } = await import("@tauri-apps/api/core");
      return invoke<T>(command, args);
    },
});

export const IPC_LISTEN = new InjectionToken<ListenFn>("IPC_LISTEN", {
  providedIn: "root",
  factory:
    () =>
    async <T>(event: string, handler: (payload: T) => void) => {
      const { listen } = await import("@tauri-apps/api/event");
      return listen<T>(event, (e) => handler(e.payload));
    },
});
