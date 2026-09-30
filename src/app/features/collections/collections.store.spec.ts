import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import type { CollectionTree } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { CollectionsStore } from "./collections.store";

function create(initial: CollectionTree) {
  let tree = initial;
  const calls: string[] = [];
  const ipc = {
    collectionsTree: async () => tree,
    collectionCreate: async (parentId: string | null, name: string) => {
      calls.push(`create:${name}`);
      const node = { id: `id-${name}`, parentId, name, sortOrder: 0 };
      tree = { ...tree, collections: [...tree.collections, node] };
      return node;
    },
    collectionDelete: async (id: string) => {
      calls.push(`delete:${id}`);
      tree = { ...tree, collections: tree.collections.filter((c) => c.id !== id) };
    },
    collectionImport: async () => ({
      collectionId: "imported",
      collectionsCreated: 1,
      requestsCreated: 0,
      skipped: [],
    }),
  };
  const injector = Injector.create({ providers: [{ provide: TauriIpcService, useValue: ipc }] });
  return { store: runInInjectionContext(injector, () => new CollectionsStore()), calls };
}

describe("CollectionsStore", () => {
  it("loads and exposes the nested tree and folder options", async () => {
    const { store } = create({
      collections: [{ id: "a", parentId: null, name: "A", sortOrder: 0 }],
      requests: [],
    });
    await store.load();
    expect(store.roots().map((r) => r.node.id)).toEqual(["a"]);
    expect(store.options()).toEqual([{ id: "a", label: "A" }]);
  });

  it("reloads after creating a folder and expands its parent", async () => {
    const { store } = create({ collections: [], requests: [] });
    await store.load();
    const parent = await store.createFolder(null, "Parent");
    await store.createFolder(parent.id, "Child");
    expect(store.roots()[0]?.folders[0]?.node.name).toBe("Child");
    expect(store.expanded().has(parent.id)).toBe(true);
  });

  it("removes deleted folders", async () => {
    const { store, calls } = create({ collections: [], requests: [] });
    const folder = await store.createFolder(null, "Temp");
    await store.deleteFolder(folder.id);
    expect(store.roots()).toEqual([]);
    expect(calls).toEqual(["create:Temp", `delete:${folder.id}`]);
  });

  it("toggles expansion", () => {
    const { store } = create({ collections: [], requests: [] });
    store.toggle("x");
    expect(store.expanded().has("x")).toBe(true);
    store.toggle("x");
    expect(store.expanded().has("x")).toBe(false);
  });

  it("expands the imported folder", async () => {
    const { store } = create({ collections: [], requests: [] });
    const report = await store.importFile("/tmp/x.json", null);
    expect(report.collectionsCreated).toBe(1);
    expect(store.expanded().has("imported")).toBe(true);
  });
});
