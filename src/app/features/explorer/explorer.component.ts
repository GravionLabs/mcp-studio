import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  effect,
  inject,
  input,
  signal,
} from "@angular/core";
import { RouterLink } from "@angular/router";
import { ToastService } from "../../core/toast.service";
import { JsonViewComponent } from "../../ui/json-view/json-view.component";
import { describeParameters, matches } from "./explorer.model";
import { ExplorerStore } from "./explorer.store";

type Section = "tools" | "resources" | "prompts";

/** Browse what a connected server offers: tools, resources, and prompts. */
@Component({
  selector: "app-explorer",
  imports: [JsonViewComponent, RouterLink],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./explorer.component.html",
  styleUrl: "./explorer.component.scss",
})
export class ExplorerComponent implements OnInit {
  readonly serverId = input.required<string>();

  private readonly store = inject(ExplorerStore);
  private readonly toasts = inject(ToastService);

  protected readonly section = signal<Section>("tools");
  protected readonly query = signal("");
  protected readonly selectedName = signal<string | null>(null);

  protected readonly snapshot = computed(() => this.store.snapshot(this.serverId()));

  protected readonly tools = computed(() =>
    this.snapshot().tools.filter((t) => matches(this.query(), t.name, t.title, t.description)),
  );
  protected readonly resources = computed(() => {
    const snap = this.snapshot();
    return [
      ...snap.resources.map((r) => ({
        key: r.uri,
        label: r.name,
        sub: r.uri,
        item: r,
        template: false,
      })),
      ...snap.templates.map((t) => ({
        key: t.uriTemplate,
        label: t.name,
        sub: t.uriTemplate,
        item: t,
        template: true,
      })),
    ].filter((r) => matches(this.query(), r.label, r.sub));
  });
  protected readonly prompts = computed(() =>
    this.snapshot().prompts.filter((p) => matches(this.query(), p.name, p.title, p.description)),
  );

  protected readonly selectedTool = computed(() =>
    this.snapshot().tools.find((t) => t.name === this.selectedName()),
  );
  protected readonly selectedResource = computed(() =>
    this.resources().find((r) => r.key === this.selectedName()),
  );
  protected readonly selectedPrompt = computed(() =>
    this.snapshot().prompts.find((p) => p.name === this.selectedName()),
  );
  protected readonly parameters = computed(() =>
    describeParameters(this.selectedTool()?.inputSchema),
  );

  constructor() {
    effect(() => {
      // Reload whenever another server is shown in this component.
      void this.store.load(this.serverId());
      this.selectedName.set(null);
    });
  }

  ngOnInit(): void {
    this.store
      .listen()
      .catch((error: unknown) => this.toasts.fail("Could not listen for list changes", error));
  }

  protected show(section: Section): void {
    this.section.set(section);
    this.selectedName.set(null);
    this.query.set("");
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement).value;
  }

  protected reload(): void {
    void this.store.load(this.serverId());
  }
}
