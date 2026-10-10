import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import type { ListChangedEvent, ToolInfo } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ExplorerStore } from "./explorer.store";

const tool = (name: string): ToolInfo => ({ name });

function create(overrides: Record<string, unknown> = {}) {
  let tools = [tool("a")];
  const handlers = new Map<string, (e: never) => void>();
  const subscribed: string[] = [];
  const ipc = {
    listen: async (event: string, h: (e: never) => void) => {
      handlers.set(event, h);
      return () => {};
    },
    resourceSubscribe: async (_: string, uri: string) => {
      subscribed.push(uri);
    },
    resourceUnsubscribe: async () => undefined,
    sessionState: async () => ({ subscriptions: ["test://kept"], logLevel: null }),
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
    emit: (e: ListChangedEvent) => handlers.get("mcp://list-changed")?.(e as never),
    send: (event: string, payload: unknown) => handlers.get(event)?.(payload as never),
    subscribed,
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

  it("marks a watched resource as changed until it is read again", async () => {
    const { store, send } = create();
    await store.listen();
    await store.watch("s", "test://a");
    send("mcp://resource-updated", { serverId: "s", uri: "test://a" });
    send("mcp://resource-updated", { serverId: "s", uri: "test://other" });
    expect([...store.changed("s")]).toEqual(["test://a"]);
    store.markRead("s", "test://a");
    expect(store.changed("s").size).toBe(0);
    expect(store.watched("s").has("test://a")).toBe(true);
  });

  it("stops watching and forgets the change", async () => {
    const { store, send } = create();
    await store.listen();
    await store.watch("s", "test://a");
    send("mcp://resource-updated", { serverId: "s", uri: "test://a" });
    await store.unwatch("s", "test://a");
    expect(store.watched("s").size).toBe(0);
    expect(store.changed("s").size).toBe(0);
  });

  it("forgets subscriptions when the session ends", async () => {
    const { store, send } = create();
    await store.listen();
    await store.watch("s", "test://a");
    send("mcp://status", { serverId: "s", sessionId: null, state: "connected", message: null });
    expect(store.watched("s").size).toBe(1);
    send("mcp://status", { serverId: "s", sessionId: null, state: "disconnected", message: null });
    expect(store.watched("s").size).toBe(0);
  });

  it("takes over the subscriptions of the live session", async () => {
    const { store } = create();
    await store.loadSession("s");
    expect([...store.watched("s")]).toEqual(["test://kept"]);
  });
});
