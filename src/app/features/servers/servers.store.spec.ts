import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import type { ServerDefinition, ServerInput } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ServersStore } from "./servers.store";

function definition(id: string, name: string): ServerDefinition {
  return {
    id,
    name,
    transport: "stdio",
    command: "x",
    args: [],
    env: {},
    cwd: null,
    url: null,
    headers: {},
    tags: [],
    createdAt: 1,
    updatedAt: 1,
  };
}

const input = (name: string): ServerInput => ({
  name,
  transport: "stdio",
  command: "x",
  args: [],
  env: {},
  cwd: null,
  url: null,
  headers: {},
  tags: [],
});

function createStore(initial: ServerDefinition[] = []) {
  const calls: string[] = [];
  const ipc = {
    serverList: async () => initial,
    serverAdd: async (i: ServerInput) => {
      calls.push("add");
      return definition(`id-${i.name}`, i.name);
    },
    serverUpdate: async (id: string, i: ServerInput) => {
      calls.push("update");
      return definition(id, i.name);
    },
    serverRemove: async () => void calls.push("remove"),
  };
  const injector = Injector.create({ providers: [{ provide: TauriIpcService, useValue: ipc }] });
  return { store: runInInjectionContext(injector, () => new ServersStore()), calls };
}

describe("ServersStore", () => {
  it("loads servers", async () => {
    const { store } = createStore([definition("a", "Alpha")]);
    expect(store.loaded()).toBe(false);
    await store.load();
    expect(store.servers()).toHaveLength(1);
    expect(store.loaded()).toBe(true);
    expect(store.byId().get("a")?.name).toBe("Alpha");
  });

  it("adds servers and keeps the list sorted ignoring case", async () => {
    const { store } = createStore([definition("b", "beta")]);
    await store.load();
    await store.add(input("Alpha"));
    await store.add(input("charlie"));
    expect(store.servers().map((s) => s.name)).toEqual(["Alpha", "beta", "charlie"]);
  });

  it("replaces a server on update", async () => {
    const { store } = createStore([definition("a", "Alpha")]);
    await store.load();
    await store.update("a", input("Renamed"));
    expect(store.servers()).toHaveLength(1);
    expect(store.servers()[0]?.name).toBe("Renamed");
  });

  it("removes servers", async () => {
    const { store, calls } = createStore([definition("a", "Alpha"), definition("b", "Beta")]);
    await store.load();
    await store.remove("a");
    expect(store.servers().map((s) => s.id)).toEqual(["b"]);
    expect(calls).toEqual(["remove"]);
  });

  it("keeps the list unchanged when the backend rejects", async () => {
    const { store } = createStore([definition("a", "Alpha")]);
    await store.load();
    const failing = { serverAdd: async () => Promise.reject(new Error("dup")) };
    Object.assign((store as unknown as { ipc: object }).ipc, failing);
    await expect(store.add(input("Alpha"))).rejects.toThrow("dup");
    expect(store.servers()).toHaveLength(1);
  });
});
