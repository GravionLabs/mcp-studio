import { Injectable, inject, signal } from "@angular/core";
import type { HistoryEntry, HistoryFilter } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";

/** Request history. Reloads with the last used filter so views stay consistent after new calls. */
@Injectable({ providedIn: "root" })
export class HistoryStore {
  private readonly ipc = inject(TauriIpcService);
  private readonly _entries = signal<HistoryEntry[]>([]);
  private filter: HistoryFilter = {};

  readonly entries = this._entries.asReadonly();

  entryById(id: number): HistoryEntry | undefined {
    return this._entries().find((e) => e.id === id);
  }

  async load(filter: HistoryFilter = this.filter): Promise<void> {
    this.filter = filter;
    this._entries.set(await this.ipc.historyList(filter));
  }

  reload(): Promise<void> {
    return this.load(this.filter);
  }

  async clear(serverId: string | null): Promise<number> {
    const removed = await this.ipc.historyClear(serverId);
    await this.reload();
    return removed;
  }
}
