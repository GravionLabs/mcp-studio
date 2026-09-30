import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  signal,
} from "@angular/core";
import { Router } from "@angular/router";
import type { HistoryEntry } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { JsonViewComponent } from "../../ui/json-view/json-view.component";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { EnvironmentsStore } from "../environments/environments.store";
import { ResultViewComponent } from "../results/result-view.component";
import { parseToolResult } from "../results/result.model";
import { ServersStore } from "../servers/servers.store";
import { formatDuration, formatTimestamp, promptArguments, summarize } from "./history.model";
import { HistoryStore } from "./history.store";

/** Everything that was called from MCP Studio, searchable, with rerun. */
@Component({
  selector: "app-history-page",
  imports: [ResultViewComponent, JsonViewComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./history-page.component.html",
  styleUrl: "./history-page.component.scss",
})
export class HistoryPageComponent implements OnInit {
  protected readonly history = inject(HistoryStore);
  protected readonly servers = inject(ServersStore);
  private readonly ipc = inject(TauriIpcService);
  private readonly environments = inject(EnvironmentsStore);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly router = inject(Router);
  private readonly tabs = inject(WorkspaceTabsService);

  protected readonly serverFilter = signal("");
  protected readonly search = signal("");
  protected readonly expandedId = signal<number | null>(null);
  protected readonly busyId = signal<number | null>(null);

  protected readonly rows = computed(() =>
    this.history.entries().map((entry) => ({
      entry,
      summary: summarize(entry),
      server: this.servers.byId().get(entry.serverId)?.name ?? "deleted server",
      when: formatTimestamp(entry.ts),
      duration: formatDuration(entry.durationMs),
    })),
  );

  ngOnInit(): void {
    this.tabs.open({ id: "history", title: "History", route: "/history" });
    void this.refresh();
  }

  protected async refresh(): Promise<void> {
    try {
      await this.history.load({
        serverId: this.serverFilter() || null,
        search: this.search() || null,
        limit: 200,
      });
    } catch (error) {
      this.toasts.fail("Could not load the history", error);
    }
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected setServer(value: string): void {
    this.serverFilter.set(value);
    void this.refresh();
  }

  protected setSearch(value: string): void {
    this.search.set(value);
    void this.refresh();
  }

  protected toggle(entry: HistoryEntry): void {
    this.expandedId.update((id) => (id === entry.id ? null : entry.id));
  }

  protected toolResult(entry: HistoryEntry) {
    return entry.method === "tools/call" && entry.result != null
      ? parseToolResult(entry.result)
      : null;
  }

  protected openInPlayground(entry: HistoryEntry): void {
    if (entry.method !== "tools/call") return;
    void this.router.navigate(["/servers", entry.serverId, "tools", entry.target], {
      queryParams: { history: entry.id },
    });
  }

  /** Sends the same request again and reports the outcome; the new run shows up at the top. */
  protected async rerun(entry: HistoryEntry): Promise<void> {
    this.busyId.set(entry.id);
    try {
      const started = performance.now();
      if (entry.method === "tools/call") {
        const done = await this.ipc.toolCall({
          serverId: entry.serverId,
          toolName: entry.target,
          arguments: entry.arguments,
          environmentId: this.environments.activeId(),
          callId: crypto.randomUUID(),
        });
        this.toasts[done.isError ? "error" : "success"](
          `${entry.target}: ${done.isError ? "tool error" : "ok"} (${formatDuration(done.durationMs)})`,
        );
      } else if (entry.method === "resources/read") {
        await this.ipc.resourceRead(entry.serverId, entry.target);
        this.toasts.success(
          `Read ${entry.target} (${formatDuration(Math.round(performance.now() - started))})`,
        );
      } else if (entry.method === "prompts/get") {
        await this.ipc.promptGet(entry.serverId, entry.target, promptArguments(entry));
        this.toasts.success(`Got prompt ${entry.target}`);
      }
    } catch (error) {
      this.toasts.fail(`Could not rerun ${entry.target}`, error);
    } finally {
      this.busyId.set(null);
      await this.refresh();
    }
  }

  protected async clear(): Promise<void> {
    const serverId = this.serverFilter() || null;
    const scope = serverId ? "of this server" : "of all servers";
    if (
      !(await this.dialogs.confirm(`Delete the history ${scope}?`, {
        confirmLabel: "Delete",
        danger: true,
      }))
    ) {
      return;
    }
    try {
      const removed = await this.history.clear(serverId);
      this.toasts.success(`Deleted ${removed} entr${removed === 1 ? "y" : "ies"}`);
    } catch (error) {
      this.toasts.fail("Could not clear the history", error);
    }
  }
}
