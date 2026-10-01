import { Injectable, computed, inject, signal } from "@angular/core";
import type { MessageFilter, MessageRecord } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { MessageFilterState, NO_FILTER, matchesFilter } from "./inspector.model";

/** Newest messages kept in memory; older ones stay in the database. */
export const MAX_MESSAGES = 5000;

function toBackend(filter: MessageFilterState): MessageFilter {
  return {
    serverId: filter.serverId || null,
    method: filter.method || null,
    direction: filter.direction || null,
    errorsOnly: filter.errorsOnly,
    search: filter.search || null,
    limit: 1000,
  };
}

/** The message timeline: a filtered window onto everything recorded, kept current live. */
@Injectable({ providedIn: "root" })
export class InspectorStore {
  private readonly ipc = inject(TauriIpcService);

  private readonly _messages = signal<MessageRecord[]>([]);
  private readonly _filter = signal<MessageFilterState>(NO_FILTER);
  private readonly _selected = signal<number[]>([]);
  private live: Promise<() => void> | null = null;

  /** `sessionId:jsonrpcId` -> request method, for responses that do not carry one. */
  private readonly methods = new Map<string, string>();

  readonly messages = this._messages.asReadonly();
  readonly filter = this._filter.asReadonly();
  /** Ids selected in the list, at most two (the last two clicked). */
  readonly selectedIds = this._selected.asReadonly();
  readonly selected = computed(() => {
    const ids = this._selected();
    const last = ids[ids.length - 1];
    return this._messages().find((m) => m.id === last) ?? null;
  });
  readonly comparison = computed(() => {
    const [a, b] = this._selected();
    if (a === undefined || b === undefined) return null;
    const byId = new Map(this._messages().map((m) => [m.id, m]));
    const first = byId.get(a);
    const second = byId.get(b);
    return first && second ? ([first, second] as const) : null;
  });

  requestMethod(message: MessageRecord): string | undefined {
    return message.method ?? this.methods.get(this.key(message.sessionId, message.jsonrpcId));
  }

  async load(): Promise<void> {
    const rows = await this.ipc.messagesQuery(toBackend(this._filter()));
    this.methods.clear();
    rows.forEach((row) => this.remember(row));
    this._messages.set(rows);
    this._selected.set([]);
  }

  async setFilter(partial: Partial<MessageFilterState>): Promise<void> {
    this._filter.update((f) => ({ ...f, ...partial }));
    await this.load();
  }

  /** Empties the view (the recorded data is kept). New messages continue to arrive. */
  clearView(): void {
    this._messages.set([]);
    this._selected.set([]);
  }

  /** Selects a message; with `additive` the previous selection is kept as comparison partner. */
  select(id: number, additive: boolean): void {
    this._selected.update((ids) => {
      if (!additive) return [id];
      const without = ids.filter((x) => x !== id);
      return [...without, id].slice(-2);
    });
  }

  /** Subscribes to `mcp://message` once. */
  startLive(): Promise<() => void> {
    this.live ??= this.ipc.listen<MessageRecord>("mcp://message", (message) =>
      this.receive(message),
    );
    return this.live;
  }

  receive(message: MessageRecord): void {
    this.remember(message);
    const lookup = (id: string | null, session: string) => this.methods.get(this.key(session, id));
    if (!matchesFilter(message, this._filter(), lookup)) return;
    const withDuration = this.withDuration(message);
    this._messages.update((list) => {
      if (list.some((m) => m.id === message.id)) return list;
      const next = [...list, withDuration];
      return next.length > MAX_MESSAGES ? next.slice(next.length - MAX_MESSAGES) : next;
    });
  }

  /** Shows an exact token count that the backend saved for a message. */
  applyExactCount(id: number, tokens: number): void {
    this._messages.update((list) =>
      list.map((m) => (m.id === id ? { ...m, tokens, tokenSource: "exact" as const } : m)),
    );
  }

  private withDuration(message: MessageRecord): MessageRecord {
    if (message.method || message.durationMs != null || message.jsonrpcId === null) return message;
    const list = this._messages();
    let request: MessageRecord | undefined;
    for (let i = list.length - 1; i >= 0 && !request; i--) {
      const candidate = list[i];
      if (
        candidate &&
        candidate.sessionId === message.sessionId &&
        candidate.jsonrpcId === message.jsonrpcId &&
        candidate.method !== null &&
        candidate.direction !== message.direction
      ) {
        request = candidate;
      }
    }
    return request ? { ...message, durationMs: Math.max(0, message.ts - request.ts) } : message;
  }

  private remember(message: MessageRecord): void {
    if (message.method && message.jsonrpcId !== null) {
      this.methods.set(this.key(message.sessionId, message.jsonrpcId), message.method);
    }
  }

  private key(sessionId: string, jsonrpcId: string | null): string {
    return `${sessionId}:${jsonrpcId ?? ""}`;
  }
}
