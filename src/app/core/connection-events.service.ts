import { Injectable, inject } from "@angular/core";
import type { StatusEvent } from "./bindings";
import { ConnectionStatusService } from "./connection-status.service";
import { TauriIpcService } from "./tauri-ipc.service";

/** Feeds `mcp://status` events into {@link ConnectionStatusService}. Started once at app start. */
@Injectable({ providedIn: "root" })
export class ConnectionEventsService {
  private readonly ipc = inject(TauriIpcService);
  private readonly status = inject(ConnectionStatusService);
  private started: Promise<() => void> | null = null;

  start(): Promise<() => void> {
    this.started ??= this.ipc.listen<StatusEvent>("mcp://status", (event) => {
      this.status.update({
        serverId: event.serverId,
        state: event.state,
        message: event.message ?? undefined,
      });
    });
    return this.started;
  }
}
