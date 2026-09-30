import { describe, expect, it } from "vitest";
import { ConnectionStatusService } from "./connection-status.service";

describe("ConnectionStatusService", () => {
  it("reports unknown servers as disconnected", () => {
    expect(new ConnectionStatusService().stateOf("x")).toBe("disconnected");
  });

  it("tracks state changes and counts", () => {
    const service = new ConnectionStatusService();
    service.update({ serverId: "a", state: "connecting" });
    service.update({ serverId: "b", state: "connected" });
    service.update({ serverId: "a", state: "connected" });
    service.update({ serverId: "c", state: "error", message: "spawn failed" });
    expect(service.connectedCount()).toBe(2);
    expect(service.errorCount()).toBe(1);
    expect(service.stateOf("c")).toBe("error");
  });

  it("forgets removed servers", () => {
    const service = new ConnectionStatusService();
    service.update({ serverId: "a", state: "connected" });
    service.forget("a");
    expect(service.statuses()).toEqual([]);
  });
});
