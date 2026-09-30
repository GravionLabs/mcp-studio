import { ChangeDetectionStrategy, Component } from "@angular/core";
import { RouterOutlet } from "@angular/router";

@Component({
  selector: "app-root",
  imports: [RouterOutlet],
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    <main class="hello">
      <h1>MCP Studio</h1>
      <p>Register, test, inspect, and trace MCP servers.</p>
    </main>
    <router-outlet />
  `,
  styles: `
    .hello {
      padding: 2rem;
    }
  `,
})
export class AppComponent {}
