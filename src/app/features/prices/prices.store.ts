import { Injectable, computed, inject, signal } from "@angular/core";
import type { Price } from "../../core/bindings";
import { KEY_VALUE_STORE } from "../../core/storage";
import { TauriIpcService } from "../../core/tauri-ipc.service";

const ACTIVE_KEY = "mcp-studio.pricingModel";

/** The price table and the model whose prices are used for the costs shown in the app. */
@Injectable({ providedIn: "root" })
export class PricesStore {
  private readonly ipc = inject(TauriIpcService);
  private readonly storage = inject(KEY_VALUE_STORE);

  private readonly _prices = signal<Price[]>([]);
  private readonly _activeModel = signal<string | null>(this.storage.get(ACTIVE_KEY) || null);
  private loaded: Promise<void> | null = null;

  readonly prices = this._prices.asReadonly();
  /** Model chosen for cost estimates, or null. Only models that have a price are returned. */
  readonly activeModel = computed(() => {
    const model = this._activeModel();
    return model !== null && this._prices().some((p) => p.model === model) ? model : null;
  });
  readonly activePrice = computed(() => this._prices().find((p) => p.model === this.activeModel()));

  /** Loads the table once; later calls reuse the result. */
  load(): Promise<void> {
    this.loaded ??= this.ipc.priceList().then((prices) => this._prices.set(prices));
    this.loaded.catch(() => (this.loaded = null));
    return this.loaded;
  }

  setActive(model: string | null): void {
    this._activeModel.set(model);
    this.storage.set(ACTIVE_KEY, model ?? "");
  }

  async save(price: Price): Promise<Price> {
    const saved = await this.ipc.priceSet(price);
    this._prices.update((list) =>
      [...list.filter((p) => p.model !== saved.model), saved].sort((a, b) =>
        a.model.localeCompare(b.model, undefined, { sensitivity: "base" }),
      ),
    );
    return saved;
  }

  async remove(model: string): Promise<void> {
    await this.ipc.priceRemove(model);
    this._prices.update((list) => list.filter((p) => p.model !== model));
  }
}
