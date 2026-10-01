import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it, vi } from "vitest";
import type { UpdateInfo } from "./bindings";
import { TauriIpcService } from "./tauri-ipc.service";
import { UpdateService } from "./update.service";

const newer: UpdateInfo = {
  version: "0.2.0",
  currentVersion: "0.1.0",
  notes: "Fixes",
  date: null,
};

function create(ipc: Partial<TauriIpcService>): UpdateService {
  const injector = Injector.create({
    providers: [{ provide: TauriIpcService, useValue: ipc }],
  });
  return runInInjectionContext(injector, () => new UpdateService());
}

describe("UpdateService", () => {
  it("starts idle and does not check by itself", () => {
    const updateCheck = vi.fn();
    const service = create({ updateCheck });
    expect(service.state()).toBe("idle");
    expect(updateCheck).not.toHaveBeenCalled();
  });

  it("reports an available update", async () => {
    const service = create({ updateCheck: async () => newer });
    await service.check();
    expect(service.state()).toBe("available");
    expect(service.info()).toEqual(newer);
  });

  it("reports when the app is up to date", async () => {
    const service = create({ updateCheck: async () => null });
    await service.check();
    expect(service.state()).toBe("upToDate");
    expect(service.info()).toBeNull();
  });

  it("shows the error when the check fails and allows trying again", async () => {
    const updateCheck = vi
      .fn()
      .mockRejectedValueOnce(new Error("offline"))
      .mockResolvedValueOnce(null);
    const service = create({ updateCheck });
    await service.check();
    expect(service.state()).toBe("error");
    expect(service.error()).toContain("offline");
    await service.check();
    expect(service.state()).toBe("upToDate");
    expect(service.error()).toBeNull();
  });

  it("ignores a second check while one is running", async () => {
    let resolve: (value: UpdateInfo | null) => void = () => {};
    const updateCheck = vi.fn(() => new Promise<UpdateInfo | null>((r) => (resolve = r)));
    const service = create({ updateCheck });
    const first = service.check();
    await service.check();
    expect(updateCheck).toHaveBeenCalledTimes(1);
    resolve(null);
    await first;
  });

  it("installs only after an update was found", async () => {
    const updateInstall = vi.fn(async () => {});
    const service = create({ updateCheck: async () => newer, updateInstall });
    await service.install();
    expect(updateInstall).not.toHaveBeenCalled();
    await service.check();
    await service.install();
    expect(updateInstall).toHaveBeenCalledTimes(1);
    expect(service.state()).toBe("installing");
  });

  it("returns to an error state when the installation fails", async () => {
    const service = create({
      updateCheck: async () => newer,
      updateInstall: async () => Promise.reject(new Error("signature mismatch")),
    });
    await service.check();
    await service.install();
    expect(service.state()).toBe("error");
    expect(service.error()).toContain("signature mismatch");
  });
});
