import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from "@angular/core";
import type { FlowRecord } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { FileDialogService, fileNameFor } from "../../core/file-dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { describeFlow, newFlow } from "./flows.model";

/** The flow library: create, import from and export to YAML files, delete. */
@Component({
  selector: "app-flows-page",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./flows-page.component.html",
  styleUrl: "./flows-page.component.scss",
})
export class FlowsPageComponent implements OnInit {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly files = inject(FileDialogService);
  private readonly tabs = inject(WorkspaceTabsService);

  protected readonly flows = signal<FlowRecord[]>([]);
  protected readonly newName = signal("");
  protected describe = describeFlow;

  constructor() {
    this.tabs.open({ id: "flows", title: "Flows", route: "/flows" });
  }

  ngOnInit(): void {
    void this.refresh();
  }

  protected async refresh(): Promise<void> {
    try {
      this.flows.set(await this.ipc.flowList());
    } catch (error) {
      this.toasts.fail("Could not load the flows", error);
    }
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }

  protected async create(): Promise<void> {
    const name = this.newName().trim();
    if (name === "") return;
    try {
      await this.ipc.flowSave(null, newFlow(name));
      this.newName.set("");
      await this.refresh();
    } catch (error) {
      this.toasts.fail("Could not create the flow", error);
    }
  }

  protected async importFile(): Promise<void> {
    try {
      const path = await this.files.pickOpenPath(["yaml", "yml"]);
      if (path === null) return;
      const imported = await this.ipc.flowImport(path);
      this.toasts.success(`Imported ${imported.flow.name}`);
      await this.refresh();
    } catch (error) {
      this.toasts.fail("Could not import the flow", error);
    }
  }

  protected async exportFile(record: FlowRecord): Promise<void> {
    try {
      const path = await this.files.pickSavePath(fileNameFor(record.flow.name, "yaml"), "yaml");
      if (path === null) return;
      await this.ipc.flowExport(record.id, path);
      this.toasts.success(`Exported ${record.flow.name}`);
    } catch (error) {
      this.toasts.fail("Could not export the flow", error);
    }
  }

  protected async remove(record: FlowRecord): Promise<void> {
    const confirmed = await this.dialogs.confirm(`Delete the flow ${record.flow.name}?`, {
      confirmLabel: "Delete",
      danger: true,
    });
    if (!confirmed) return;
    try {
      await this.ipc.flowDelete(record.id);
      await this.refresh();
    } catch (error) {
      this.toasts.fail("Could not delete the flow", error);
    }
  }
}
