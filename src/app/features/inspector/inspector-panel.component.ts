import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  effect,
  inject,
  signal,
  viewChild,
} from "@angular/core";
import type { MessageRecord, SessionUsage } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { JsonDiffComponent } from "../../ui/json-diff/json-diff.component";
import { JsonViewComponent } from "../../ui/json-view/json-view.component";
import { VirtualListComponent } from "../../ui/virtual-list/virtual-list.component";
import { ServersStore } from "../servers/servers.store";
import {
  formatBytes,
  formatClock,
  formatTokens,
  labelFor,
  prettyPayload,
  tokenSourceHint,
} from "./inspector.model";
import { InspectorStore } from "./inspector.store";
import { callCost, formatCost, messageCost } from "../prices/prices.model";
import { PricesStore } from "../prices/prices.store";

const METHODS = [
  "initialize",
  "tools/list",
  "tools/call",
  "resources/list",
  "resources/read",
  "prompts/list",
  "prompts/get",
  "ping",
];

/** Right column: every JSON-RPC message of every session, live. */
@Component({
  selector: "app-inspector-panel",
  imports: [VirtualListComponent, JsonViewComponent, JsonDiffComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./inspector-panel.component.html",
  styleUrl: "./inspector-panel.component.scss",
})
export class InspectorPanelComponent implements OnInit {
  protected readonly store = inject(InspectorStore);
  protected readonly servers = inject(ServersStore);
  private readonly toasts = inject(ToastService);
  private readonly ipc = inject(TauriIpcService);
  private readonly prices = inject(PricesStore);
  private readonly list = viewChild(VirtualListComponent);

  protected readonly methods = METHODS;
  /** Totals of the selected message's session. */
  protected readonly usage = signal<SessionUsage | null>(null);
  protected readonly follow = signal(true);
  protected readonly compareOpen = signal(false);
  protected readonly ignoreId = signal(true);
  protected readonly compared = computed(() => {
    const pair = this.store.comparison();
    if (!pair) return null;
    const parse = (message: MessageRecord): unknown => {
      try {
        return JSON.parse(message.payload);
      } catch {
        return message.payload;
      }
    };
    return { left: pair[0], right: pair[1], leftJson: parse(pair[0]), rightJson: parse(pair[1]) };
  });
  protected readonly selectedIds = computed(() => new Set(this.store.selectedIds()));
  protected readonly detail = computed(() => {
    const message = this.store.selected();
    return message ? { message, text: prettyPayload(message.payload) } : null;
  });

  constructor() {
    void this.prices.load().catch(() => undefined);
    effect(() => {
      const selected = this.store.selected();
      const model = this.prices.activeModel();
      if (!selected) {
        this.usage.set(null);
        return;
      }
      this.ipc.sessionUsage(selected.sessionId, model).then(
        (usage) => this.usage.set(usage),
        () => this.usage.set(null),
      );
    });
  }

  ngOnInit(): void {
    this.store
      .startLive()
      .then(() => this.store.load())
      .catch((error: unknown) => this.toasts.fail("Could not load messages", error));
  }

  protected label(message: MessageRecord) {
    return labelFor(message, this.store.requestMethod(message));
  }

  protected clock = formatClock;
  protected bytes = formatBytes;
  protected tokens = formatTokens;
  protected tokenHint = tokenSourceHint;
  protected formatCost = formatCost;
  protected readonly counting = signal(false);

  /** Replaces the estimate with the provider's exact count. Sends the content to Anthropic. */
  protected async countExactly(message: MessageRecord): Promise<void> {
    this.counting.set(true);
    try {
      const tokens = await this.ipc.messageCountExact(message.id);
      this.store.applyExactCount(message.id, tokens);
    } catch (error) {
      this.toasts.fail("Could not count tokens exactly", error);
    } finally {
      this.counting.set(false);
    }
  }

  /** Cost of one message at the chosen model's price, as text. */
  protected cost(message: MessageRecord): string | null {
    const cost = messageCost(message, this.prices.activePrice());
    return cost ? formatCost(cost) : null;
  }

  /** Cost of the whole tool call a message belongs to (arguments and result), as text. */
  protected callCost(message: MessageRecord): string | null {
    if (this.store.requestMethod(message) !== "tools/call") return null;
    const cost = callCost(this.store.messages(), message, this.prices.activePrice());
    return cost ? formatCost(cost) : null;
  }

  protected totalTokens(usage: SessionUsage): number {
    return usage.argumentTokens + usage.resultTokens + usage.definitionTokens;
  }

  protected trackById = (message: MessageRecord) => message.id;

  protected serverName(id: string): string {
    return this.servers.byId().get(id)?.name ?? "";
  }

  protected value(event: Event): string {
    return (event.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected setFilter(partial: Parameters<InspectorStore["setFilter"]>[0]): void {
    this.store
      .setFilter(partial)
      .catch((error: unknown) => this.toasts.fail("Could not filter", error));
  }

  protected click(message: MessageRecord, event: MouseEvent): void {
    this.store.select(message.id, event.ctrlKey || event.metaKey || event.shiftKey);
  }

  protected closeCompare(): void {
    this.compareOpen.set(false);
  }

  protected onBottom(atBottom: boolean): void {
    this.follow.set(atBottom);
  }

  protected jumpToEnd(): void {
    this.follow.set(true);
    this.list()?.scrollToEnd();
  }

  protected async copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      this.toasts.success("Copied");
    } catch (error) {
      this.toasts.fail("Could not copy", error);
    }
  }
}
