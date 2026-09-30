import { Injectable, computed, inject, signal } from "@angular/core";
import { KEY_VALUE_STORE } from "./storage";

export type Theme = "dark" | "light";
const STORAGE_KEY = "mcp-studio.theme";

/** Dark is the default; the choice is remembered. */
@Injectable({ providedIn: "root" })
export class ThemeService {
  private readonly store = inject(KEY_VALUE_STORE);
  private readonly _theme = signal<Theme>(this.read());

  readonly theme = this._theme.asReadonly();
  readonly isDark = computed(() => this._theme() === "dark");

  set(theme: Theme): void {
    this._theme.set(theme);
    this.store.set(STORAGE_KEY, theme);
  }

  toggle(): void {
    this.set(this._theme() === "dark" ? "light" : "dark");
  }

  private read(): Theme {
    return this.store.get(STORAGE_KEY) === "light" ? "light" : "dark";
  }
}
