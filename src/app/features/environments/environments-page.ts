import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  signal,
} from "@angular/core";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { DialogService } from "../../core/dialog.service";
import { ToastService } from "../../core/toast.service";
import { KeyValueEditor } from "../../ui/key-value-editor/key-value-editor";
import type { KeyValueRow } from "../../ui/key-value-editor/key-value-row";
import { SecretWrite, recordToRows, rowsToRecord } from "../../ui/key-value-editor/key-value-rows";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { EnvironmentsStore } from "./environments.store";

/** Manage environments: `{{variable}}` sets that are applied when connecting and calling tools. */
@Component({
  selector: "app-environments-page",
  imports: [KeyValueEditor],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./environments-page.html",
  styleUrl: "./environments-page.scss",
})
export class EnvironmentsPage {
  protected readonly store = inject(EnvironmentsStore);
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly tabs = inject(WorkspaceTabsService);

  /** Id of the environment being edited, or null for a new one. */
  protected readonly selectedId = signal<string | null>(null);
  protected readonly name = signal("");
  protected readonly rows = signal<KeyValueRow[]>([]);
  protected readonly canDelete = computed(() => this.selectedId() !== null);

  constructor() {
    this.tabs.open({ id: "environments", title: "Environments", route: "/environments" });
    effect(() => {
      const first = this.store.environments()[0];
      if (this.selectedId() === null && this.name() === "" && first) this.select(first.id);
    });
  }

  protected select(id: string | null): void {
    this.selectedId.set(id);
    const environment = id ? this.store.environments().find((e) => e.id === id) : undefined;
    this.name.set(environment?.name ?? "");
    this.rows.set(environment ? recordToRows(environment.variables) : []);
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }

  protected async save(): Promise<void> {
    const writes: SecretWrite[] = [];
    const variables = rowsToRecord(this.rows(), () => crypto.randomUUID(), writes);
    const input = { name: this.name(), variables };
    try {
      for (const write of writes) await this.ipc.secretSet(write.name, write.value);
      const id = this.selectedId();
      const saved = id ? await this.store.update(id, input) : await this.store.add(input);
      this.select(saved.id);
      this.toasts.success(`Saved ${saved.name}`);
    } catch (error) {
      this.toasts.fail("Could not save the environment", error);
    }
  }

  protected async remove(): Promise<void> {
    const id = this.selectedId();
    if (
      !id ||
      !(await this.dialogs.confirm("Delete this environment?", {
        confirmLabel: "Delete",
        danger: true,
      }))
    ) {
      return;
    }
    try {
      await this.store.remove(id);
      this.select(null);
    } catch (error) {
      this.toasts.fail("Could not delete the environment", error);
    }
  }
}
