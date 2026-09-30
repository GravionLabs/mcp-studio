import { Injectable, signal } from "@angular/core";

export type ToastKind = "info" | "success" | "error";

export interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
  /** Extra technical detail, shown on demand. */
  detail?: string;
}

const LIFETIME_MS: Record<ToastKind, number> = { info: 4000, success: 3000, error: 10000 };

/** Transient notifications. Errors linger longer and can carry details. */
@Injectable({ providedIn: "root" })
export class ToastService {
  private nextId = 1;
  private readonly _toasts = signal<Toast[]>([]);
  readonly toasts = this._toasts.asReadonly();

  info(message: string): number {
    return this.push("info", message);
  }

  success(message: string): number {
    return this.push("success", message);
  }

  error(message: string, detail?: string): number {
    return this.push("error", message, detail);
  }

  /** Reports an unknown thrown value as an error toast. */
  fail(context: string, error: unknown): number {
    const detail = error instanceof Error ? error.message : String(error);
    return this.error(context, detail);
  }

  dismiss(id: number): void {
    this._toasts.update((list) => list.filter((t) => t.id !== id));
  }

  private push(kind: ToastKind, message: string, detail?: string): number {
    const id = this.nextId++;
    this._toasts.update((list) => [...list, { id, kind, message, detail }]);
    setTimeout(() => this.dismiss(id), LIFETIME_MS[kind]);
    return id;
  }
}
