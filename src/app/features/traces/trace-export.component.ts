import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  signal,
} from "@angular/core";
import type { ExportStatus } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { KeyValueEditorComponent } from "../../ui/key-value-editor/key-value-editor.component";
import type { KeyValueRow } from "../../ui/key-value-editor/key-value-row";
import { SecretWrite, recordToRows, rowsToRecord } from "../../ui/key-value-editor/key-value-rows";
import { describeExport } from "./trace-export.model";

const NO_STATUS: ExportStatus = {
  lastAttemptAt: null,
  lastSuccessAt: null,
  lastError: null,
  exported: 0,
};

/** Optional export of spans to an OpenTelemetry collector (OTLP over HTTP). Off by default. */
@Component({
  selector: "app-trace-export",
  imports: [KeyValueEditorComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./trace-export.component.html",
  styleUrl: "./trace-export.component.scss",
})
export class TraceExportComponent implements OnInit {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);

  protected readonly enabled = signal(false);
  protected readonly endpoint = signal("");
  protected readonly rows = signal<KeyValueRow[]>([]);
  protected readonly status = signal<ExportStatus>(NO_STATUS);
  protected readonly busy = signal(false);
  protected readonly summary = computed(() => describeExport(this.enabled(), this.status()));

  ngOnInit(): void {
    Promise.all([this.ipc.traceExportConfig(), this.ipc.traceExportStatus()]).then(
      ([config, status]) => {
        this.enabled.set(config.enabled ?? false);
        this.endpoint.set(config.endpoint ?? "");
        this.rows.set(recordToRows(config.headers ?? {}));
        this.status.set(status);
      },
      (error: unknown) => this.toasts.fail("Could not load the export settings", error),
    );
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }

  protected async save(): Promise<void> {
    this.busy.set(true);
    const writes: SecretWrite[] = [];
    const headers = rowsToRecord(this.rows(), () => crypto.randomUUID(), writes);
    try {
      for (const write of writes) await this.ipc.secretSet(write.name, write.value);
      const saved = await this.ipc.traceExportSetConfig({
        enabled: this.enabled(),
        endpoint: this.endpoint(),
        headers,
      });
      this.endpoint.set(saved.endpoint ?? "");
      this.rows.set(recordToRows(saved.headers ?? {}));
      this.status.set(await this.ipc.traceExportStatus());
      this.toasts.success(saved.enabled === true ? "Export is on" : "Export is off");
    } catch (error) {
      this.toasts.fail("Could not save the export settings", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected async exportNow(): Promise<void> {
    this.busy.set(true);
    try {
      this.status.set(await this.ipc.traceExportNow());
    } catch (error) {
      this.toasts.fail("Could not export", error);
    } finally {
      this.busy.set(false);
    }
  }
}
