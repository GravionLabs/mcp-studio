import { ChangeDetectionStrategy, Component, OnDestroy, inject, signal } from "@angular/core";
import type { ConfirmEvent, Decision, RunEvent } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { describeArguments, describeCall } from "./run.model";

/**
 * Asks before a flow calls a tool. Nothing runs without consent: the run waits here until the user
 * decides, and a dropped question counts as no.
 */
@Component({
  selector: "app-flow-confirmations",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./flow-confirmations.component.html",
  styleUrl: "./flow-confirmations.component.scss",
})
export class FlowConfirmationsComponent implements OnDestroy {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly stops: (() => void)[] = [];
  private destroyed = false;

  protected readonly queue = signal<ConfirmEvent[]>([]);
  protected describeCall = describeCall;
  protected describeArguments = describeArguments;

  constructor() {
    this.subscribe<ConfirmEvent>("flow://confirm", (event) =>
      this.queue.update((list) => [...list, event]),
    );
    // A run that has ended no longer waits for an answer.
    this.subscribe<RunEvent>("flow://event", (event) => {
      if (event.type === "run_finished") {
        this.queue.update((list) => list.filter((q) => q.request.runId !== event.runId));
      }
    });
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    this.stops.forEach((stop) => stop());
  }

  private subscribe<T>(event: string, handler: (payload: T) => void): void {
    void this.ipc.listen<T>(event, handler).then((stop) => {
      if (this.destroyed) stop();
      else this.stops.push(stop);
    });
  }

  protected async answer(event: ConfirmEvent, decision: Decision): Promise<void> {
    this.queue.update((list) => list.filter((q) => q.id !== event.id));
    try {
      await this.ipc.flowConfirm(event.id, decision);
    } catch (error) {
      this.toasts.fail("Could not send the answer", error);
    }
  }
}
