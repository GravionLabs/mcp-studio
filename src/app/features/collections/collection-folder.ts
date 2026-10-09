import { ChangeDetectionStrategy, Component, inject, input } from "@angular/core";
import { Router } from "@angular/router";
import type { SavedRequest } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { FileDialogService, fileNameFor } from "../../core/file-dialog.service";
import { ToastService } from "../../core/toast.service";
import { FolderNode, countRequests } from "./collections.model";
import { CollectionsStore } from "./collections.store";

/** One folder of the collections tree, with its subfolders and requests. Recursive. */
@Component({
  selector: "app-collection-folder",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./collection-folder.html",
  styleUrl: "./collection-folder.scss",
})
export class CollectionFolder {
  readonly folder = input.required<FolderNode>();

  protected readonly store = inject(CollectionsStore);
  private readonly router = inject(Router);
  private readonly dialogs = inject(DialogService);
  private readonly files = inject(FileDialogService);
  private readonly toasts = inject(ToastService);
  protected readonly count = countRequests;

  protected async addFolder(): Promise<void> {
    const name = await this.dialogs.prompt("Name of the new folder", "", "Create");
    if (!name) return;
    await this.run("Could not create the folder", () =>
      this.store.createFolder(this.folder().node.id, name),
    );
  }

  protected async rename(): Promise<void> {
    const name = await this.dialogs.prompt("Rename folder", this.folder().node.name, "Rename");
    if (!name) return;
    await this.run("Could not rename the folder", () =>
      this.store.renameFolder(this.folder().node.id, name),
    );
  }

  protected async remove(): Promise<void> {
    const { node } = this.folder();
    const total = countRequests(this.folder());
    const detail = total > 0 ? ` and its ${total} saved request${total === 1 ? "" : "s"}` : "";
    if (
      !(await this.dialogs.confirm(`Delete "${node.name}"${detail}?`, {
        confirmLabel: "Delete",
        danger: true,
      }))
    ) {
      return;
    }
    await this.run("Could not delete the folder", () => this.store.deleteFolder(node.id));
  }

  protected async export(): Promise<void> {
    const { node } = this.folder();
    await this.run("Could not export the folder", async () => {
      const path = await this.files.pickSavePath(fileNameFor(node.name));
      if (!path) return;
      await this.store.exportFolder(node.id, path);
      this.toasts.success(`Exported ${node.name}`);
    });
  }

  protected open(request: SavedRequest): void {
    const { serverId, toolName } = request;
    if (!toolName) return;
    void this.router.navigate(["/servers", serverId, "tools", toolName], {
      queryParams: { request: request.id },
    });
  }

  protected async removeRequest(request: SavedRequest): Promise<void> {
    if (
      !(await this.dialogs.confirm(`Delete "${request.name}"?`, {
        confirmLabel: "Delete",
        danger: true,
      }))
    ) {
      return;
    }
    await this.run("Could not delete the request", () => this.store.deleteRequest(request.id));
  }

  private async run(failure: string, action: () => Promise<unknown>): Promise<void> {
    try {
      await action();
    } catch (error) {
      this.toasts.fail(failure, error);
    }
  }
}
