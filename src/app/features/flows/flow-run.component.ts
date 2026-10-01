import {
  ChangeDetectionStrategy,
  Component,
  OnDestroy,
  OnInit,
  computed,
  inject,
  input,
  signal,
} from "@angular/core";
import type { FlowRecord, FlowRun, RunEvent, RunSummary } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { JsonViewComponent } from "../../ui/json-view/json-view.component";
import { EnvironmentsStore } from "../environments/environments.store";
import {
  InputField,
  RunView,
  applyEvent,
  formatDuration,
  inputFields,
  parseInputs,
  startView,
  viewOf,
} from "./run.model";

/** Runs one flow: a form for its inputs, live progress, the result, and the earlier runs. */
@Component({
  selector: "app-flow-run",
  imports: [JsonViewComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./flow-run.component.html",
  styleUrl: "./flow-run.component.scss",
})
export class FlowRunComponent implements OnInit, OnDestroy {
  readonly record = input.required<FlowRecord>();

  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly environments = inject(EnvironmentsStore);
  private stop: (() => void) | null = null;
  private destroyed = false;

  protected readonly fields = computed<InputField[]>(() => inputFields(this.record().flow));
  protected readonly raw = signal<Record<string, string | boolean>>({});
  protected readonly errors = signal<Record<string, string>>({});
  protected readonly view = signal<RunView | null>(null);
  /** The stored run, once it has finished: outputs and the details of every step. */
  protected readonly stored = signal<FlowRun | null>(null);
  protected readonly history = signal<RunSummary[]>([]);
  protected readonly running = computed(() => this.view()?.status === "running");
  protected readonly selectedStep = signal<string | null>(null);
  protected readonly detail = computed(() => {
    const run = this.stored();
    const id = this.selectedStep();
    const step = run?.steps.find((s) => s.stepId === id);
    return step
      ? {
          resolved: JSON.stringify(step.resolved ?? null, null, 2),
          output: JSON.stringify(step.output ?? null, null, 2),
        }
      : null;
  });
  protected readonly outputs = computed(() => {
    const run = this.stored();
    return run?.outputs ? JSON.stringify(run.outputs, null, 2) : null;
  });
  protected formatDuration = formatDuration;

  ngOnInit(): void {
    void this.ipc
      .listen<RunEvent>("flow://event", (event) => this.onEvent(event))
      .then((stop) => {
        if (this.destroyed) stop();
        else this.stop = stop;
      });
    void this.loadHistory();
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    this.stop?.();
  }

  protected setText(name: string, event: Event): void {
    this.raw.update((r) => ({ ...r, [name]: (event.target as HTMLInputElement).value }));
  }

  protected setChecked(name: string, event: Event): void {
    this.raw.update((r) => ({ ...r, [name]: (event.target as HTMLInputElement).checked }));
  }

  protected async start(): Promise<void> {
    const parsed = parseInputs(this.fields(), this.raw());
    this.errors.set(parsed.errors);
    if (Object.keys(parsed.errors).length > 0) return;
    const runId = crypto.randomUUID();
    this.view.set(startView(runId, this.record().flow.name));
    this.stored.set(null);
    this.selectedStep.set(null);
    try {
      await this.ipc.flowRunStart({
        runId,
        flowId: this.record().id,
        flow: null,
        inputs: parsed.values,
        environmentId: this.environments.activeId(),
      });
    } catch (error) {
      this.view.set(null);
      this.toasts.fail("Could not start the run", error);
    }
  }

  protected async cancel(): Promise<void> {
    const view = this.view();
    if (!view) return;
    try {
      await this.ipc.flowRunCancel(view.runId);
    } catch (error) {
      this.toasts.fail("Could not cancel the run", error);
    }
  }

  private onEvent(event: RunEvent): void {
    const view = this.view();
    if (!view || event.runId !== view.runId) return;
    this.view.set(applyEvent(view, event));
    if (event.type === "run_finished") void this.finished(event.runId);
  }

  private async finished(runId: string): Promise<void> {
    try {
      this.stored.set(await this.ipc.flowRunGet(runId));
      await this.loadHistory();
    } catch (error) {
      this.toasts.fail("Could not load the finished run", error);
    }
  }

  protected async loadHistory(): Promise<void> {
    try {
      this.history.set(await this.ipc.flowRunList(this.record().id, 20));
    } catch (error) {
      this.toasts.fail("Could not load the earlier runs", error);
    }
  }

  protected async open(summary: RunSummary): Promise<void> {
    try {
      const run = await this.ipc.flowRunGet(summary.id);
      this.view.set(viewOf(run));
      this.stored.set(run);
      this.selectedStep.set(null);
    } catch (error) {
      this.toasts.fail("Could not open the run", error);
    }
  }

  protected async remove(summary: RunSummary): Promise<void> {
    const confirmed = await this.dialogs.confirm("Delete this run?", {
      confirmLabel: "Delete",
      danger: true,
    });
    if (!confirmed) return;
    try {
      await this.ipc.flowRunDelete(summary.id);
      if (this.view()?.runId === summary.id) {
        this.view.set(null);
        this.stored.set(null);
      }
      await this.loadHistory();
    } catch (error) {
      this.toasts.fail("Could not delete the run", error);
    }
  }

  protected durationOf(stepId: string): string {
    const step = this.stored()?.steps.find((s) => s.stepId === stepId);
    return step ? formatDuration(step.startedAt, step.endedAt) : "";
  }

  protected when(ts: number): string {
    return new Date(ts).toLocaleString();
  }
}
