import { Injectable, computed, signal } from "@angular/core";

export interface WorkspaceTab {
  /** Stable identity, e.g. `server:abc` or `tool:abc:echo`. Opening the same id focuses the tab. */
  id: string;
  title: string;
  /** Router link the tab shows. */
  route: string;
}

/** Open tabs of the middle column. */
@Injectable({ providedIn: "root" })
export class WorkspaceTabsService {
  private readonly _tabs = signal<WorkspaceTab[]>([]);
  private readonly _activeId = signal<string | null>(null);

  readonly tabs = this._tabs.asReadonly();
  readonly activeId = this._activeId.asReadonly();
  readonly active = computed(() => this._tabs().find((t) => t.id === this._activeId()) ?? null);

  /** Opens a tab (or refreshes title/route of an existing one) and activates it. */
  open(tab: WorkspaceTab): void {
    this._tabs.update((list) => {
      const index = list.findIndex((t) => t.id === tab.id);
      if (index === -1) return [...list, tab];
      const next = [...list];
      next[index] = tab;
      return next;
    });
    this._activeId.set(tab.id);
  }

  activate(id: string): void {
    if (this._tabs().some((t) => t.id === id)) this._activeId.set(id);
  }

  /** Closes a tab; if it was active, the neighbour to the left (or right) becomes active. */
  close(id: string): void {
    const list = this._tabs();
    const index = list.findIndex((t) => t.id === id);
    if (index === -1) return;
    const next = list.filter((t) => t.id !== id);
    this._tabs.set(next);
    if (this._activeId() === id) {
      this._activeId.set(next[Math.max(0, index - 1)]?.id ?? null);
    }
  }
}
