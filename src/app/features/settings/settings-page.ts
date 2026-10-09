import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  signal,
} from "@angular/core";
import type { StorageInfo, WorkspaceChanges } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { FileDialogService } from "../../core/file-dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ThemeService } from "../../core/theme.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { TraceExport } from "../traces/trace-export";
import {
  RetentionForm,
  describeSecret,
  describeStorage,
  describeTable,
  hasChanges,
  moreNames,
  parseRetention,
  tablesWithRows,
  toRetentionForm,
} from "./settings.model";

/** A workspace file that was read and is waiting for the user to confirm the import. */
interface PendingImport {
  path: string;
  changes: WorkspaceChanges;
}

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
  private readonly files = inject(FileDialogService);
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

  protected readonly pending = signal<PendingImport | null>(null);
  /** What the last import did; shown so the secrets it still needs can be entered. */
  protected readonly imported = signal<WorkspaceChanges | null>(null);
  protected readonly describeTable = describeTable;
  protected readonly describeSecret = describeSecret;
  protected readonly moreNames = moreNames;
  protected readonly tablesWithRows = tablesWithRows;
  protected readonly hasChanges = hasChanges;

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

  protected async exportWorkspace(): Promise<void> {
    const path = await this.files.pickSavePath("mcp-studio-workspace.json");
    if (!path) return;
    this.busy.set(true);
    try {
      await this.ipc.workspaceExport(path);
      this.toasts.info("Workspace saved. The file names secrets but holds none of their values.");
    } catch (error) {
      this.toasts.fail("Could not save the workspace", error);
    } finally {
      this.busy.set(false);
    }
  }

  /** Reads a workspace file and shows what importing it would do; nothing changes yet. */
  protected async chooseImport(): Promise<void> {
    const path = await this.files.pickOpenPath();
    if (!path) return;
    this.busy.set(true);
    this.imported.set(null);
    try {
      this.pending.set({ path, changes: await this.ipc.workspacePreview(path) });
    } catch (error) {
      this.pending.set(null);
      this.toasts.fail("Could not read the workspace file", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected cancelImport(): void {
    this.pending.set(null);
  }

  protected async applyImport(): Promise<void> {
    const pending = this.pending();
    if (!pending) return;
    this.busy.set(true);
    try {
      this.imported.set(await this.ipc.workspaceImport(pending.path));
      this.pending.set(null);
      this.toasts.info("Workspace imported.");
    } catch (error) {
      this.toasts.fail("Could not import the workspace", error);
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
