import { ChangeDetectionStrategy, Component } from "@angular/core";

@Component({
  selector: "app-welcome",
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <section class="welcome">
      <h1>MCP Studio</h1>
      <p class="muted">Register an MCP server on the left, then explore and call its tools.</p>
    </section>
  `,
  styles: `
    .welcome {
      padding: 32px;
    }
  `,
})
export class Welcome {}
