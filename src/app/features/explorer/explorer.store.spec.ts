import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import type { ListChangedEvent, ToolInfo } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ExplorerStore } from "./explorer.store";

const tool = (name: string): ToolInfo => ({ name });

function create(overrides: Record<string, unknown> = {}) {
  let tools = [tool("a")];
  let handler: ((e: ListChangedEvent) => void) | undefined;
  const ipc = {
    listen: async (_: string, h: (e: ListChangedEvent) => void) => {
      handler = h;
      return () => {};
    },
    serverDetails: async () => ({ name: "srv", version: "1", protocolVersion: "x" }),
    toolsList: async () => tools,
    resourcesList: async () => [],
    resourceTemplatesList: async () => [],
    promptsList: async () => [],
    ...overrides,
  };
  const injector = Injector.create({ providers: [{ provide: TauriIpcService, useValue: ipc }] });
  return {
    store: runInInjectionContext(injector, () => new ExplorerStore()),
    setTools: (next: ToolInfo[]) => (tools = next),
    emit: (e: ListChangedEvent) => handler?.(e),
  };
}

describe("ExplorerStore", () => {
  it("returns an empty snapshot for unknown servers", () => {
    expect(create().store.snapshot("x")).toMatchObject({
      tools: [],
      details: null,
      loading: false,
    });
  });

  it("loads everything a server offers", async () => {
    const { store } = create();
    await store.load("s");
    expect(store.snapshot("s").tools).toEqual([tool("a")]);
    expect(store.snapshot("s").details?.name).toBe("srv");
    expect(store.snapshot("s").loading).toBe(false);
  });

  it("stores the error message when loading fails", async () => {
    const { store } = create({
      serverDetails: async () => Promise.reject(new Error("not connected")),
    });
    await store.load("s");
    expect(store.snapshot("s").error).toBe("not connected");
    expect(store.snapshot("s").loading).toBe(false);
  });

  it("refreshes only the changed list when the server announces a change", async () => {
    const { store, setTools, emit } = create();
    await store.listen();
    await store.load("s");
    setTools([tool("a"), tool("b")]);
    emit({ serverId: "s", kind: "tools" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(store.snapshot("s").tools.map((t) => t.name)).toEqual(["a", "b"]);
  });

  it("ignores change events for servers that were never loaded", async () => {
    const { store, emit } = create();
    await store.listen();
    emit({ serverId: "other", kind: "tools" });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(store.snapshots().size).toBe(0);
  });

  it("clears a server", async () => {
    const { store } = create();
    await store.load("s");
    store.clear("s");
    expect(store.snapshots().has("s")).toBe(false);
  });
});
