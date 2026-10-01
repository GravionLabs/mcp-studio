import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import type { MessageFilter, MessageRecord } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { InspectorStore, MAX_MESSAGES } from "./inspector.store";

let nextId = 1;
const message = (overrides: Partial<MessageRecord> = {}): MessageRecord => ({
  id: nextId++,
  sessionId: "sess",
  serverId: "srv",
  direction: "out",
  jsonrpcId: "1",
  method: "tools/list",
  payload: "{}",
  bytes: 2,
  isError: false,
  ts: 1000,
  durationMs: null,
  tokens: null,
  tokenSource: null,
  ...overrides,
});

function create(rows: MessageRecord[] = []) {
  const queries: MessageFilter[] = [];
  let handler: ((m: MessageRecord) => void) | undefined;
  const ipc = {
    messagesQuery: async (filter: MessageFilter) => {
      queries.push(filter);
      return rows;
    },
    listen: async (_: string, h: (m: MessageRecord) => void) => {
      handler = h;
      return () => {};
    },
  };
  const injector = Injector.create({ providers: [{ provide: TauriIpcService, useValue: ipc }] });
  return {
    store: runInInjectionContext(injector, () => new InspectorStore()),
    queries,
    emit: (m: MessageRecord) => handler?.(m),
  };
}

describe("InspectorStore", () => {
  it("loads with the filter translated for the backend", async () => {
    const { store, queries } = create();
    await store.setFilter({ serverId: "srv", errorsOnly: true, search: "boom" });
    expect(queries.at(-1)).toMatchObject({
      serverId: "srv",
      errorsOnly: true,
      search: "boom",
      method: null,
      direction: null,
      limit: 1000,
    });
  });

  it("appends live messages that match the filter, once", async () => {
    const { store, emit } = create();
    await store.startLive();
    await store.setFilter({ serverId: "srv" });
    const mine = message();
    emit(mine);
    emit(mine);
    emit(message({ serverId: "other" }));
    expect(store.messages().map((m) => m.id)).toEqual([mine.id]);
  });

  it("derives durations and request methods for live responses", async () => {
    const { store, emit } = create();
    await store.startLive();
    const request = message({ jsonrpcId: "7", method: "tools/call", ts: 1000 });
    emit(request);
    const response = message({ jsonrpcId: "7", method: null, direction: "in", ts: 1042 });
    emit(response);
    const stored = store.messages().find((m) => m.id === response.id);
    expect(stored?.durationMs).toBe(42);
    expect(store.requestMethod(response)).toBe("tools/call");
  });

  it("lets a method filter through responses of matching requests", async () => {
    const { store, emit } = create();
    await store.startLive();
    await store.setFilter({ method: "tools/call" });
    emit(message({ jsonrpcId: "1", method: "tools/list" }));
    emit(message({ jsonrpcId: "2", method: "tools/call" }));
    emit(message({ jsonrpcId: "2", method: null, direction: "in" }));
    emit(message({ jsonrpcId: "1", method: null, direction: "in" }));
    expect(store.messages()).toHaveLength(2);
  });

  it("keeps only the newest messages in memory", async () => {
    const { store } = create();
    for (let i = 0; i < MAX_MESSAGES + 10; i++) store.receive(message());
    expect(store.messages()).toHaveLength(MAX_MESSAGES);
  });

  it("tracks a selection of at most two messages for comparison", async () => {
    const a = message();
    const b = message();
    const c = message();
    const { store } = create([a, b, c]);
    await store.load();
    store.select(a.id, false);
    expect(store.selected()?.id).toBe(a.id);
    expect(store.comparison()).toBeNull();
    store.select(b.id, true);
    expect(store.comparison()?.map((m) => m.id)).toEqual([a.id, b.id]);
    store.select(c.id, true);
    expect(store.selectedIds()).toEqual([b.id, c.id]);
    store.select(a.id, false);
    expect(store.selectedIds()).toEqual([a.id]);
  });

  it("clears the view without touching later arrivals", async () => {
    const { store, emit } = create([message()]);
    await store.startLive();
    await store.load();
    store.clearView();
    expect(store.messages()).toEqual([]);
    emit(message());
    expect(store.messages()).toHaveLength(1);
  });
});
