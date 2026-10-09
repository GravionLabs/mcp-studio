import { ChangeDetectionStrategy, Component, input, output } from "@angular/core";

import type { KeyValueRow } from "./key-value-row";

/** Editable list of name/value pairs (environment variables, headers). */
@Component({
  selector: "app-key-value-editor",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./key-value-editor.html",
  styleUrl: "./key-value-editor.scss",
})
export class KeyValueEditor {
  readonly rows = input.required<KeyValueRow[]>();
  readonly keyPlaceholder = input("NAME");
  readonly valuePlaceholder = input("value");
  /** New rows start as secrets (used for headers such as Authorization). */
  readonly secretsDefault = input(false);
  readonly rowsChange = output<KeyValueRow[]>();

  protected edit(index: number, patch: Partial<KeyValueRow>): void {
    this.rowsChange.emit(this.rows().map((row, i) => (i === index ? { ...row, ...patch } : row)));
  }

  protected add(): void {
    this.rowsChange.emit([...this.rows(), { key: "", value: "", secret: this.secretsDefault() }]);
  }

  protected remove(index: number): void {
    this.rowsChange.emit(this.rows().filter((_, i) => i !== index));
  }

  protected checked(event: Event): boolean {
    return (event.target as HTMLInputElement).checked;
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }
}
