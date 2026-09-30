import { ChangeDetectionStrategy, Component } from "@angular/core";

/** Right column: message timeline. Filled in by the inspector PBI. */
@Component({
  selector: "app-inspector-panel",
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<div class="pad muted">Inspector: no messages recorded yet.</div>`,
  styles: `
    .pad {
      padding: 12px;
    }
  `,
})
export class InspectorPanelComponent {}
