import { ChangeDetectionStrategy, Component, inject } from "@angular/core";
import { ThemeService } from "../../core/theme.service";
import { PaneLayoutService } from "../pane-layout.service";

@Component({
  selector: "app-toolbar",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./toolbar.component.html",
  styleUrl: "./toolbar.component.scss",
})
export class ToolbarComponent {
  protected readonly theme = inject(ThemeService);
  protected readonly layout = inject(PaneLayoutService);
}
