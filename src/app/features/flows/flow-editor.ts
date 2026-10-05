import {
  ChangeDetectionStrategy,
  Component,
  OnDestroy,
  OnInit,
  computed,
  inject,
  input,
  signal,
  viewChild,
} from "@angular/core";
import { RouterLink } from "@angular/router";
import { FCanvasComponent, FFlowModule } from "@foblex/flow";
import type { Flow, FlowIssue, FlowValidation, Step } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import {
  Branch,
  StepType,
  addStep,
  connectJump,
  connectionsOf,
  inConnector,
  jumpTargets,
  layoutOf,
  moveStep,
  outConnector,
  parseConnector,
  placeAfter,
  removeStep,
  renameStep,
  summarize,
  updateStep,
} from "./editor.model";
import { STEP_TYPES } from "./flows.model";
import { StringMapEditor } from "./string-map-editor";

/** Edits a flow as a graph, with the same flow as YAML next to it. Both views edit one flow. */
@Component({
  selector: "app-flow-editor",
  imports: [FFlowModule, RouterLink, StringMapEditor],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./flow-editor.html",
  styleUrl: "./flow-editor.scss",
})
export class FlowEditor implements OnInit, OnDestroy {
  /** The flow in the library, from the route. */
  readonly id = input.required<string>();

  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly tabs = inject(WorkspaceTabsService);
  private readonly canvas = viewChild(FCanvasComponent);
  private timers: ReturnType<typeof setTimeout>[] = [];
  private typingYaml = false;

  protected readonly flow = signal<Flow | null>(null);
  private readonly saved = signal("");
  protected readonly dirty = computed(() => {
    const flow = this.flow();
    return flow !== null && JSON.stringify(flow) !== this.saved();
  });
  protected readonly selectedId = signal<string | null>(null);
  protected readonly selected = computed(
    () => this.flow()?.steps.find((s) => s.id === this.selectedId()) ?? null,
  );
  protected readonly moved = signal<ReadonlyMap<string, { x: number; y: number }>>(new Map());
  protected readonly positions = computed(() => {
    const flow = this.flow();
    const base = flow ? layoutOf(flow) : new Map<string, { x: number; y: number }>();
    for (const [id, point] of this.moved()) if (base.has(id)) base.set(id, point);
    return base;
  });
  protected readonly connections = computed(() => {
    const flow = this.flow();
    return flow ? connectionsOf(flow) : [];
  });
  protected readonly validation = signal<FlowValidation | null>(null);
  protected readonly issuesByStep = computed(() => {
    const byStep = new Map<string, FlowIssue[]>();
    for (const issue of this.validation()?.issues ?? []) {
      if (issue.stepId === null) continue;
      byStep.set(issue.stepId, [...(byStep.get(issue.stepId) ?? []), issue]);
    }
    return byStep;
  });
  protected readonly tab = signal<"form" | "yaml">("form");
  protected readonly yamlText = signal("");
  protected readonly yamlError = signal<string | null>(null);
  protected readonly argumentsError = signal<string | null>(null);

  protected readonly types = STEP_TYPES;
  protected readonly inConnector = inConnector;
  protected readonly outConnector = outConnector;
  protected readonly summarize = summarize;
  protected readonly inputKinds = ["string", "number", "integer", "boolean", "array", "object"];

  ngOnInit(): void {
    this.ipc.flowGet(this.id()).then(
      (record) => {
        this.flow.set(record.flow);
        this.saved.set(JSON.stringify(record.flow));
        this.tabs.open({
          id: `flow-${record.id}`,
          title: `Edit ${record.flow.name}`,
          route: `/flows/${record.id}/edit`,
        });
        this.afterChange(true);
      },
      (error: unknown) => this.toasts.fail("Could not open the flow", error),
    );
  }

  ngOnDestroy(): void {
    this.timers.forEach(clearTimeout);
  }

  // ---- one flow, two views ----------------------------------------------------------------

  /** Every edit of the graph or the form goes through here. */
  protected commit(next: Flow | null): void {
    if (next === null) return;
    this.flow.set(next);
    this.afterChange(false);
  }

  private later(key: number, run: () => void, ms: number): void {
    clearTimeout(this.timers[key]);
    this.timers[key] = setTimeout(run, ms);
  }

  private afterChange(immediately: boolean): void {
    const wait = immediately ? 0 : 350;
    this.later(0, () => void this.validate(), wait);
    if (!this.typingYaml) this.later(1, () => void this.refreshYaml(), immediately ? 0 : 150);
  }

  private async validate(): Promise<void> {
    const flow = this.flow();
    if (!flow) return;
    try {
      this.validation.set(await this.ipc.flowValidate(flow));
    } catch {
      this.validation.set(null);
    }
  }

  private async refreshYaml(): Promise<void> {
    const flow = this.flow();
    if (!flow || this.typingYaml) return;
    try {
      this.yamlText.set(await this.ipc.flowToYaml(flow));
      this.yamlError.set(null);
    } catch (error) {
      this.yamlError.set(error instanceof Error ? error.message : String(error));
    }
  }

  /** The YAML text was edited: read it, and when it is valid the graph follows. */
  protected yamlEdited(event: Event): void {
    const text = (event.target as HTMLTextAreaElement).value;
    this.yamlText.set(text);
    this.typingYaml = true;
    this.later(2, () => void this.readYaml(text), 300);
  }

  private async readYaml(text: string): Promise<void> {
    try {
      const flow = await this.ipc.flowFromYaml(text);
      this.yamlError.set(null);
      this.flow.set(flow);
      if (this.selectedId() !== null && !flow.steps.some((s) => s.id === this.selectedId())) {
        this.selectedId.set(null);
      }
      this.later(0, () => void this.validate(), 350);
    } catch (error) {
      this.yamlError.set(error instanceof Error ? error.message : String(error));
    }
  }

  /** Leaving the YAML text: write it out in the normal form again. */
  protected yamlLeft(): void {
    this.typingYaml = false;
    if (this.yamlError() === null) void this.refreshYaml();
  }

  // ---- saving -----------------------------------------------------------------------------

  protected async save(): Promise<void> {
    const flow = this.flow();
    if (!flow) return;
    try {
      const record = await this.ipc.flowSave(this.id(), flow);
      this.flow.set(record.flow);
      this.saved.set(JSON.stringify(record.flow));
      this.toasts.success("Saved");
    } catch (error) {
      this.toasts.fail("Could not save the flow", error);
    }
  }

  protected rename(event: Event): void {
    const flow = this.flow();
    if (flow) this.commit({ ...flow, name: (event.target as HTMLInputElement).value });
  }

  // ---- the graph --------------------------------------------------------------------------

  /** Fits the picture into the canvas once the nodes are drawn. */
  protected rendered(): void {
    this.canvas()?.resetScaleAndCenter(false);
  }

  protected select(stepId: string): void {
    this.selectedId.set(stepId);
    this.argumentsError.set(null);
  }

  protected dragged(stepId: string, point: { x: number; y: number }): void {
    this.moved.update((m) => new Map(m).set(stepId, { x: point.x, y: point.y }));
  }

  protected add(type: StepType): void {
    const flow = this.flow();
    if (!flow) return;
    const next = addStep(flow, type, this.selectedId());
    const added = next.steps.find((s) => !flow.steps.some((o) => o.id === s.id));
    this.commit(next);
    if (added) this.select(added.id);
  }

  protected remove(stepId: string): void {
    const flow = this.flow();
    if (!flow) return;
    this.commit(removeStep(flow, stepId));
    if (this.selectedId() === stepId) this.selectedId.set(null);
  }

  protected move(stepId: string, delta: -1 | 1): void {
    const flow = this.flow();
    if (flow) this.commit(moveStep(flow, stepId, delta));
  }

  /** Connecting two nodes by dragging from an output to an input. */
  protected connected(event: { sourceId: string; targetId: string | undefined }): void {
    const flow = this.flow();
    if (!flow || event.targetId === undefined) return;
    const from = parseConnector(event.sourceId);
    const to = parseConnector(event.targetId);
    if (from?.direction !== "out" || to?.direction !== "in") return;
    if (from.branch !== null) {
      const next = connectJump(flow, from.stepId, from.branch, to.stepId);
      if (next === null) {
        this.toasts.info("A condition can only jump to a step that comes after it");
        return;
      }
      this.commit(next);
    } else {
      // Steps run in order, so connecting one to another puts it right after.
      this.commit(placeAfter(flow, to.stepId, from.stepId));
    }
  }

  // ---- the form ---------------------------------------------------------------------------

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value;
  }

  protected setId(stepId: string, event: Event): void {
    const flow = this.flow();
    if (!flow) return;
    const next = this.text(event).trim();
    const renamed = renameStep(flow, stepId, next);
    if (renamed === flow) {
      if (next !== stepId)
        this.toasts.info(
          "Use letters, digits and _, starting with a letter, and not an id that is taken",
        );
      (event.target as HTMLInputElement).value = stepId;
      return;
    }
    this.commit(renamed);
    this.selectedId.set(next);
    this.moved.update((m) => {
      const copy = new Map(m);
      const point = copy.get(stepId);
      copy.delete(stepId);
      if (point) copy.set(next, point);
      return copy;
    });
  }

  protected set(stepId: string, patch: Record<string, unknown>): void {
    const flow = this.flow();
    if (flow) this.commit(updateStep(flow, stepId, patch));
  }

  protected setText(stepId: string, field: string, event: Event): void {
    this.set(stepId, { [field]: this.text(event) });
  }

  protected setOptionalText(stepId: string, field: string, event: Event): void {
    const value = this.text(event);
    this.set(stepId, { [field]: value === "" ? null : value });
  }

  protected setBranch(stepId: string, branch: Branch, event: Event): void {
    const flow = this.flow();
    if (!flow) return;
    const value = this.text(event);
    this.commit(connectJump(flow, stepId, branch, value === "" ? null : value));
  }

  protected targetsOf(stepId: string): string[] {
    const flow = this.flow();
    return flow ? jumpTargets(flow, stepId) : [];
  }

  protected argumentsText(step: Extract<Step, { type: "tool" }>): string {
    const values = Object.fromEntries(Object.entries(step.arguments ?? {}));
    return Object.keys(values).length === 0 ? "{}" : JSON.stringify(values, null, 2);
  }

  protected setArguments(stepId: string, event: Event): void {
    try {
      const parsed: unknown = JSON.parse(this.text(event));
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("The arguments must be a JSON object");
      }
      this.argumentsError.set(null);
      this.set(stepId, { arguments: parsed });
    } catch (error) {
      this.argumentsError.set(error instanceof Error ? error.message : "Not valid JSON");
    }
  }

  protected setInputDecl(
    step: Extract<Step, { type: "input" }>,
    name: string,
    patch: { type?: string; description?: string | null },
  ): void {
    const current = step.inputs?.[name] ?? { type: "string" };
    this.set(step.id, {
      inputs: { ...step.inputs, [name]: { ...current, ...patch } },
    });
  }

  protected renameInput(step: Extract<Step, { type: "input" }>, from: string, event: Event): void {
    const to = this.text(event).trim();
    const inputs = step.inputs ?? {};
    if (to === "" || to === from || to in inputs) return;
    this.set(step.id, {
      inputs: Object.fromEntries(Object.entries(inputs).map(([k, v]) => [k === from ? to : k, v])),
    });
  }

  protected addInput(step: Extract<Step, { type: "input" }>): void {
    const inputs = step.inputs ?? {};
    let n = Object.keys(inputs).length + 1;
    while (`input${n}` in inputs) n += 1;
    this.set(step.id, { inputs: { ...inputs, [`input${n}`]: { type: "string" } } });
  }

  protected removeInput(step: Extract<Step, { type: "input" }>, name: string): void {
    this.set(step.id, {
      inputs: Object.fromEntries(Object.entries(step.inputs ?? {}).filter(([k]) => k !== name)),
    });
  }

  protected inputEntries(step: Extract<Step, { type: "input" }>) {
    return Object.entries(step.inputs ?? {}).map(([name, decl]) => ({ name, ...decl }));
  }

  protected addTool(step: Extract<Step, { type: "llm" }>): void {
    this.set(step.id, { tools: [...(step.tools ?? []), { server: "", tool: "" }] });
  }

  protected setTool(
    step: Extract<Step, { type: "llm" }>,
    index: number,
    field: "server" | "tool",
    event: Event,
  ): void {
    const tools = (step.tools ?? []).map((t, i) =>
      i === index ? { ...t, [field]: this.text(event) } : t,
    );
    this.set(step.id, { tools });
  }

  protected removeTool(step: Extract<Step, { type: "llm" }>, index: number): void {
    this.set(step.id, { tools: (step.tools ?? []).filter((_, i) => i !== index) });
  }

  protected issuesOf(stepId: string): FlowIssue[] {
    return this.issuesByStep().get(stepId) ?? [];
  }

  protected flowIssues(): FlowIssue[] {
    return (this.validation()?.issues ?? []).filter((i) => i.stepId === null);
  }
}
