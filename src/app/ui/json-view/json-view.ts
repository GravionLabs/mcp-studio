import { ChangeDetectionStrategy, Component, computed, input } from "@angular/core";

/** Pretty-printed, read-only JSON. */
@Component({
  selector: "app-json-view",
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `<pre class="json mono">{{ text() }}</pre>`,
  styles: `
    .json {
      margin: 0;
      padding: 8px;
      overflow: auto;
      max-height: var(--json-max-height, 320px);
      background: var(--bg);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      white-space: pre-wrap;
      word-break: break-word;
    }
  `,
})
export class JsonView {
  readonly value = input<unknown>();
  protected readonly text = computed(() => {
    const value = this.value();
    if (value === undefined) return "";
    if (typeof value === "string") {
      try {
        return JSON.stringify(JSON.parse(value), null, 2);
      } catch {
        return value;
      }
    }
    return JSON.stringify(value, null, 2);
  });
}
