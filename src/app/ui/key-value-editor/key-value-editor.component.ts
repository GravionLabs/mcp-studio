import { ChangeDetectionStrategy, Component, input, output } from "@angular/core";

export interface KeyValueRow {
  key: string;
  value: string;
}

/** Editable list of name/value pairs (environment variables, headers). */
@Component({
  selector: "app-key-value-editor",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./key-value-editor.component.html",
  styleUrl: "./key-value-editor.component.scss",
})
export class KeyValueEditorComponent {
  readonly rows = input.required<KeyValueRow[]>();
  readonly keyPlaceholder = input("NAME");
  readonly valuePlaceholder = input("value");
  readonly rowsChange = output<KeyValueRow[]>();

  protected edit(index: number, patch: Partial<KeyValueRow>): void {
    this.rowsChange.emit(this.rows().map((row, i) => (i === index ? { ...row, ...patch } : row)));
  }

  protected add(): void {
    this.rowsChange.emit([...this.rows(), { key: "", value: "" }]);
  }

  protected remove(index: number): void {
    this.rowsChange.emit(this.rows().filter((_, i) => i !== index));
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }
}
