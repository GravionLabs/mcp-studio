import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  signal,
} from "@angular/core";
import type { Span } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { JsonViewComponent } from "../../ui/json-view/json-view.component";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { formatTokens, tokenSourceHint } from "../inspector/inspector.model";
import { TraceExportComponent } from "./trace-export.component";
import { buildWaterfall, formatDuration } from "./waterfall.model";

/** Sessions as traces: pick a session, see its calls as a waterfall of duration, tokens, and errors. */
@Component({
  selector: "app-traces-page",
  imports: [JsonViewComponent, TraceExportComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./traces-page.component.html",
  styleUrl: "./traces-page.component.scss",
})
export class TracesPageComponent implements OnInit {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly tabs = inject(WorkspaceTabsService);

  protected readonly traces = signal<Span[]>([]);
  protected readonly selectedTrace = signal<string | null>(null);
  protected readonly spans = signal<Span[]>([]);
  protected readonly selectedSpan = signal<string | null>(null);
  protected readonly loading = signal(false);

  protected readonly waterfall = computed(() => buildWaterfall(this.spans(), Date.now()));
  protected readonly detail = computed(() => {
    const id = this.selectedSpan();
    const span = this.spans().find((s) => s.id === id);
    return span ? { span, text: JSON.stringify(span.attributes, null, 2) } : null;
  });
  protected readonly root = computed(() => this.spans().find((s) => s.parentId === null) ?? null);

  protected formatDuration = formatDuration;
  protected formatTokens = formatTokens;
  protected hint = tokenSourceHint;

  constructor() {
    this.tabs.open({ id: "traces", title: "Traces", route: "/traces" });
  }

  ngOnInit(): void {
    void this.refresh();
  }

  protected async refresh(): Promise<void> {
    this.loading.set(true);
    try {
      // The backend returns the newest window oldest first; show the newest session on top.
      const roots = await this.ipc.spansQuery({
        traceId: null,
        serverId: null,
        rootsOnly: true,
        limit: 200,
      });
      this.traces.set([...roots].reverse());
      const current = this.selectedTrace();
      const keep = current !== null && roots.some((r) => r.traceId === current);
      const next = keep ? current : (this.traces()[0]?.traceId ?? null);
      await this.select(next);
    } catch (error) {
      this.toasts.fail("Could not load traces", error);
    } finally {
      this.loading.set(false);
    }
  }

  protected async select(traceId: string | null): Promise<void> {
    this.selectedTrace.set(traceId);
    this.selectedSpan.set(null);
    if (traceId === null) {
      this.spans.set([]);
      return;
    }
    try {
      this.spans.set(
        await this.ipc.spansQuery({ traceId, serverId: null, rootsOnly: false, limit: null }),
      );
    } catch (error) {
      this.toasts.fail("Could not load the trace", error);
    }
  }

  protected clock(ts: number): string {
    return new Date(ts).toLocaleString();
  }

  protected duration(span: Span): string {
    return formatDuration(span.endedAt === null ? null : span.endedAt - span.startedAt);
  }
}
