import {
  ChangeDetectionStrategy,
  Component,
  OnDestroy,
  computed,
  effect,
  inject,
  signal,
} from "@angular/core";
import type { EvalEvent, TestSuite, Variant, VariantResult } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { ServersStore } from "../servers/servers.store";
import {
  BASELINE,
  callCount,
  describeVariant,
  failingOf,
  formatPercent,
  matrix,
  progressText,
  summarize,
} from "./compare.model";

/** Compares prompt and tool description variants on a test suite, side by side. */
@Component({
  selector: "app-compare-page",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./compare-page.html",
  styleUrl: "./compare-page.scss",
})
export class ComparePage implements OnDestroy {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly tabs = inject(WorkspaceTabsService);
  protected readonly servers = inject(ServersStore);

  protected readonly serverId = signal<string | null>(null);
  protected readonly suites = signal<TestSuite[]>([]);
  protected readonly suiteId = signal<string | null>(null);
  protected readonly model = signal("claude-sonnet-5-5");
  protected readonly variants = signal<Variant[]>([BASELINE]);
  protected readonly results = signal<VariantResult[]>([]);
  protected readonly runId = signal<string | null>(null);
  protected readonly done = signal(0);
  protected readonly proposing = signal(false);
  protected readonly suite = computed(() => this.suites().find((s) => s.id === this.suiteId()));
  protected readonly running = computed(() => this.runId() !== null);
  protected readonly calls = computed(() => {
    const suite = this.suite();
    return suite ? callCount(suite, this.variants()) : 0;
  });
  protected readonly summary = computed(() => summarize(this.results()));
  protected readonly matrix = computed(() => matrix(this.results()));
  protected readonly progress = computed(() => progressText(this.done(), this.calls()));
  protected readonly percent = formatPercent;
  protected readonly describe = describeVariant;

  private unlisten: (() => void) | undefined;

  constructor() {
    this.tabs.open({ id: "compare", title: "Compare variants", route: "/compare" });
    effect(() => {
      const first = this.servers.servers()[0];
      if (this.serverId() === null && first) void this.chooseServer(first.id);
    });
    void this.ipc
      .listen<EvalEvent>("eval://event", (event) => this.onEvent(event))
      .then((fn) => {
        this.unlisten = fn;
      });
  }

  ngOnDestroy(): void {
    this.unlisten?.();
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected async chooseServer(id: string): Promise<void> {
    this.serverId.set(id);
    this.reset();
    try {
      const suites = await this.ipc.testSuiteList(id);
      this.suites.set(suites);
      this.suiteId.set(suites[0]?.id ?? null);
    } catch (error) {
      this.toasts.fail("Could not load the suites", error);
    }
  }

  protected chooseSuite(id: string): void {
    this.suiteId.set(id);
    this.reset();
  }

  private reset(): void {
    this.variants.set([BASELINE]);
    this.results.set([]);
  }

  private onEvent(event: EvalEvent): void {
    if (event.runId !== this.runId()) return;
    if (event.type === "case_done") this.done.update((n) => n + 1);
    else if (event.type === "variant_done") {
      this.results.update((list) => [
        ...list.filter((r) => r.variant.id !== event.result.variant.id),
        event.result,
      ]);
    } else {
      this.runId.set(null);
      if (event.error) this.toasts.fail("The comparison failed", event.error);
    }
  }

  protected async run(): Promise<void> {
    const suiteId = this.suiteId();
    if (suiteId === null || this.running()) return;
    const runId = crypto.randomUUID();
    this.results.set([]);
    this.done.set(0);
    this.runId.set(runId);
    try {
      await this.ipc.variantsRun({
        runId,
        suiteId,
        model: this.model(),
        variants: this.variants(),
        environmentId: null,
      });
    } catch (error) {
      this.runId.set(null);
      this.toasts.fail("Could not start the comparison", error);
    }
  }

  protected async cancel(): Promise<void> {
    const id = this.runId();
    if (id) await this.ipc.flowRunCancel(id);
  }

  protected async propose(): Promise<void> {
    const suiteId = this.suiteId();
    if (suiteId === null || this.proposing()) return;
    const baseline = this.results().find((r) => r.variant.id === BASELINE.id);
    this.proposing.set(true);
    try {
      const proposed = await this.ipc.variantsPropose({
        suiteId,
        model: this.model(),
        count: 3,
        failing: baseline ? failingOf(baseline) : [],
        environmentId: null,
      });
      this.variants.set([BASELINE, ...proposed]);
      if (proposed.length === 0)
        this.toasts.fail("No variants", "The model proposed nothing usable.");
    } catch (error) {
      this.toasts.fail("Could not propose variants", error);
    } finally {
      this.proposing.set(false);
    }
  }

  protected async copy(variant: Variant): Promise<void> {
    try {
      await navigator.clipboard.writeText(describeVariant(variant));
      this.toasts.success(`Copied ${variant.label}`);
    } catch (error) {
      this.toasts.fail("Could not copy", error);
    }
  }
}
