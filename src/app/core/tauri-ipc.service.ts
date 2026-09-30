import { Injectable, Signal, inject, signal } from "@angular/core";
import type {
  AppInfo,
  Environment,
  EnvironmentInput,
  MessageFilter,
  MessageRecord,
  ServerDefinition,
  ServerInput,
} from "./bindings";
import { IpcError, describeError } from "./ipc-error";
import { IPC_INVOKE, IPC_LISTEN } from "./ipc.tokens";

/** Typed wrapper around Tauri commands and events. Every command has one method here. */
@Injectable({ providedIn: "root" })
export class TauriIpcService {
  private readonly invokeFn = inject(IPC_INVOKE);
  private readonly listenFn = inject(IPC_LISTEN);

  /** Calls a command and converts rejections into {@link IpcError}. */
  async call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
    try {
      return await this.invokeFn<T>(command, args);
    } catch (error) {
      throw new IpcError(command, describeError(error));
    }
  }

  /**
   * Exposes the latest payload of an event as a signal. The subscription lives as long as the
   * app; call the returned `stop` to unsubscribe earlier.
   */
  eventSignal<T>(event: string): { value: Signal<T | undefined>; stop: () => void } {
    const value = signal<T | undefined>(undefined);
    let unlisten: (() => void) | undefined;
    let stopped = false;
    void this.listenFn<T>(event, (payload) => value.set(payload)).then((fn) => {
      if (stopped) fn();
      else unlisten = fn;
    });
    return {
      value: value.asReadonly(),
      stop: () => {
        stopped = true;
        unlisten?.();
      },
    };
  }

  appInfo(): Promise<AppInfo> {
    return this.call<AppInfo>("app_info");
  }

  serverList(): Promise<ServerDefinition[]> {
    return this.call("server_list");
  }

  serverGet(id: string): Promise<ServerDefinition> {
    return this.call("server_get", { id });
  }

  serverAdd(input: ServerInput): Promise<ServerDefinition> {
    return this.call("server_add", { input });
  }

  serverUpdate(id: string, input: ServerInput): Promise<ServerDefinition> {
    return this.call("server_update", { id, input });
  }

  serverRemove(id: string): Promise<void> {
    return this.call("server_remove", { id });
  }

  secretSet(name: string, value: string): Promise<void> {
    return this.call("secret_set", { name, value });
  }

  secretDelete(name: string): Promise<void> {
    return this.call("secret_delete", { name });
  }

  environmentList(): Promise<Environment[]> {
    return this.call("environment_list");
  }

  environmentAdd(input: EnvironmentInput): Promise<Environment> {
    return this.call("environment_add", { input });
  }

  environmentUpdate(id: string, input: EnvironmentInput): Promise<Environment> {
    return this.call("environment_update", { id, input });
  }

  environmentRemove(id: string): Promise<void> {
    return this.call("environment_remove", { id });
  }

  messagesQuery(filter: Partial<MessageFilter> = {}): Promise<MessageRecord[]> {
    return this.call("messages_query", { filter });
  }
}
