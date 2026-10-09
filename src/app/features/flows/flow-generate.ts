import {
  ChangeDetectionStrategy,
  Component,
  computed,
  inject,
  output,
  signal,
} from "@angular/core";
import { Router } from "@angular/router";
import type { GeneratedFlow } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { ServersStore } from "../servers/servers.store";
import { blocker, issueLines, outcome, toggled } from "./generate.model";

/**
 * Asks a model for a flow that reaches a goal. The flow is saved and opened in the editor only when
 * it passed validation, and it is never run from here.
 */
@Component({
  selector: "app-flow-generate",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./flow-generate.html",
  styleUrl: "./flow-generate.scss",
})
export class FlowGenerate {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly router = inject(Router);
  protected readonly servers = inject(ServersStore);

  /** Emitted after a flow was saved, so the library can reload. */
  readonly saved = output<void>();

  protected readonly goal = signal("");
  protected readonly model = signal("claude-sonnet-5-5");
  protected readonly serverIds = signal<string[]>([]);
  protected readonly busy = signal(false);
  protected readonly failed = signal<GeneratedFlow | null>(null);
  protected readonly blocked = computed(() => blocker(this.goal(), this.serverIds()));
  protected readonly lines = computed(() => issueLines(this.failed()?.issues ?? []));
  protected readonly summary = computed(() => {
    const failed = this.failed();
    return failed ? outcome(failed) : "";
  });

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLTextAreaElement).value;
  }

  protected toggle(id: string): void {
    this.serverIds.update((ids) => toggled(ids, id));
  }

  protected async generate(): Promise<void> {
    if (this.blocked() !== null || this.busy()) return;
    this.busy.set(true);
    this.failed.set(null);
    try {
      const generated = await this.ipc.flowGenerate({
        goal: this.goal(),
        model: this.model(),
        serverIds: this.serverIds(),
        environmentId: null,
      });
      if (generated.flow && generated.issues.length === 0) {
        const record = await this.ipc.flowSave(null, generated.flow);
        this.saved.emit();
        this.toasts.success(outcome(generated));
        await this.router.navigate(["/flows", record.id, "edit"]);
      } else {
        this.failed.set(generated);
      }
    } catch (error) {
      this.toasts.fail("Could not generate a flow", error);
    } finally {
      this.busy.set(false);
    }
  }
}
