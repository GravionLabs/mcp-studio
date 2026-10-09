import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  signal,
} from "@angular/core";
import { RouterLink } from "@angular/router";
import type { ClientEntry, RoutePreview } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { ServersStore } from "../servers/servers.store";
import { canRestore, canRoute, forceQuestion, groupEntries, statusLabel } from "./clients.model";

/** The entry whose routing the user is looking at, with what would change. */
interface PendingRoute {
  entry: ClientEntry;
  preview: RoutePreview;
}

/** Routes the servers of other clients through MCP Studio, and puts them back. */
@Component({
  selector: "app-clients-page",
  imports: [RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./clients-page.html",
  styleUrl: "./clients-page.scss",
})
export class ClientsPage implements OnInit {
  private readonly ipc = inject(TauriIpcService);
  private readonly servers = inject(ServersStore);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly tabs = inject(WorkspaceTabsService);

  protected readonly entries = signal<ClientEntry[]>([]);
  protected readonly loaded = signal(false);
  protected readonly busy = signal(false);
  protected readonly pending = signal<PendingRoute | null>(null);
  protected readonly groups = computed(() => groupEntries(this.entries()));
  protected readonly statusLabel = statusLabel;
  protected readonly canRoute = canRoute;
  protected readonly canRestore = canRestore;

  constructor() {
    this.tabs.open({ id: "clients", title: "Clients", route: "/clients" });
  }

  ngOnInit(): void {
    void this.reload();
  }

  protected async reload(): Promise<void> {
    try {
      this.entries.set(await this.ipc.clientEntries());
    } catch (error) {
      this.toasts.fail("Could not read the client configurations", error);
    } finally {
      this.loaded.set(true);
    }
  }

  /** Shows what routing would change; nothing is written yet. */
  protected async review(entry: ClientEntry): Promise<void> {
    this.busy.set(true);
    try {
      const preview = await this.ipc.clientRoutePreview({
        path: entry.path,
        pointer: entry.pointer,
        name: entry.name,
      });
      this.pending.set({ entry, preview });
    } catch (error) {
      this.pending.set(null);
      this.toasts.fail(`Cannot route "${entry.name}"`, error);
    } finally {
      this.busy.set(false);
    }
  }

  protected cancel(): void {
    this.pending.set(null);
  }

  protected async confirm(): Promise<void> {
    const pending = this.pending();
    if (!pending) return;
    this.busy.set(true);
    try {
      const result = await this.ipc.clientRoute({
        path: pending.entry.path,
        pointer: pending.entry.pointer,
        name: pending.entry.name,
      });
      this.pending.set(null);
      this.toasts.success(
        `"${pending.entry.name}" now goes through MCP Studio as "${result.serverName}". Restart ${pending.entry.client}.`,
      );
      await Promise.all([this.reload(), this.servers.load()]);
    } catch (error) {
      this.toasts.fail("Could not route the server", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected async restore(entry: ClientEntry): Promise<void> {
    const routeId = entry.routeId;
    if (!routeId) return;
    this.busy.set(true);
    try {
      let result = await this.ipc.clientUnroute(routeId, false);
      if (!result.restored && result.conflict) {
        const force = await this.dialogs.confirm(forceQuestion(result.conflict), {
          confirmLabel: "Restore anyway",
          danger: true,
        });
        if (!force) return;
        result = await this.ipc.clientUnroute(routeId, true);
      }
      if (result.restored) {
        this.toasts.success(
          `"${entry.name}" is back as it was. Restart ${entry.client}. The server stays in MCP Studio.`,
        );
        await this.reload();
      }
    } catch (error) {
      this.toasts.fail("Could not restore the entry", error);
    } finally {
      this.busy.set(false);
    }
  }
}
