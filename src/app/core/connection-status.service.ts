import { Injectable, computed, signal } from "@angular/core";

export type ConnectionState = "disconnected" | "connecting" | "connected" | "error";

export interface ServerStatus {
  serverId: string;
  state: ConnectionState;
  message?: string;
}

/** Connection state per server; fed by `mcp://status` events. */
@Injectable({ providedIn: "root" })
export class ConnectionStatusService {
  private readonly _statuses = signal<ReadonlyMap<string, ServerStatus>>(new Map());

  readonly statuses = computed(() => [...this._statuses().values()]);
  readonly connectedCount = computed(
    () => this.statuses().filter((s) => s.state === "connected").length,
  );
  readonly errorCount = computed(() => this.statuses().filter((s) => s.state === "error").length);

  stateOf(serverId: string): ConnectionState {
    return this._statuses().get(serverId)?.state ?? "disconnected";
  }

  update(status: ServerStatus): void {
    this._statuses.update((map) => new Map(map).set(status.serverId, status));
  }

  forget(serverId: string): void {
    this._statuses.update((map) => {
      const next = new Map(map);
      next.delete(serverId);
      return next;
    });
  }
}
