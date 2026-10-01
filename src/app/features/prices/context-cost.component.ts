import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
} from "@angular/core";
import { RouterLink } from "@angular/router";
import type { ContextCost } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ExplorerStore } from "../explorer/explorer.store";
import { formatTokens, tokenSourceHint } from "../inspector/inspector.model";
import { formatCost } from "./prices.model";
import { PricesStore } from "./prices.store";

/** How much of a model's context the tool definitions of a connected server take. */
@Component({
  selector: "app-context-cost",
  imports: [RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./context-cost.component.html",
  styleUrl: "./context-cost.component.scss",
})
export class ContextCostComponent {
  readonly serverId = input.required<string>();

  private readonly ipc = inject(TauriIpcService);
  private readonly explorer = inject(ExplorerStore);
  protected readonly prices = inject(PricesStore);

  protected readonly cost = signal<ContextCost | null>(null);
  protected readonly failed = signal(false);
  protected readonly top = computed(() => this.cost()?.tools.slice(0, 5) ?? []);
  protected readonly more = computed(() => Math.max(0, (this.cost()?.tools.length ?? 0) - 5));
  protected formatTokens = formatTokens;
  protected hint = tokenSourceHint;
  protected formatCost = formatCost;

  constructor() {
    void this.prices.load().catch(() => undefined);
    effect(() => {
      const tools = this.explorer.snapshot(this.serverId()).tools;
      const model = this.prices.activeModel();
      this.ipc.toolsContextCost(tools, model).then(
        (cost) => {
          this.cost.set(cost);
          this.failed.set(false);
        },
        () => this.failed.set(true),
      );
    });
  }
}
