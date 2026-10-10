import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
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
import { CompletionSource } from "./completion-source";
import { describeParameters, fillTemplate, matches, templateVariables } from "./explorer.model";
import { ExplorerStore } from "./explorer.store";
import { Tab, TabList, TabPanel } from "../../ui/tablist/tablist";

type Section = "tools" | "resources" | "prompts";

/** Browse what a connected server offers: tools, resources, and prompts. */
@Component({
  selector: "app-explorer",
  imports: [JsonView, RouterLink, ResultView, TabList, Tab, TabPanel],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./explorer.html",
  styleUrl: "./explorer.scss",
})
export class Explorer implements OnInit {
  readonly serverId = input.required<string>();

  private readonly store = inject(ExplorerStore);
  private readonly toasts = inject(ToastService);
  private readonly ipc = inject(TauriIpcService);

  /** Suggestions of the server while the user types an argument or a template variable. */
  private readonly completions = new CompletionSource(
    (key, value) => this.fetchSuggestions(key, value),
    (key, values) => this.suggestions.update((all) => ({ ...all, [key]: values })),
  );

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
  protected readonly templateValues = signal<Record<string, string>>({});
  /** Suggestions per field: `prompt:<argument>` or `template:<variable>`. */
  protected readonly suggestions = signal<Record<string, string[]>>({});
  protected readonly watched = computed(() => this.store.watched(this.serverId()));
  protected readonly changed = computed(() => this.store.changed(this.serverId()));
  protected readonly canWatch = computed(() => this.snapshot().details?.canSubscribe === true);
  protected readonly templateVars = computed(() => {
    const resource = this.selectedResource();
    return resource?.template ? templateVariables(resource.key) : null;
  });
  protected readonly templateTarget = computed(() =>
    this.templateVars() === null ? this.templateUri() : this.filledTemplate(),
  );
  private readonly filledTemplate = computed(() => {
    const resource = this.selectedResource();
    return resource ? fillTemplate(resource.key, this.templateValues()) : "";
  });
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
    inject(DestroyRef).onDestroy(() => this.completions.cancel());
    effect(() => {
      // Switching the selection clears what was read for the previous item. Only the selection is a
      // dependency; reloaded lists must not clear a result the user is looking at.
      const name = this.selectedName();
      untracked(() => {
        this.resourceResult.set(null);
        this.promptResult.set(null);
        this.promptValues.set({});
        this.templateValues.set({});
        this.suggestions.set({});
        this.completions.cancel();
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
        this.store.loadSession(id).catch(() => undefined);
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
      this.store.markRead(this.serverId(), uri);
    } catch (error) {
      this.toasts.fail("Could not read the resource", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected async toggleWatch(uri: string): Promise<void> {
    try {
      if (this.watched().has(uri)) await this.store.unwatch(this.serverId(), uri);
      else await this.store.watch(this.serverId(), uri);
    } catch (error) {
      this.toasts.fail("Could not change the subscription", error);
    }
  }

  protected setPromptValue(name: string, value: string): void {
    this.promptValues.update((all) => ({ ...all, [name]: value }));
    this.suggest(`prompt:${name}`, value);
  }

  protected setTemplateValue(name: string, value: string): void {
    this.templateValues.update((all) => ({ ...all, [name]: value }));
    this.suggest(`template:${name}`, value);
  }

  private suggest(key: string, value: string): void {
    if (this.snapshot().details?.hasCompletions) this.completions.request(key, value);
  }

  private async fetchSuggestions(key: string, value: string): Promise<string[]> {
    const [kind, ...rest] = key.split(":");
    const argument = rest.join(":");
    const prompt = this.selectedPrompt();
    const resource = this.selectedResource();
    const target =
      kind === "prompt" && prompt
        ? ({ type: "prompt", name: prompt.name } as const)
        : kind === "template" && resource
          ? ({ type: "resource", uriTemplate: resource.key } as const)
          : null;
    if (!target) return [];
    const filled = Object.entries(kind === "prompt" ? this.promptValues() : this.templateValues());
    const context = Object.fromEntries(filled.filter(([name, v]) => name !== argument && v !== ""));
    const found = await this.ipc.completionComplete(
      this.serverId(),
      target,
      argument,
      value,
      context,
    );
    return found.values;
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
