import { ChangeDetectionStrategy, Component, OnInit, inject } from "@angular/core";
import { RouterLink, RouterLinkActive } from "@angular/router";
import { DialogService } from "../../core/dialog.service";
import { FileDialogService } from "../../core/file-dialog.service";
import { CollectionFolderComponent } from "../collections/collection-folder.component";
import { CollectionsStore } from "../collections/collections.store";
import { ConnectionEventsService } from "../../core/connection-events.service";
import { ConnectionStatusService } from "../../core/connection-status.service";
import { ToastService } from "../../core/toast.service";
import { EnvironmentsStore } from "../environments/environments.store";
import { ServersStore } from "../servers/servers.store";

/** Left column: servers (and, later, collections). */
@Component({
  selector: "app-sidebar",
  imports: [RouterLink, RouterLinkActive, CollectionFolderComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./sidebar.component.html",
  styleUrl: "./sidebar.component.scss",
})
export class SidebarComponent implements OnInit {
  protected readonly store = inject(ServersStore);
  protected readonly status = inject(ConnectionStatusService);
  private readonly environments = inject(EnvironmentsStore);
  private readonly events = inject(ConnectionEventsService);
  protected readonly collections = inject(CollectionsStore);
  private readonly dialogs = inject(DialogService);
  private readonly files = inject(FileDialogService);
  private readonly toasts = inject(ToastService);

  ngOnInit(): void {
    this.collections
      .load()
      .catch((error: unknown) => this.toasts.fail("Could not load collections", error));
    this.events
      .start()
      .catch((error: unknown) => this.toasts.fail("Could not listen for connection events", error));
    this.store.load().catch((error: unknown) => this.toasts.fail("Could not load servers", error));
    this.environments
      .load()
      .catch((error: unknown) => this.toasts.fail("Could not load environments", error));
  }

  protected async newFolder(): Promise<void> {
    const name = await this.dialogs.prompt("Name of the new collection", "", "Create");
    if (!name) return;
    try {
      await this.collections.createFolder(null, name);
    } catch (error) {
      this.toasts.fail("Could not create the collection", error);
    }
  }

  protected async importCollection(): Promise<void> {
    try {
      const path = await this.files.pickOpenPath();
      if (!path) return;
      const report = await this.collections.importFile(path, null);
      const skipped = report.skipped.length;
      this.toasts.success(
        `Imported ${report.requestsCreated} request(s)` +
          (skipped > 0 ? `, skipped ${skipped}` : ""),
      );
      if (skipped > 0) this.toasts.error("Some requests were skipped", report.skipped.join("\n"));
    } catch (error) {
      this.toasts.fail("Could not import the collection", error);
    }
  }
}
