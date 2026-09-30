import { ChangeDetectionStrategy, Component, OnInit, inject } from "@angular/core";
import { RouterLink, RouterLinkActive } from "@angular/router";
import { ConnectionStatusService } from "../../core/connection-status.service";
import { ToastService } from "../../core/toast.service";
import { EnvironmentsStore } from "../environments/environments.store";
import { ServersStore } from "../servers/servers.store";

/** Left column: servers (and, later, collections). */
@Component({
  selector: "app-sidebar",
  imports: [RouterLink, RouterLinkActive],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./sidebar.component.html",
  styleUrl: "./sidebar.component.scss",
})
export class SidebarComponent implements OnInit {
  protected readonly store = inject(ServersStore);
  protected readonly status = inject(ConnectionStatusService);
  private readonly environments = inject(EnvironmentsStore);
  private readonly toasts = inject(ToastService);

  ngOnInit(): void {
    this.store.load().catch((error: unknown) => this.toasts.fail("Could not load servers", error));
    this.environments
      .load()
      .catch((error: unknown) => this.toasts.fail("Could not load environments", error));
  }
}
