import { ChangeDetectionStrategy, Component, computed, input, output } from "@angular/core";

/** Edits a list of name/text pairs, keeping the order. Used for values, outputs, and the like. */
@Component({
  selector: "app-string-map-editor",
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: `
    @for (row of rows(); track row.index) {
      <div class="row">
        <input
          [attr.aria-label]="keyLabel() + ' name'"
          [placeholder]="keyLabel()"
          [value]="row.key"
          (change)="rename(row.key, text($event))"
        />
        <input
          class="wide"
          [attr.aria-label]="'Value of ' + row.key"
          [placeholder]="valuePlaceholder()"
          [value]="row.value"
          (change)="setValue(row.key, text($event))"
        />
        <button
          type="button"
          class="ghost"
          [attr.aria-label]="'Remove ' + row.key"
          (click)="remove(row.key)"
        >
          ✕
        </button>
      </div>
    }
    <button type="button" class="ghost" (click)="add()">+ Add {{ keyLabel() }}</button>
  `,
  styles: `
    .row {
      display: flex;
      gap: 4px;
      margin-bottom: 4px;
    }

    .wide {
      flex: 1;
      min-width: 0;
    }
  `,
})
export class StringMapEditor {
  readonly value = input.required<Record<string, string>>();
  readonly keyLabel = input("entry");
  readonly valuePlaceholder = input("{{ steps.id.field }}");
  readonly valueChange = output<Record<string, string>>();

  protected readonly rows = computed(() =>
    Object.entries(this.value()).map(([key, value], index) => ({ key, value, index })),
  );

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }

  protected rename(from: string, to: string): void {
    const next = to.trim();
    if (next === "" || next === from || next in this.value()) return;
    this.valueChange.emit(
      Object.fromEntries(Object.entries(this.value()).map(([k, v]) => [k === from ? next : k, v])),
    );
  }

  protected setValue(key: string, text: string): void {
    this.valueChange.emit({ ...this.value(), [key]: text });
  }

  protected remove(key: string): void {
    this.valueChange.emit(
      Object.fromEntries(Object.entries(this.value()).filter(([k]) => k !== key)),
    );
  }

  protected add(): void {
    let n = Object.keys(this.value()).length + 1;
    while (`${this.keyLabel()}${n}` in this.value()) n += 1;
    this.valueChange.emit({ ...this.value(), [`${this.keyLabel()}${n}`]: "" });
  }
}
