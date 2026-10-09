import { Directive, ElementRef, afterEveryRender, inject, input } from "@angular/core";
import { handleTabKey } from "./tablist.model";

/**
 * The container of a row of tabs: `role="tablist"` and the keyboard behaviour. Arrow keys, Home and
 * End move to another tab and select it; only the selected tab is in the tab order, so Tab leaves
 * the list (when no tab is selected the first one stays reachable).
 *
 * The value names the group; it keeps the ids of tabs and panels apart when several lists share a
 * page.
 */
@Directive({
  selector: "[appTablist]",
  host: {
    role: "tablist",
    "(keydown)": "onKeydown($event)",
  },
})
export class TabList {
  readonly appTablist = input.required<string>();
  private readonly host = inject<ElementRef<HTMLElement>>(ElementRef);

  constructor() {
    afterEveryRender(() => {
      const tabs = this.tabs();
      if (tabs.length > 0 && !tabs.some((tab) => tab.tabIndex === 0)) tabs[0].tabIndex = 0;
    });
  }

  protected onKeydown(event: KeyboardEvent): void {
    handleTabKey(event, this.tabs());
  }

  private tabs(): HTMLElement[] {
    return Array.from(this.host.nativeElement.querySelectorAll<HTMLElement>('[role="tab"]'));
  }
}

/** One tab. `tabSelected` marks the active one; the value is the id its panel refers to. */
@Directive({
  selector: "[appTab]",
  host: {
    role: "tab",
    "[id]": "elementId()",
    "[attr.aria-selected]": "tabSelected()",
    "[attr.aria-controls]": "tabSelected() ? panelId() : null",
    "[attr.tabindex]": "tabSelected() ? 0 : -1",
  },
})
export class Tab {
  readonly appTab = input.required<string>();
  readonly tabSelected = input(false);
  private readonly list = inject(TabList);

  protected elementId(): string {
    return tabElementId(this.list.appTablist(), this.appTab());
  }

  protected panelId(): string {
    return panelElementId(this.list.appTablist(), this.appTab());
  }
}

/** The panel a tab controls; `tabPanelGroup` is the value of the tab list. */
@Directive({
  selector: "[appTabPanel]",
  host: {
    role: "tabpanel",
    tabindex: "0",
    "[id]": "elementId()",
    "[attr.aria-labelledby]": "labelId()",
  },
})
export class TabPanel {
  readonly appTabPanel = input.required<string>();
  readonly tabPanelGroup = input.required<string>();

  protected elementId(): string {
    return panelElementId(this.tabPanelGroup(), this.appTabPanel());
  }

  protected labelId(): string {
    return tabElementId(this.tabPanelGroup(), this.appTabPanel());
  }
}

function tabElementId(group: string, id: string): string {
  return `${group}-tab-${id}`;
}

function panelElementId(group: string, id: string): string {
  return `${group}-panel-${id}`;
}
