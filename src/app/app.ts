import { ChangeDetectionStrategy, Component, effect, inject } from "@angular/core";
import { RouterOutlet } from "@angular/router";
import { ThemeService } from "./core/theme.service";
import { PaneLayoutService } from "./ui/pane-layout.service";
import { ResizeHandleDirective } from "./ui/resize-handle.directive";
import { ClientRequests } from "./features/client-requests/client-requests";
import { FlowConfirmations } from "./features/flows/flow-confirmations";
import { DialogHost } from "./ui/dialog/dialog-host";
import { StatusBar } from "./ui/status-bar/status-bar";
import { TabPanel } from "./ui/tablist/tablist";
import { Tabs } from "./ui/tabs/tabs";
import { WorkspaceTabsService } from "./ui/tabs/workspace-tabs.service";
import { Toasts } from "./ui/toasts/toasts";
import { Toolbar } from "./ui/toolbar/toolbar";
import { Sidebar } from "./features/sidebar/sidebar";
import { InspectorPanel } from "./features/inspector/inspector-panel";

@Component({
  selector: "app-root",
  imports: [
    RouterOutlet,
    ResizeHandleDirective,
    DialogHost,
    ClientRequests,
    FlowConfirmations,
    StatusBar,
    TabPanel,
    Tabs,
    Toasts,
    Toolbar,
    Sidebar,
    InspectorPanel,
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./app.html",
  styleUrl: "./app.scss",
})
export class App {
  protected readonly layout = inject(PaneLayoutService);
  protected readonly tabs = inject(WorkspaceTabsService);
  private readonly theme = inject(ThemeService);

  constructor() {
    effect(() => {
      document.documentElement.dataset["theme"] = this.theme.theme();
    });
  }
}
