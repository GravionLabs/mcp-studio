import { Injectable, computed, inject, signal } from "@angular/core";
import type { UpdateInfo } from "./bindings";
import { describeError } from "./ipc-error";
import { TauriIpcService } from "./tauri-ipc.service";

export type UpdateState = "idle" | "checking" | "upToDate" | "available" | "installing" | "error";

/**
 * Checks for and installs application updates. Nothing is checked in the background: the network
 * request only happens when the user asks for it.
 */
@Injectable({ providedIn: "root" })
export class UpdateService {
  private readonly ipc = inject(TauriIpcService);

  readonly state = signal<UpdateState>("idle");
  readonly info = signal<UpdateInfo | null>(null);
  readonly error = signal<string | null>(null);
  readonly busy = computed(() => this.state() === "checking" || this.state() === "installing");

  async check(): Promise<void> {
    if (this.busy()) return;
    this.state.set("checking");
    this.error.set(null);
    try {
      const info = await this.ipc.updateCheck();
      this.info.set(info);
      this.state.set(info ? "available" : "upToDate");
    } catch (error) {
      this.fail(error);
    }
  }

  /** Installs the update found by {@link check}. On success the app restarts, so nothing follows. */
  async install(): Promise<void> {
    if (this.state() !== "available") return;
    this.state.set("installing");
    this.error.set(null);
    try {
      await this.ipc.updateInstall();
    } catch (error) {
      this.fail(error);
    }
  }

  private fail(error: unknown): void {
    this.error.set(describeError(error));
    this.state.set("error");
  }
}
