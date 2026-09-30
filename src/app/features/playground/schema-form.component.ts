import { ChangeDetectionStrategy, Component, computed, input, output } from "@angular/core";
import { FieldKind, JsonSchema, defaultValue, kindOf, resolve } from "./schema-form.model";

/**
 * Renders a form for a JSON Schema value. Recursive: objects and arrays render this component for
 * their children. Anything the form cannot express is edited as JSON text.
 */
@Component({
  selector: "app-schema-form",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./schema-form.component.html",
  styleUrl: "./schema-form.component.scss",
})
export class SchemaFormComponent {
  readonly schema = input.required<JsonSchema>();
  /** The schema of the whole tool input, for `$ref` resolution. */
  readonly root = input.required<JsonSchema>();
  readonly value = input<unknown>();
  readonly label = input("");
  readonly required = input(false);
  readonly valueChange = output<unknown>();

  protected readonly resolved = computed(() => resolve(this.schema(), this.root()));
  protected readonly kind = computed<FieldKind>(() => kindOf(this.schema(), this.root()));
  protected readonly properties = computed(() =>
    Object.entries(this.resolved().properties ?? {}).map(([name, schema]) => ({
      name,
      schema,
      required: (this.resolved().required ?? []).includes(name),
    })),
  );
  protected readonly items = computed(() =>
    Array.isArray(this.value()) ? (this.value() as unknown[]) : [],
  );
  protected readonly record = computed(() =>
    typeof this.value() === "object" && this.value() !== null && !Array.isArray(this.value())
      ? (this.value() as Record<string, unknown>)
      : {},
  );
  protected readonly jsonText = computed(() => {
    const value = this.value();
    return value === undefined ? "" : JSON.stringify(value, null, 2);
  });
  protected jsonInvalid = false;

  protected field(name: string, next: unknown): void {
    this.valueChange.emit({ ...this.record(), [name]: next });
  }

  protected item(index: number, next: unknown): void {
    this.valueChange.emit(this.items().map((v, i) => (i === index ? next : v)));
  }

  protected addItem(): void {
    const itemSchema = this.resolved().items;
    const initial = itemSchema ? defaultValue(itemSchema, this.root()) : undefined;
    this.valueChange.emit([
      ...this.items(),
      initial ?? (itemSchema && kindOf(itemSchema, this.root()) === "string" ? "" : undefined),
    ]);
  }

  protected removeItem(index: number): void {
    this.valueChange.emit(this.items().filter((_, i) => i !== index));
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement).value;
  }

  protected numberValue(event: Event): number | undefined {
    const raw = this.text(event).trim();
    if (raw === "") return undefined;
    const parsed = Number(raw);
    return Number.isNaN(parsed) ? undefined : parsed;
  }

  protected enumOptions(): unknown[] {
    return this.resolved().enum ?? [];
  }

  protected selectEnum(event: Event): void {
    const index = Number(this.text(event));
    this.valueChange.emit(Number.isNaN(index) ? undefined : this.enumOptions()[index]);
  }

  protected enumIndex(): number {
    const current = JSON.stringify(this.value());
    return this.enumOptions().findIndex((o) => JSON.stringify(o) === current);
  }

  protected setJson(event: Event): void {
    const raw = this.text(event);
    if (raw.trim() === "") {
      this.jsonInvalid = false;
      this.valueChange.emit(undefined);
      return;
    }
    try {
      this.valueChange.emit(JSON.parse(raw));
      this.jsonInvalid = false;
    } catch {
      this.jsonInvalid = true;
    }
  }

  protected title(): string {
    return this.label() || this.resolved().title || "";
  }
}
