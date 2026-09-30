import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import type { StatusEvent } from "./bindings";
import { ConnectionEventsService } from "./connection-events.service";
import { ConnectionStatusService } from "./connection-status.service";
import { TauriIpcService } from "./tauri-ipc.service";

describe("ConnectionEventsService", () => {
  it("maps status events onto the status service and subscribes only once", async () => {
    let handler: ((e: StatusEvent) => void) | undefined;
    let subscriptions = 0;
    const ipc = {
      listen: async (_: string, h: (e: StatusEvent) => void) => {
        subscriptions++;
        handler = h;
        return () => {};
      },
    };
    const status = new ConnectionStatusService();
    const injector = Injector.create({
      providers: [
        { provide: TauriIpcService, useValue: ipc },
        { provide: ConnectionStatusService, useValue: status },
      ],
    });
    const service = runInInjectionContext(injector, () => new ConnectionEventsService());
    await service.start();
    await service.start();
    expect(subscriptions).toBe(1);

    handler?.({ serverId: "a", sessionId: "s", state: "connected", message: null });
    handler?.({ serverId: "b", sessionId: null, state: "error", message: "boom" });
    expect(status.stateOf("a")).toBe("connected");
    expect(status.statuses().find((s) => s.serverId === "b")).toMatchObject({
      state: "error",
      message: "boom",
    });
  });
});
