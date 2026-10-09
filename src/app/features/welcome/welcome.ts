import { ChangeDetectionStrategy, Component, computed, inject, signal } from "@angular/core";
import { Router, RouterLink } from "@angular/router";
import { ToastService } from "../../core/toast.service";
import { ServersStore } from "../servers/servers.store";
import { DEMO_SERVER, findDemoServer, recentServers } from "./welcome.model";

/** The start page: the first steps for a new user and the servers used last. */
@Component({
  selector: "app-welcome",
  imports: [RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./welcome.html",
  styleUrl: "./welcome.scss",
})
export class Welcome {
  private readonly store = inject(ServersStore);
  private readonly router = inject(Router);
  private readonly toasts = inject(ToastService);

  protected readonly busy = signal(false);
  protected readonly recent = computed(() => recentServers(this.store.servers()));
  protected readonly hasServers = computed(() => this.store.servers().length > 0);
  protected readonly loaded = this.store.loaded;
  /** Where "record a real client" leads: the first recent server's page, else the new server form. */
  protected readonly firstServer = computed(() => this.recent()[0]);

  /** Adds the reference server (once) and opens it. Needs Node.js, because it runs through `npx`. */
  protected async tryDemo(): Promise<void> {
    this.busy.set(true);
    try {
      const server = findDemoServer(this.store.servers()) ?? (await this.store.add(DEMO_SERVER));
      await this.router.navigate(["/servers", server.id]);
    } catch (error) {
      this.toasts.fail("Could not add the demo server", error);
    } finally {
      this.busy.set(false);
    }
  }
}
