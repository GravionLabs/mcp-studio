import { ChangeDetectionStrategy, Component } from "@angular/core";

/** Left column: servers and collections. Filled in by the server registry and collections PBIs. */
@Component({
  selector: "app-sidebar",
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<nav class="pad muted">No servers yet.</nav>`,
  styles: `
    .pad {
      padding: 12px;
    }
  `,
})
export class SidebarComponent {}
