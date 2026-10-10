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
import { DialogService } from "../../core/dialog.service";
import { ToastService } from "../../core/toast.service";
import { EnvironmentsStore } from "../environments/environments.store";
import { Explorer } from "../explorer/explorer";
import { ToolDocs } from "../docs/tool-docs";
import { ToolLint } from "../lint/tool-lint";
import { ContextCostPanel } from "../prices/context-cost-panel";
import { ProxyPanel } from "../proxy/proxy-panel";
import { LogLevel } from "./log-level";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import {
  SERVER_TABS,
  type ServerTab,
  activeTab,
  needsAzureLogin,
  needsTenant,
  visibleTabs,
} from "./server-detail.model";
import { ServersStore } from "./servers.store";
import { Tab, TabList, TabPanel } from "../../ui/tablist/tablist";

/** Summary of one server. The explorer takes over this page in a later PBI. */
@Component({
  selector: "app-server-detail",
  imports: [
    RouterLink,
    Explorer,
    LogLevel,
    ProxyPanel,
    ContextCostPanel,
    ToolLint,
    ToolDocs,
    TabList,
    Tab,
    TabPanel,
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./server-detail.html",
  styleUrl: "./server-detail.scss",
})
export class ServerDetail {
  readonly id = input.required<string>();
  /** `?tab=client` opens that section; unknown or hidden sections are ignored. */
  readonly tab = input<string>();

  private readonly store = inject(ServersStore);
  private readonly router = inject(Router);
  private readonly toasts = inject(ToastService);
  private readonly dialogs = inject(DialogService);
  private readonly tabs = inject(WorkspaceTabsService);
  private readonly ipc = inject(TauriIpcService);
  private readonly environments = inject(EnvironmentsStore);
  private readonly destroyRef = inject(DestroyRef);
  protected readonly status = inject(ConnectionStatusService);
  protected readonly logs = signal<LogEvent[]>([]);
  protected readonly busy = signal(false);
  /** `null` while unknown. Only meaningful for servers that use OAuth. */
  protected readonly signedIn = signal<boolean | null>(null);
  protected readonly signingIn = signal(false);
  protected readonly azureSigningIn = signal(false);
  protected readonly state = computed(() => this.status.stateOf(this.id()));
  protected readonly lastError = computed(
    () =>
      this.status.statuses().find((s) => s.serverId === this.id() && s.state === "error")?.message,
  );

  /** The tab the user picked; `null` until they pick one. */
  protected readonly selectedTab = signal<ServerTab | null>(null);
  private readonly connected = computed(() => this.state() === "connected");
  protected readonly visibleTabs = computed(() => visibleTabs(this.connected()));
  protected readonly activeTab = computed(() => activeTab(this.selectedTab(), this.connected()));

  protected readonly server = computed(() => this.store.byId().get(this.id()));
  protected readonly loaded = this.store.loaded;

  constructor() {
    effect(() => {
      const wanted = SERVER_TABS.find((t) => t.id === this.tab());
      if (wanted) this.selectedTab.set(wanted.id);
    });
    effect(() => {
      const server = this.server();
      this.signedIn.set(null);
      if (server?.oauth) {
        this.ipc.oauthStatus(server.id).then(
          (status) => this.signedIn.set(status),
          () => this.signedIn.set(false),
        );
      }
    });
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

  /** Opens the browser for OAuth sign-in and waits until it is done. Returns whether it worked. */
  protected async signIn(): Promise<boolean> {
    this.signingIn.set(true);
    try {
      await this.ipc.oauthSignIn(this.id());
      this.signedIn.set(true);
      this.toasts.success("Signed in");
      return true;
    } catch (error) {
      this.toasts.fail("Sign-in failed", error);
      return false;
    } finally {
      this.signingIn.set(false);
    }
  }

  protected async signOut(): Promise<void> {
    try {
      await this.ipc.oauthSignOut(this.id());
      this.signedIn.set(false);
      this.toasts.success("Signed out");
    } catch (error) {
      this.toasts.fail("Could not sign out", error);
    }
  }

  /** Whether the last connection error says that the Azure login is missing. */
  protected readonly azureLoginNeeded = computed(() => needsAzureLogin(this.lastError()));

  /** Runs `az login` (the CLI opens the browser) and connects once the user has signed in. */
  protected async azureLogin(): Promise<void> {
    let tenant: string | null = null;
    if (needsTenant(this.lastError())) {
      tenant = await this.dialogs.prompt(
        "Your tenant requires multi-factor authentication. Enter its tenant ID or domain (for example contoso.onmicrosoft.com).",
        "",
        "Sign in",
      );
      if (tenant === null) return;
    }
    this.azureSigningIn.set(true);
    try {
      await this.ipc.azureLogin(tenant);
    } catch (error) {
      this.toasts.fail("Could not sign in to Azure", error);
      return;
    } finally {
      this.azureSigningIn.set(false);
    }
    this.toasts.success("Signed in to Azure");
    await this.connect();
  }

  protected async connect(): Promise<void> {
    if (this.server()?.oauth && this.signedIn() !== true && !(await this.signIn())) return;
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
      !(await this.dialogs.confirm(
        `Delete "${server.name}"? Its history and saved requests are deleted too.`,
        { confirmLabel: "Delete", danger: true },
      ))
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
