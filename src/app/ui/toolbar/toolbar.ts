import { ChangeDetectionStrategy, Component, inject } from "@angular/core";
import { RouterLink } from "@angular/router";
import { ThemeService } from "../../core/theme.service";
import { EnvironmentsStore } from "../../features/environments/environments.store";
import { PaneLayoutService } from "../pane-layout.service";

@Component({
  selector: "app-toolbar",
  imports: [RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./toolbar.html",
  styleUrl: "./toolbar.scss",
})
export class Toolbar {
  protected readonly theme = inject(ThemeService);
  protected readonly layout = inject(PaneLayoutService);
  protected readonly environments = inject(EnvironmentsStore);

  protected select(event: Event): void {
    this.environments.setActive((event.target as HTMLSelectElement).value || null);
  }
}
