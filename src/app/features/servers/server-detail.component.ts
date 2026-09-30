import { ChangeDetectionStrategy, Component, computed, effect, inject, input } from "@angular/core";
import { Router, RouterLink } from "@angular/router";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { ServersStore } from "./servers.store";

/** Summary of one server. The explorer takes over this page in a later PBI. */
@Component({
  selector: "app-server-detail",
  imports: [RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./server-detail.component.html",
  styleUrl: "./server-detail.component.scss",
})
export class ServerDetailComponent {
  readonly id = input.required<string>();

  private readonly store = inject(ServersStore);
  private readonly router = inject(Router);
  private readonly toasts = inject(ToastService);
  private readonly tabs = inject(WorkspaceTabsService);

  protected readonly server = computed(() => this.store.byId().get(this.id()));
  protected readonly loaded = this.store.loaded;

  constructor() {
    effect(() => {
      const server = this.server();
      if (server) {
        this.tabs.open({
          id: `server:${server.id}`,
          title: server.name,
          route: `/servers/${server.id}`,
        });
      }
    });
  }

  protected entries(record: Record<string, string>): [string, string][] {
    return Object.entries(record);
  }

  protected async remove(): Promise<void> {
    const server = this.server();
    if (
      !server ||
      !window.confirm(`Delete "${server.name}"? Its history and saved requests are deleted too.`)
    ) {
      return;
    }
    try {
      await this.store.remove(server.id);
      this.tabs.close(`server:${server.id}`);
      this.toasts.success(`Deleted ${server.name}`);
      await this.router.navigateByUrl("/");
    } catch (error) {
      this.toasts.fail("Could not delete the server", error);
    }
  }
}
