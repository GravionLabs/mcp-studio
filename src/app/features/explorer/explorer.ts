import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  effect,
  inject,
  input,
  signal,
  untracked,
} from "@angular/core";
import { RouterLink } from "@angular/router";
import type { PromptInfo } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { ResultView } from "../results/result-view";
import { missingArguments, promptMessages, resourceBlocks } from "../results/mcp-content";
import { JsonView } from "../../ui/json-view/json-view";
import { describeParameters, matches } from "./explorer.model";
import { ExplorerStore } from "./explorer.store";

type Section = "tools" | "resources" | "prompts";

/** Browse what a connected server offers: tools, resources, and prompts. */
@Component({
  selector: "app-explorer",
  imports: [JsonView, RouterLink, ResultView],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./explorer.html",
  styleUrl: "./explorer.scss",
})
export class Explorer implements OnInit {
  readonly serverId = input.required<string>();

  private readonly store = inject(ExplorerStore);
  private readonly toasts = inject(ToastService);
  private readonly ipc = inject(TauriIpcService);

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

  /** Result of reading the selected resource. */
  protected readonly resourceResult = signal<unknown>(null);
  protected readonly templateUri = signal("");
  protected readonly promptValues = signal<Record<string, string>>({});
  protected readonly promptResult = signal<unknown>(null);
  protected readonly busy = signal(false);

  protected readonly resourceView = computed(() => {
    const result = this.resourceResult();
    return result === null
      ? null
      : { blocks: resourceBlocks(result), structured: undefined, isError: false };
  });
  protected readonly promptView = computed(() => {
    const result = this.promptResult();
    return result === null ? null : promptMessages(result);
  });
  protected readonly promptMissing = computed(() => {
    const prompt = this.selectedPrompt();
    return prompt ? missingArguments(prompt.arguments ?? [], this.promptValues()) : [];
  });

  constructor() {
    effect(() => {
      // Switching the selection clears what was read for the previous item. Only the selection is a
      // dependency; reloaded lists must not clear a result the user is looking at.
      const name = this.selectedName();
      untracked(() => {
        this.resourceResult.set(null);
        this.promptResult.set(null);
        this.promptValues.set({});
        const resource = this.resources().find((r) => r.key === name);
        this.templateUri.set(resource?.template ? resource.key : "");
      });
    });

    effect(() => {
      // Reload whenever another server is shown in this component. `load` reads and writes the
      // store's signals, so it must run untracked or the effect would trigger itself forever.
      const id = this.serverId();
      untracked(() => {
        void this.store.load(id);
        this.selectedName.set(null);
      });
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

  protected async readResource(uri: string): Promise<void> {
    this.busy.set(true);
    try {
      this.resourceResult.set(await this.ipc.resourceRead(this.serverId(), uri));
    } catch (error) {
      this.toasts.fail("Could not read the resource", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected setPromptValue(name: string, value: string): void {
    this.promptValues.update((all) => ({ ...all, [name]: value }));
  }

  protected async getPrompt(prompt: PromptInfo): Promise<void> {
    this.busy.set(true);
    try {
      const values = Object.fromEntries(
        Object.entries(this.promptValues()).filter(([, v]) => v.trim() !== ""),
      );
      this.promptResult.set(await this.ipc.promptGet(this.serverId(), prompt.name, values));
    } catch (error) {
      this.toasts.fail("Could not get the prompt", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected reload(): void {
    void this.store.load(this.serverId());
  }
}
