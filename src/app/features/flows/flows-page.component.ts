import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from "@angular/core";
import type { FlowRecord, ToolPolicy } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { FileDialogService, fileNameFor } from "../../core/file-dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { RouterLink } from "@angular/router";
import { FlowRunComponent } from "./flow-run.component";
import { describeFlow, newFlow } from "./flows.model";

/** The flow library: create, import from and export to YAML files, delete. */
@Component({
  selector: "app-flows-page",
  imports: [FlowRunComponent, RouterLink],
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
  /** The flow whose run panel is open. */
  protected readonly running = signal<FlowRecord | null>(null);
  protected readonly policy = signal<ToolPolicy>({ allow: [] });
  protected readonly newAllowed = signal("");
  protected describe = describeFlow;

  constructor() {
    this.tabs.open({ id: "flows", title: "Flows", route: "/flows" });
  }

  ngOnInit(): void {
    void this.refresh();
    void this.loadPolicy();
  }

  protected toggleRun(record: FlowRecord): void {
    this.running.update((current) => (current?.id === record.id ? null : record));
  }

  private async loadPolicy(): Promise<void> {
    try {
      this.policy.set(await this.ipc.toolPolicyGet());
    } catch (error) {
      this.toasts.fail("Could not load the allowed tools", error);
    }
  }

  private async savePolicy(allow: string[]): Promise<void> {
    try {
      this.policy.set(await this.ipc.toolPolicySet({ allow }));
    } catch (error) {
      this.toasts.fail("Could not save the allowed tools", error);
    }
  }

  protected async allowEntry(): Promise<void> {
    const entry = this.newAllowed().trim();
    if (entry === "") return;
    await this.savePolicy([...(this.policy().allow ?? []), entry]);
    this.newAllowed.set("");
  }

  protected removeEntry(entry: string): Promise<void> {
    return this.savePolicy((this.policy().allow ?? []).filter((e) => e !== entry));
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
      if (this.running()?.id === record.id) this.running.set(null);
      await this.refresh();
    } catch (error) {
      this.toasts.fail("Could not delete the flow", error);
    }
  }
}
