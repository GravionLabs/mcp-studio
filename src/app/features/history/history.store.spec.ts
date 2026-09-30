import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import type { HistoryEntry, HistoryFilter } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { HistoryStore } from "./history.store";

const entry = (id: number): HistoryEntry => ({
  id,
  serverId: "s",
  method: "tools/call",
  target: "echo",
  arguments: {},
  isError: false,
  cancelled: false,
  durationMs: 1,
  result: null,
  error: null,
  ts: id,
});

function create() {
  const filters: HistoryFilter[] = [];
  let rows = [entry(2), entry(1)];
  const ipc = {
    historyList: async (filter: HistoryFilter) => {
      filters.push(filter);
      return rows;
    },
    historyClear: async () => {
      const removed = rows.length;
      rows = [];
      return removed;
    },
  };
  const injector = Injector.create({ providers: [{ provide: TauriIpcService, useValue: ipc }] });
  return { store: runInInjectionContext(injector, () => new HistoryStore()), filters };
}

describe("HistoryStore", () => {
  it("loads entries and finds them by id", async () => {
    const { store } = create();
    await store.load({ serverId: "s" });
    expect(store.entries().map((e) => e.id)).toEqual([2, 1]);
    expect(store.entryById(1)?.id).toBe(1);
    expect(store.entryById(99)).toBeUndefined();
  });

  it("reloads with the last filter", async () => {
    const { store, filters } = create();
    await store.load({ search: "echo" });
    await store.reload();
    expect(filters).toEqual([{ search: "echo" }, { search: "echo" }]);
  });

  it("clears and refreshes", async () => {
    const { store } = create();
    await store.load();
    expect(await store.clear(null)).toBe(2);
    expect(store.entries()).toEqual([]);
  });
});
