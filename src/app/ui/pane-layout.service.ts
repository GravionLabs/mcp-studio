import { Injectable, inject, signal } from "@angular/core";
import { KEY_VALUE_STORE } from "../core/storage";

export const SIDEBAR_LIMITS = { min: 180, max: 480, initial: 260 };
export const INSPECTOR_LIMITS = { min: 260, max: 760, initial: 420 };
const STORAGE_KEY = "mcp-studio.layout";

/** Clamps a pane width into its allowed range; non-finite input falls back to the minimum. */
export function clampWidth(value: number, limits: { min: number; max: number }): number {
  if (!Number.isFinite(value)) return limits.min;
  return Math.min(limits.max, Math.max(limits.min, Math.round(value)));
}

interface Persisted {
  sidebar: number;
  inspector: number;
  inspectorOpen: boolean;
}

/** Widths and visibility of the three columns; remembered between runs. */
@Injectable({ providedIn: "root" })
export class PaneLayoutService {
  private readonly store = inject(KEY_VALUE_STORE);
  private readonly saved = this.read();

  readonly sidebarWidth = signal(this.saved.sidebar);
  readonly inspectorWidth = signal(this.saved.inspector);
  readonly inspectorOpen = signal(this.saved.inspectorOpen);

  setSidebarWidth(value: number): void {
    this.sidebarWidth.set(clampWidth(value, SIDEBAR_LIMITS));
    this.persist();
  }

  setInspectorWidth(value: number): void {
    this.inspectorWidth.set(clampWidth(value, INSPECTOR_LIMITS));
    this.persist();
  }

  toggleInspector(): void {
    this.inspectorOpen.update((open) => !open);
    this.persist();
  }

  private persist(): void {
    const value: Persisted = {
      sidebar: this.sidebarWidth(),
      inspector: this.inspectorWidth(),
      inspectorOpen: this.inspectorOpen(),
    };
    this.store.set(STORAGE_KEY, JSON.stringify(value));
  }

  private read(): Persisted {
    const fallback: Persisted = {
      sidebar: SIDEBAR_LIMITS.initial,
      inspector: INSPECTOR_LIMITS.initial,
      inspectorOpen: true,
    };
    try {
      const raw = this.store.get(STORAGE_KEY);
      if (!raw) return fallback;
      const parsed = JSON.parse(raw) as Partial<Persisted>;
      return {
        sidebar: clampWidth(parsed.sidebar ?? fallback.sidebar, SIDEBAR_LIMITS),
        inspector: clampWidth(parsed.inspector ?? fallback.inspector, INSPECTOR_LIMITS),
        inspectorOpen: parsed.inspectorOpen ?? true,
      };
    } catch {
      return fallback;
    }
  }
}
