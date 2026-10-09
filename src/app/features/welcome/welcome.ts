import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  signal,
} from "@angular/core";
import { Router, RouterLink } from "@angular/router";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { ServersStore } from "../servers/servers.store";
import { demoServerInput, findDemoServer, recentServers } from "./welcome.model";

/** The start page: the first steps for a new user and the servers used last. */
@Component({
  selector: "app-welcome",
  imports: [RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./welcome.html",
  styleUrl: "./welcome.scss",
})
export class Welcome implements OnInit {
  private readonly store = inject(ServersStore);
  private readonly ipc = inject(TauriIpcService);
  private readonly router = inject(Router);
  private readonly toasts = inject(ToastService);

  protected readonly busy = signal(false);
  /** Where the reference server that ships with the app is installed; `null` when it was not found. */
  private readonly bundledDemo = signal<string | null>(null);
  protected readonly demoNeedsNode = computed(() => this.bundledDemo() === null);
  protected readonly recent = computed(() => recentServers(this.store.servers()));
  protected readonly hasServers = computed(() => this.store.servers().length > 0);
  protected readonly loaded = this.store.loaded;
  /** Where "record a real client" leads: the first recent server's page, else the new server form. */
  protected readonly firstServer = computed(() => this.recent()[0]);

  ngOnInit(): void {
    this.ipc.demoServerPath().then(
      (path) => this.bundledDemo.set(path),
      () => this.bundledDemo.set(null),
    );
  }

  /** Adds the reference server (once) and opens it. */
  protected async tryDemo(): Promise<void> {
    this.busy.set(true);
    try {
      const demo = demoServerInput(this.bundledDemo());
      const server = findDemoServer(this.store.servers(), demo) ?? (await this.store.add(demo));
      await this.router.navigate(["/servers", server.id]);
    } catch (error) {
      this.toasts.fail("Could not add the demo server", error);
    } finally {
      this.busy.set(false);
    }
  }
}
