import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it, vi } from "vitest";
import { IpcError } from "./ipc-error";
import { IPC_INVOKE, IPC_LISTEN, InvokeFn, ListenFn } from "./ipc.tokens";
import { TauriIpcService } from "./tauri-ipc.service";

function create(invoke: InvokeFn, listen: ListenFn = async () => () => {}): TauriIpcService {
  const injector = Injector.create({
    providers: [
      { provide: IPC_INVOKE, useValue: invoke },
      { provide: IPC_LISTEN, useValue: listen },
    ],
  });
  return runInInjectionContext(injector, () => new TauriIpcService());
}

describe("TauriIpcService", () => {
  it("returns the typed result of a command", async () => {
    const invoke = vi.fn(async () => ({ name: "MCP Studio", version: "0.1.0" }));
    const service = create(invoke as unknown as InvokeFn);
    await expect(service.appInfo()).resolves.toEqual({ name: "MCP Studio", version: "0.1.0" });
    expect(invoke).toHaveBeenCalledWith("app_info", undefined);
  });

  it("wraps string rejections in IpcError", async () => {
    const service = create((async () => Promise.reject("boom")) as InvokeFn);
    await expect(service.call("x")).rejects.toMatchObject({
      name: "IpcError",
      command: "x",
      detail: "boom",
    });
  });

  it("wraps Error rejections in IpcError", async () => {
    const service = create((async () => Promise.reject(new Error("bad"))) as InvokeFn);
    const error = await service.call("y").catch((e: unknown) => e);
    expect(error).toBeInstanceOf(IpcError);
    expect((error as IpcError).detail).toBe("bad");
  });

  it("updates an event signal when payloads arrive and stops listening", async () => {
    let handler: ((payload: string) => void) | undefined;
    const unlisten = vi.fn();
    const listen = (async (_event: string, h: (payload: string) => void) => {
      handler = h;
      return unlisten;
    }) as unknown as ListenFn;
    const service = create((async () => undefined) as InvokeFn, listen);
    const { value, stop } = service.eventSignal<string>("mcp://status");
    expect(value()).toBeUndefined();
    await Promise.resolve();
    handler?.("connected");
    expect(value()).toBe("connected");
    stop();
    expect(unlisten).toHaveBeenCalledOnce();
  });

  it("unsubscribes even if stop is called before the listener is registered", async () => {
    const unlisten = vi.fn();
    const listen = (async () => unlisten) as unknown as ListenFn;
    const service = create((async () => undefined) as InvokeFn, listen);
    service.eventSignal<string>("e").stop();
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledOnce();
  });
});
