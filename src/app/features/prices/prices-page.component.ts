import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from "@angular/core";
import { DialogService } from "../../core/dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { RouterLink } from "@angular/router";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { PriceDraft, draftToPrice, emptyDraft, toDraft } from "./prices.model";
import { PricesStore } from "./prices.store";

type NumberField = "input" | "output" | "cacheRead" | "cacheWrite";

/** Edit the price per million tokens of each model; costs in the app use the model marked "Use". */
@Component({
  selector: "app-prices-page",
  imports: [RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./prices-page.component.html",
  styleUrl: "./prices-page.component.scss",
})
export class PricesPageComponent implements OnInit {
  protected readonly store = inject(PricesStore);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly tabs = inject(WorkspaceTabsService);
  private readonly ipc = inject(TauriIpcService);

  /** Exact token counts through Anthropic's token counting endpoint. */
  protected readonly countingModel = signal("");
  protected readonly hasKey = signal(false);

  protected readonly drafts = signal<PriceDraft[]>([emptyDraft()]);
  protected readonly fields: { key: NumberField; label: string }[] = [
    { key: "input", label: "Input" },
    { key: "output", label: "Output" },
    { key: "cacheRead", label: "Cache read" },
    { key: "cacheWrite", label: "Cache write" },
  ];

  constructor() {
    this.tabs.open({ id: "prices", title: "Prices", route: "/prices" });
  }

  ngOnInit(): void {
    this.ipc.tokenCountingStatus().then(
      (status) => {
        this.countingModel.set(status.model);
        this.hasKey.set(status.hasKey);
      },
      (error: unknown) => this.toasts.fail("Could not load the token counting settings", error),
    );
    this.store.load().then(
      () => this.drafts.set([...this.store.prices().map(toDraft), emptyDraft()]),
      (error: unknown) => this.toasts.fail("Could not load prices", error),
    );
  }

  protected async saveCounting(): Promise<void> {
    try {
      await this.ipc.tokenCountingSetModel(this.countingModel());
      this.toasts.success("Saved the model for exact token counts");
    } catch (error) {
      this.toasts.fail("Could not save the model", error);
    }
  }

  protected edit(index: number, patch: Partial<PriceDraft>): void {
    this.drafts.update((list) => list.map((d, i) => (i === index ? { ...d, ...patch } : d)));
  }

  protected editField(index: number, key: NumberField, event: Event): void {
    this.edit(index, { [key]: this.text(event) });
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }

  protected async save(index: number): Promise<void> {
    const draft = this.drafts()[index];
    if (!draft) return;
    const price = draftToPrice(draft);
    if (typeof price === "string") {
      this.toasts.fail("Could not save the price", price);
      return;
    }
    try {
      const saved = await this.store.save(price);
      this.drafts.set([...this.store.prices().map(toDraft), emptyDraft()]);
      this.toasts.success(`Saved ${saved.model}`);
    } catch (error) {
      this.toasts.fail("Could not save the price", error);
    }
  }

  protected async remove(index: number): Promise<void> {
    const draft = this.drafts()[index];
    if (!draft) return;
    if (draft.isNew) {
      this.drafts.update((list) => list.map((d, i) => (i === index ? emptyDraft() : d)));
      return;
    }
    const confirmed = await this.dialogs.confirm(`Delete the price of ${draft.model}?`, {
      confirmLabel: "Delete",
      danger: true,
    });
    if (!confirmed) return;
    try {
      await this.store.remove(draft.model);
      this.drafts.set([...this.store.prices().map(toDraft), emptyDraft()]);
      if (this.store.activeModel() === null) this.store.setActive(null);
    } catch (error) {
      this.toasts.fail("Could not delete the price", error);
    }
  }
}
