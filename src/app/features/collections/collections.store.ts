import { Injectable, computed, inject, signal } from "@angular/core";
import type {
  CollectionNode,
  CollectionTree,
  ImportReport,
  SavedRequest,
  SavedRequestInput,
} from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { buildTree, folderOptions } from "./collections.model";

const EMPTY: CollectionTree = { collections: [], requests: [] };

/** Collections and saved requests. Every change is followed by a reload so the UI mirrors Rust. */
@Injectable({ providedIn: "root" })
export class CollectionsStore {
  private readonly ipc = inject(TauriIpcService);
  private readonly _tree = signal<CollectionTree>(EMPTY);
  private readonly _expanded = signal<ReadonlySet<string>>(new Set());

  readonly roots = computed(() => buildTree(this._tree()));
  readonly options = computed(() => folderOptions(this.roots()));
  readonly requests = computed(() => this._tree().requests);
  readonly expanded = this._expanded.asReadonly();

  requestById(id: string): SavedRequest | undefined {
    return this._tree().requests.find((r) => r.id === id);
  }

  async load(): Promise<void> {
    this._tree.set(await this.ipc.collectionsTree());
  }

  toggle(id: string): void {
    this._expanded.update((set) => {
      const next = new Set(set);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  }

  expand(id: string): void {
    this._expanded.update((set) => new Set(set).add(id));
  }

  async createFolder(parentId: string | null, name: string): Promise<CollectionNode> {
    const created = await this.ipc.collectionCreate(parentId, name);
    await this.load();
    if (parentId) this.expand(parentId);
    return created;
  }

  async renameFolder(id: string, name: string): Promise<void> {
    await this.ipc.collectionRename(id, name);
    await this.load();
  }

  async moveFolder(id: string, parentId: string | null): Promise<void> {
    await this.ipc.collectionMove(id, parentId);
    await this.load();
  }

  async deleteFolder(id: string): Promise<void> {
    await this.ipc.collectionDelete(id);
    await this.load();
  }

  async saveRequest(input: SavedRequestInput): Promise<SavedRequest> {
    const saved = await this.ipc.requestSave(input);
    await this.load();
    this.expand(input.collectionId);
    return saved;
  }

  async updateRequest(id: string, input: SavedRequestInput): Promise<void> {
    await this.ipc.requestUpdate(id, input);
    await this.load();
  }

  async deleteRequest(id: string): Promise<void> {
    await this.ipc.requestDelete(id);
    await this.load();
  }

  async exportFolder(id: string, path: string): Promise<void> {
    await this.ipc.collectionExport(id, path);
  }

  async importFile(path: string, parentId: string | null): Promise<ImportReport> {
    const report = await this.ipc.collectionImport(path, parentId);
    await this.load();
    this.expand(report.collectionId);
    return report;
  }
}
