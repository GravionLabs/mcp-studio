import { Injectable, computed, signal } from "@angular/core";

export interface DialogRequest {
  kind: "confirm" | "prompt";
  message: string;
  /** Initial text of a prompt. */
  initial: string;
  confirmLabel: string;
  danger: boolean;
  resolve: (value: boolean | string | null) => void;
}

/**
 * In-app confirm and prompt dialogs. Native `window.confirm` / `window.prompt` are unreliable in
 * desktop webviews, so the app renders its own (see DialogHost). Requests are queued.
 */
@Injectable({ providedIn: "root" })
export class DialogService {
  private readonly queue = signal<DialogRequest[]>([]);
  readonly current = computed(() => this.queue()[0] ?? null);

  confirm(
    message: string,
    options: { confirmLabel?: string; danger?: boolean } = {},
  ): Promise<boolean> {
    return new Promise((resolve) =>
      this.enqueue({
        kind: "confirm",
        message,
        initial: "",
        confirmLabel: options.confirmLabel ?? "OK",
        danger: options.danger ?? false,
        resolve: (value) => resolve(value === true),
      }),
    );
  }

  /** Resolves to the trimmed text, or null when cancelled or left empty. */
  prompt(message: string, initial = "", confirmLabel = "OK"): Promise<string | null> {
    return new Promise((resolve) =>
      this.enqueue({
        kind: "prompt",
        message,
        initial,
        confirmLabel,
        danger: false,
        resolve: (value) => {
          const text = typeof value === "string" ? value.trim() : "";
          resolve(text === "" ? null : text);
        },
      }),
    );
  }

  /** Answers the dialog currently shown: `true`/text to confirm, `false`/`null` to cancel. */
  respond(value: boolean | string | null): void {
    const first = this.queue()[0];
    if (!first) return;
    this.queue.update((q) => q.slice(1));
    first.resolve(value);
  }

  private enqueue(request: DialogRequest): void {
    this.queue.update((q) => [...q, request]);
  }
}
