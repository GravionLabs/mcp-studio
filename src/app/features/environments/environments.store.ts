import { Injectable, computed, inject, signal } from "@angular/core";
import type { Environment, EnvironmentInput } from "../../core/bindings";
import { KEY_VALUE_STORE } from "../../core/storage";
import { TauriIpcService } from "../../core/tauri-ipc.service";

const ACTIVE_KEY = "mcp-studio.activeEnvironment";

/** Environments and the one that is currently active (used to resolve `{{placeholders}}`). */
@Injectable({ providedIn: "root" })
export class EnvironmentsStore {
  private readonly ipc = inject(TauriIpcService);
  private readonly storage = inject(KEY_VALUE_STORE);

  private readonly _environments = signal<Environment[]>([]);
  private readonly _activeId = signal<string | null>(this.storage.get(ACTIVE_KEY) || null);

  readonly environments = this._environments.asReadonly();
  /** Id of the active environment, or null. Only ids of existing environments are returned. */
  readonly activeId = computed(() => {
    const id = this._activeId();
    return id !== null && this._environments().some((e) => e.id === id) ? id : null;
  });
  readonly active = computed(() => this._environments().find((e) => e.id === this.activeId()));

  async load(): Promise<void> {
    this._environments.set(await this.ipc.environmentList());
    this.syncProxy();
  }

  setActive(id: string | null): void {
    this._activeId.set(id);
    this.storage.set(ACTIVE_KEY, id ?? "");
    this.syncProxy();
  }

  /** Sessions started through the proxy use the active environment's variables. */
  private syncProxy(): void {
    void this.ipc.proxySetEnvironment(this.activeId()).catch(() => undefined);
  }

  async add(input: EnvironmentInput): Promise<Environment> {
    const created = await this.ipc.environmentAdd(input);
    this.upsert(created);
    return created;
  }

  async update(id: string, input: EnvironmentInput): Promise<Environment> {
    const updated = await this.ipc.environmentUpdate(id, input);
    this.upsert(updated);
    return updated;
  }

  async remove(id: string): Promise<void> {
    await this.ipc.environmentRemove(id);
    this._environments.update((list) => list.filter((e) => e.id !== id));
  }

  private upsert(environment: Environment): void {
    this._environments.update((list) =>
      [...list.filter((e) => e.id !== environment.id), environment].sort((a, b) =>
        a.name.localeCompare(b.name, undefined, { sensitivity: "base" }),
      ),
    );
  }
}
