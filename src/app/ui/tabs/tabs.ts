import { ChangeDetectionStrategy, Component, inject } from "@angular/core";
import { Router } from "@angular/router";
import { Tab, TabList } from "../tablist/tablist";
import { WorkspaceTabsService } from "./workspace-tabs.service";

@Component({
  selector: "app-tabs",
  imports: [TabList, Tab],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./tabs.html",
  styleUrl: "./tabs.scss",
})
export class Tabs {
  protected readonly tabs = inject(WorkspaceTabsService);
  private readonly router = inject(Router);

  protected select(id: string, route: string): void {
    this.tabs.activate(id);
    void this.router.navigateByUrl(route);
  }

  protected close(event: Event, id: string): void {
    event.stopPropagation();
    this.tabs.close(id);
    const next = this.tabs.active();
    void this.router.navigateByUrl(next?.route ?? "/");
  }
}
