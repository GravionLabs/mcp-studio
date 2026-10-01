import { ChangeDetectionStrategy, Component, inject } from "@angular/core";
import { ConnectionStatusService } from "../../core/connection-status.service";
import { UpdateService } from "../../core/update.service";

@Component({
  selector: "app-status-bar",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./status-bar.component.html",
  styleUrl: "./status-bar.component.scss",
})
export class StatusBarComponent {
  protected readonly status = inject(ConnectionStatusService);
  protected readonly updates = inject(UpdateService);
}
