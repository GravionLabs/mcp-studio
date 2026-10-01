import { ChangeDetectionStrategy, Component, effect, inject } from "@angular/core";
import { RouterOutlet } from "@angular/router";
import { ThemeService } from "./core/theme.service";
import { PaneLayoutService } from "./ui/pane-layout.service";
import { ResizeHandleDirective } from "./ui/resize-handle.directive";
import { FlowConfirmationsComponent } from "./features/flows/flow-confirmations.component";
import { DialogHostComponent } from "./ui/dialog/dialog-host.component";
import { StatusBarComponent } from "./ui/status-bar/status-bar.component";
import { TabsComponent } from "./ui/tabs/tabs.component";
import { ToastsComponent } from "./ui/toasts/toasts.component";
import { ToolbarComponent } from "./ui/toolbar/toolbar.component";
import { SidebarComponent } from "./features/sidebar/sidebar.component";
import { InspectorPanelComponent } from "./features/inspector/inspector-panel.component";

@Component({
  selector: "app-root",
  imports: [
    RouterOutlet,
    ResizeHandleDirective,
    DialogHostComponent,
    FlowConfirmationsComponent,
    StatusBarComponent,
    TabsComponent,
    ToastsComponent,
    ToolbarComponent,
    SidebarComponent,
    InspectorPanelComponent,
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./app.component.html",
  styleUrl: "./app.component.scss",
})
export class AppComponent {
  protected readonly layout = inject(PaneLayoutService);
  private readonly theme = inject(ThemeService);

  constructor() {
    effect(() => {
      document.documentElement.dataset["theme"] = this.theme.theme();
    });
  }
}
