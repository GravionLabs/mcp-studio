import { Injectable, inject, signal } from "@angular/core";
import type {
  ListChangedEvent,
  PromptInfo,
  ResourceInfo,
  ResourceTemplateInfo,
  ServerDetails,
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

/** What each connected server offers. Refreshes lists when the server says they changed. */
@Injectable({ providedIn: "root" })
export class ExplorerStore {
  private readonly ipc = inject(TauriIpcService);
  private readonly _snapshots = signal<ReadonlyMap<string, ServerSnapshot>>(new Map());
  readonly snapshots = this._snapshots.asReadonly();
  private listening: Promise<() => void> | null = null;

  snapshot(serverId: string): ServerSnapshot {
    return this._snapshots().get(serverId) ?? EMPTY;
  }

  /** Subscribes to `mcp://list-changed` once. */
  listen(): Promise<() => void> {
    this.listening ??= this.ipc.listen<ListChangedEvent>("mcp://list-changed", (event) => {
      void this.reloadKind(event.serverId, event.kind);
    });
    return this.listening;
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
