import { ChangeDetectionStrategy, Component, effect, inject, input, signal } from "@angular/core";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";

/** The log levels of MCP, least to most severe. */
export const LOG_LEVELS = [
  "debug",
  "info",
  "notice",
  "warning",
  "error",
  "critical",
  "alert",
  "emergency",
] as const;

/**
 * Lets the user choose how much a connected server logs (`logging/setLevel`). Shown only when the
 * server advertises the `logging` capability.
 */
@Component({
  selector: "app-log-level",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./log-level.html",
  styleUrl: "./log-level.scss",
})
export class LogLevel {
  readonly serverId = input.required<string>();

  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);

  protected readonly levels = LOG_LEVELS;
  protected readonly supported = signal(false);
  /** The level sent in this session, or `null` when none was sent (the server's default applies). */
  protected readonly current = signal<string | null>(null);

  constructor() {
    effect(() => {
      const id = this.serverId();
      this.supported.set(false);
      this.current.set(null);
      this.ipc.serverDetails(id).then(
        (details) => this.supported.set(details.hasLogging),
        () => undefined,
      );
      this.ipc.sessionState(id).then(
        (state) => this.current.set(state.logLevel),
        () => undefined,
      );
    });
  }

  protected async choose(event: Event): Promise<void> {
    const level = (event.target as HTMLSelectElement).value;
    if (level === "") return;
    try {
      await this.ipc.serverSetLogLevel(this.serverId(), level);
      this.current.set(level);
    } catch (error) {
      this.toasts.fail("Could not set the log level", error);
    }
  }
}
