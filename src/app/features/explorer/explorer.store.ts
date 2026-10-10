import { Injectable, WritableSignal, inject, signal } from "@angular/core";
import type {
  ListChangedEvent,
  PromptInfo,
  ResourceInfo,
  ResourceTemplateInfo,
  ResourceUpdatedEvent,
  ServerDetails,
  StatusEvent,
  ToolInfo,
} from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";

export interface ServerSnapshot {
  details: ServerDetails | null;
  tools: ToolInfo[];
  resources: ResourceInfo[];
  templates: ResourceTemplateInfo[];
  prompts: PromptInfo[];
  loading: boolean;
  error: string | null;
}

const EMPTY: ServerSnapshot = {
  details: null,
  tools: [],
  resources: [],
  templates: [],
  prompts: [],
  loading: false,
  error: null,
};

const NONE: ReadonlySet<string> = new Set();

/** What each connected server offers. Refreshes lists when the server says they changed. */
@Injectable({ providedIn: "root" })
export class ExplorerStore {
  private readonly ipc = inject(TauriIpcService);
  private readonly _snapshots = signal<ReadonlyMap<string, ServerSnapshot>>(new Map());
  readonly snapshots = this._snapshots.asReadonly();
  private listening: Promise<() => void> | null = null;
  /** Resources being watched, per server. Ends with the session. */
  private readonly _watched = signal<ReadonlyMap<string, ReadonlySet<string>>>(new Map());
  /** Watched resources that changed since they were last read, per server. */
  private readonly _changed = signal<ReadonlyMap<string, ReadonlySet<string>>>(new Map());

  snapshot(serverId: string): ServerSnapshot {
    return this._snapshots().get(serverId) ?? EMPTY;
  }

  watched(serverId: string): ReadonlySet<string> {
    return this._watched().get(serverId) ?? NONE;
  }

  changed(serverId: string): ReadonlySet<string> {
    return this._changed().get(serverId) ?? NONE;
  }

  /** Subscribes to the server events once. */
  listen(): Promise<() => void> {
    this.listening ??= this.listenAll();
    return this.listening;
  }

  private async listenAll(): Promise<() => void> {
    const stops = await Promise.all([
      this.ipc.listen<ListChangedEvent>("mcp://list-changed", (event) => {
        void this.reloadKind(event.serverId, event.kind);
      }),
      this.ipc.listen<ResourceUpdatedEvent>("mcp://resource-updated", (event) => {
        if (this.watched(event.serverId).has(event.uri)) {
          this.setIn(this._changed, event.serverId, (set) => set.add(event.uri));
        }
      }),
      // Subscriptions end with the session.
      this.ipc.listen<StatusEvent>("mcp://status", (event) => {
        if (event.state !== "connected") this.forgetSession(event.serverId);
      }),
    ]);
    return () => stops.forEach((stop) => stop());
  }

  /** Starts watching a resource; the server must offer `resources.subscribe`. */
  async watch(serverId: string, uri: string): Promise<void> {
    await this.ipc.resourceSubscribe(serverId, uri);
    this.setIn(this._watched, serverId, (set) => set.add(uri));
  }

  async unwatch(serverId: string, uri: string): Promise<void> {
    await this.ipc.resourceUnsubscribe(serverId, uri);
    this.setIn(this._watched, serverId, (set) => set.delete(uri));
    this.setIn(this._changed, serverId, (set) => set.delete(uri));
  }

  /** The user read the resource again. */
  markRead(serverId: string, uri: string): void {
    this.setIn(this._changed, serverId, (set) => set.delete(uri));
  }

  /** Takes over what the live session says is watched (after connecting or reloading). */
  async loadSession(serverId: string): Promise<void> {
    const state = await this.ipc.sessionState(serverId);
    this.setIn(this._watched, serverId, (set) => {
      set.clear();
      state.subscriptions.forEach((uri) => set.add(uri));
    });
  }

  private forgetSession(serverId: string): void {
    this.setIn(this._watched, serverId, (set) => set.clear());
    this.setIn(this._changed, serverId, (set) => set.clear());
  }

  private setIn(
    target: WritableSignal<ReadonlyMap<string, ReadonlySet<string>>>,
    serverId: string,
    change: (set: Set<string>) => void,
  ): void {
    target.update((map) => {
      const set = new Set(map.get(serverId) ?? []);
      change(set);
      return new Map(map).set(serverId, set);
    });
  }

  async load(serverId: string): Promise<void> {
    this.patch(serverId, { loading: true, error: null });
    try {
      const details = await this.ipc.serverDetails(serverId);
      const [tools, resources, templates, prompts] = await Promise.all([
        this.ipc.toolsList(serverId),
        this.ipc.resourcesList(serverId),
        this.ipc.resourceTemplatesList(serverId),
        this.ipc.promptsList(serverId),
      ]);
      this.patch(serverId, { details, tools, resources, templates, prompts, loading: false });
    } catch (error) {
      this.patch(serverId, {
        loading: false,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }

  async reloadKind(serverId: string, kind: ListChangedEvent["kind"]): Promise<void> {
    if (!this._snapshots().has(serverId)) return;
    try {
      if (kind === "tools") this.patch(serverId, { tools: await this.ipc.toolsList(serverId) });
      else if (kind === "prompts") {
        this.patch(serverId, { prompts: await this.ipc.promptsList(serverId) });
      } else {
        const [resources, templates] = await Promise.all([
          this.ipc.resourcesList(serverId),
          this.ipc.resourceTemplatesList(serverId),
        ]);
        this.patch(serverId, { resources, templates });
      }
    } catch (error) {
      this.patch(serverId, { error: error instanceof Error ? error.message : String(error) });
    }
  }

  clear(serverId: string): void {
    this._snapshots.update((map) => {
      const next = new Map(map);
      next.delete(serverId);
      return next;
    });
  }

  private patch(serverId: string, partial: Partial<ServerSnapshot>): void {
    this._snapshots.update((map) =>
      new Map(map).set(serverId, { ...(map.get(serverId) ?? EMPTY), ...partial }),
    );
  }
}
