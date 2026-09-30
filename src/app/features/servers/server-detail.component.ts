import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  input,
  signal,
} from "@angular/core";
import { Router, RouterLink } from "@angular/router";
import { ConnectionStatusService } from "../../core/connection-status.service";
import type { LogEvent } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { EnvironmentsStore } from "../environments/environments.store";
import { ExplorerComponent } from "../explorer/explorer.component";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { ServersStore } from "./servers.store";

/** Summary of one server. The explorer takes over this page in a later PBI. */
@Component({
  selector: "app-server-detail",
  imports: [RouterLink, ExplorerComponent],
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
  private readonly ipc = inject(TauriIpcService);
  private readonly environments = inject(EnvironmentsStore);
  private readonly destroyRef = inject(DestroyRef);
  protected readonly status = inject(ConnectionStatusService);
  protected readonly logs = signal<LogEvent[]>([]);
  protected readonly busy = signal(false);
  protected readonly state = computed(() => this.status.stateOf(this.id()));
  protected readonly lastError = computed(
    () =>
      this.status.statuses().find((s) => s.serverId === this.id() && s.state === "error")?.message,
  );

  protected readonly server = computed(() => this.store.byId().get(this.id()));
  protected readonly loaded = this.store.loaded;

  constructor() {
    effect(() => {
      const id = this.id();
      this.logs.set([]);
      this.ipc.serverLogs(id).then(
        (lines) => this.logs.set(lines),
        () => undefined,
      );
    });
    void this.ipc
      .listen<LogEvent>("mcp://log", (line) => {
        if (line.serverId === this.id()) this.logs.update((all) => [...all, line].slice(-1000));
      })
      .then((stop) => this.destroyRef.onDestroy(stop));
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

  protected async connect(): Promise<void> {
    this.busy.set(true);
    try {
      await this.ipc.serverConnect(this.id(), this.environments.activeId());
    } catch (error) {
      this.toasts.fail("Could not connect", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected async disconnect(): Promise<void> {
    this.busy.set(true);
    try {
      await this.ipc.serverDisconnect(this.id());
    } catch (error) {
      this.toasts.fail("Could not disconnect", error);
    } finally {
      this.busy.set(false);
    }
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
