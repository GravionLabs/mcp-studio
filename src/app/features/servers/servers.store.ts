import { Injectable, computed, inject, signal } from "@angular/core";
import type { ServerDefinition, ServerInput } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";

/** The registry as seen by the UI. Rust is the source of truth; every change reloads from its answer. */
@Injectable({ providedIn: "root" })
export class ServersStore {
  private readonly ipc = inject(TauriIpcService);
  private readonly _servers = signal<ServerDefinition[]>([]);
  private readonly _loaded = signal(false);

  readonly servers = this._servers.asReadonly();
  readonly loaded = this._loaded.asReadonly();
  readonly byId = computed(() => new Map(this._servers().map((s) => [s.id, s])));

  async load(): Promise<void> {
    this._servers.set(await this.ipc.serverList());
    this._loaded.set(true);
  }

  async add(input: ServerInput): Promise<ServerDefinition> {
    const created = await this.ipc.serverAdd(input);
    this.upsert(created);
    return created;
  }

  async update(id: string, input: ServerInput): Promise<ServerDefinition> {
    const updated = await this.ipc.serverUpdate(id, input);
    this.upsert(updated);
    return updated;
  }

  async remove(id: string): Promise<void> {
    await this.ipc.serverRemove(id);
    this._servers.update((list) => list.filter((s) => s.id !== id));
  }

  private upsert(server: ServerDefinition): void {
    this._servers.update((list) => {
      const next = list.filter((s) => s.id !== server.id);
      next.push(server);
      return next.sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
    });
  }
}
