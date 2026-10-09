import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  signal,
} from "@angular/core";
import type { StorageInfo } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ThemeService } from "../../core/theme.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { TraceExport } from "../traces/trace-export";
import { RetentionForm, describeStorage, parseRetention, toRetentionForm } from "./settings.model";

/** Retention of the recorded history, the theme, and the optional OpenTelemetry export. */
@Component({
  selector: "app-settings-page",
  imports: [TraceExport],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./settings-page.html",
  styleUrl: "./settings-page.scss",
})
export class SettingsPage implements OnInit {
  protected readonly theme = inject(ThemeService);
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly tabs = inject(WorkspaceTabsService);

  protected readonly form = signal<RetentionForm>({ maxAgeDays: "30", maxMessages: "100000" });
  protected readonly storage = signal<StorageInfo | null>(null);
  protected readonly busy = signal(false);
  protected readonly submitted = signal(false);
  protected readonly problems = computed(() => parseRetention(this.form()).problems);
  protected readonly storageText = computed(() => {
    const info = this.storage();
    return info ? describeStorage(info) : "Loading…";
  });

  constructor() {
    this.tabs.open({ id: "settings", title: "Settings", route: "/settings" });
  }

  ngOnInit(): void {
    this.ipc.retentionGet().then(
      (policy) => this.form.set(toRetentionForm(policy)),
      (error: unknown) => this.toasts.fail("Could not load the retention settings", error),
    );
    void this.refreshStorage();
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }

  protected patch(partial: Partial<RetentionForm>): void {
    this.form.update((f) => ({ ...f, ...partial }));
  }

  protected async save(): Promise<void> {
    this.submitted.set(true);
    const { policy } = parseRetention(this.form());
    if (!policy) return;
    this.busy.set(true);
    try {
      this.form.set(toRetentionForm(await this.ipc.retentionSet(policy)));
      this.submitted.set(false);
      this.toasts.info("Retention saved; older history was deleted.");
      await this.refreshStorage();
    } catch (error) {
      this.toasts.fail("Could not save the retention settings", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected async deleteHistory(): Promise<void> {
    const confirmed = await this.dialogs.confirm(
      "Delete all recorded messages and the tool call history? Servers, flows and other settings stay.",
      { confirmLabel: "Delete history", danger: true },
    );
    if (!confirmed) return;
    this.busy.set(true);
    try {
      const deleted = await this.ipc.historyDeleteAll();
      this.toasts.info(deleted === 1 ? "Deleted 1 message." : `Deleted ${deleted} messages.`);
      await this.refreshStorage();
    } catch (error) {
      this.toasts.fail("Could not delete the history", error);
    } finally {
      this.busy.set(false);
    }
  }

  private async refreshStorage(): Promise<void> {
    try {
      this.storage.set(await this.ipc.storageInfo());
    } catch (error) {
      this.toasts.fail("Could not read the database size", error);
    }
  }
}
