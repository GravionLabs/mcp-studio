import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it, vi } from "vitest";
import type { Price } from "../../core/bindings";
import { KEY_VALUE_STORE, memoryStore } from "../../core/storage";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { PricesStore } from "./prices.store";

const price = (model: string): Price => ({
  model,
  inputPerMtok: 1,
  outputPerMtok: 2,
  cacheReadPerMtok: null,
  cacheWritePerMtok: null,
  currency: "USD",
});

function create(initial: Price[], stored: Record<string, string> = {}) {
  const priceList = vi.fn(async () => initial);
  const ipc = {
    priceList,
    priceSet: async (p: Price) => p,
    priceRemove: async () => undefined,
  };
  const storage = memoryStore(stored);
  const injector = Injector.create({
    providers: [
      { provide: TauriIpcService, useValue: ipc },
      { provide: KEY_VALUE_STORE, useValue: storage },
    ],
  });
  return { store: runInInjectionContext(injector, () => new PricesStore()), priceList, storage };
}

describe("PricesStore", () => {
  it("loads the table once", async () => {
    const { store, priceList } = create([price("a")]);
    await store.load();
    await store.load();
    expect(priceList).toHaveBeenCalledTimes(1);
    expect(store.prices()).toEqual([price("a")]);
  });

  it("has no active model until one is chosen", async () => {
    const { store } = create([price("a")]);
    await store.load();
    expect(store.activeModel()).toBeNull();
    expect(store.activePrice()).toBeUndefined();
  });

  it("remembers the active model", async () => {
    const { store, storage } = create([price("a"), price("b")]);
    await store.load();
    store.setActive("b");
    expect(store.activePrice()?.model).toBe("b");
    expect(storage.get("mcp-studio.pricingModel")).toBe("b");
  });

  it("ignores a stored model that no longer has a price", async () => {
    const { store } = create([price("a")], { "mcp-studio.pricingModel": "gone" });
    await store.load();
    expect(store.activeModel()).toBeNull();
  });

  it("adds, replaces, sorts, and removes prices", async () => {
    const { store } = create([price("b")]);
    await store.load();
    await store.save(price("a"));
    await store.save({ ...price("b"), inputPerMtok: 9 });
    expect(store.prices().map((p) => [p.model, p.inputPerMtok])).toEqual([
      ["a", 1],
      ["b", 9],
    ]);
    await store.remove("a");
    expect(store.prices().map((p) => p.model)).toEqual(["b"]);
  });
});
