import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import type { Environment, EnvironmentInput } from "../../core/bindings";
import { KEY_VALUE_STORE, memoryStore } from "../../core/storage";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { EnvironmentsStore } from "./environments.store";

const env = (id: string, name: string): Environment => ({ id, name, variables: {} });

function create(initial: Environment[], stored: Record<string, string> = {}) {
  const storage = memoryStore(stored);
  const proxyCalls: (string | null)[] = [];
  const ipc = {
    environmentList: async () => initial,
    environmentAdd: async (i: EnvironmentInput) => env(`id-${i.name}`, i.name),
    environmentUpdate: async (id: string, i: EnvironmentInput) => env(id, i.name),
    environmentRemove: async () => undefined,
    proxySetEnvironment: async (id: string | null) => void proxyCalls.push(id),
  };
  const injector = Injector.create({
    providers: [
      { provide: TauriIpcService, useValue: ipc },
      { provide: KEY_VALUE_STORE, useValue: storage },
    ],
  });
  return {
    store: runInInjectionContext(injector, () => new EnvironmentsStore()),
    storage,
    proxyCalls,
  };
}

describe("EnvironmentsStore", () => {
  it("has no active environment by default", async () => {
    const { store } = create([env("a", "dev")]);
    await store.load();
    expect(store.active()).toBeUndefined();
    expect(store.activeId()).toBeNull();
  });

  it("remembers the active environment", async () => {
    const first = create([env("a", "dev")]);
    await first.store.load();
    first.store.setActive("a");
    expect(first.store.active()?.name).toBe("dev");
    const second = create([env("a", "dev")], {
      "mcp-studio.activeEnvironment": first.storage.get("mcp-studio.activeEnvironment") ?? "",
    });
    await second.store.load();
    expect(second.store.activeId()).toBe("a");
  });

  it("ignores a remembered id that no longer exists", async () => {
    const { store } = create([env("a", "dev")], { "mcp-studio.activeEnvironment": "gone" });
    await store.load();
    expect(store.activeId()).toBeNull();
  });

  it("deselects the active environment when it is removed", async () => {
    const { store } = create([env("a", "dev"), env("b", "prod")]);
    await store.load();
    store.setActive("a");
    await store.remove("a");
    expect(store.activeId()).toBeNull();
    expect(store.environments().map((e) => e.id)).toEqual(["b"]);
  });

  it("keeps environments sorted after add and update", async () => {
    const { store } = create([env("b", "beta")]);
    await store.load();
    await store.add({ name: "Alpha", variables: {} });
    await store.update("b", { name: "zeta", variables: {} });
    expect(store.environments().map((e) => e.name)).toEqual(["Alpha", "zeta"]);
  });

  it("tells the proxy which environment is active", async () => {
    const { store, proxyCalls } = create([env("a", "dev")]);
    await store.load();
    store.setActive("a");
    store.setActive(null);
    expect(proxyCalls).toEqual([null, "a", null]);
  });
});
