import { ChangeDetectionStrategy, Component } from "@angular/core";

/** Lint probe (#321): an inline template outside a spec file fails `mcp-studio/no-inline-template`. */
@Component({
  selector: "app-inline-template-probe",
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<p>inline</p>`,
})
export class InlineTemplateProbe {}
