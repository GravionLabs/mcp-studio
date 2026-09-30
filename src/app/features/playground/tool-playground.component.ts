import {
  ChangeDetectionStrategy,
  Component,
  DestroyRef,
  computed,
  effect,
  inject,
  input,
  signal,
  untracked,
} from "@angular/core";
import { RouterLink } from "@angular/router";
import type { ProgressEvent, ToolCallResult } from "../../core/bindings";
import { DialogService } from "../../core/dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { JsonEditorComponent } from "../../ui/json-editor/json-editor.component";
import { JsonViewComponent } from "../../ui/json-view/json-view.component";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { CollectionsStore } from "../collections/collections.store";
import { EnvironmentsStore } from "../environments/environments.store";
import { ExplorerStore } from "../explorer/explorer.store";
import { ResultViewComponent } from "../results/result-view.component";
import { parseToolResult } from "../results/result.model";
import { ServersStore } from "../servers/servers.store";
import { parseRaw, readiness, toArguments, toRawText } from "./playground.model";
import { SchemaFormComponent } from "./schema-form.component";
import { JsonSchema, defaultValue } from "./schema-form.model";

/** Call one tool: fill the generated form (or edit raw JSON), run it, inspect the result. */
@Component({
  selector: "app-tool-playground",
  imports: [
    RouterLink,
    SchemaFormComponent,
    JsonEditorComponent,
    JsonViewComponent,
    ResultViewComponent,
  ],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./tool-playground.component.html",
  styleUrl: "./tool-playground.component.scss",
})
export class ToolPlaygroundComponent {
  readonly id = input.required<string>();
  readonly name = input.required<string>();
  /** Id of a saved request whose arguments are loaded (query parameter `request`). */
  readonly request = input<string>();

  private readonly explorer = inject(ExplorerStore);
  private readonly servers = inject(ServersStore);
  private readonly environments = inject(EnvironmentsStore);
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly tabs = inject(WorkspaceTabsService);
  private readonly destroyRef = inject(DestroyRef);
  protected readonly collections = inject(CollectionsStore);
  private readonly dialogs = inject(DialogService);

  protected readonly server = computed(() => this.servers.byId().get(this.id()));
  protected readonly tool = computed(() =>
    this.explorer.snapshot(this.id()).tools.find((t) => t.name === this.name()),
  );
  protected readonly schema = computed<JsonSchema>(
    () => (this.tool()?.inputSchema as JsonSchema | undefined) ?? { type: "object" },
  );

  protected readonly mode = signal<"form" | "json">("form");
  protected readonly value = signal<unknown>(undefined);
  protected readonly rawText = signal("{}");
  protected readonly rawError = signal<string | null>(null);
  protected readonly readiness = computed(() =>
    readiness(this.schema(), this.value(), this.rawError()),
  );

  protected readonly savedRequest = computed(() => {
    const id = this.request();
    const saved = id ? this.collections.requestById(id) : undefined;
    return saved && saved.serverId === this.id() && saved.toolName === this.name()
      ? saved
      : undefined;
  });
  protected readonly saveOpen = signal(false);
  protected readonly saveName = signal("");
  protected readonly saveFolder = signal("");

  protected readonly callId = signal<string | null>(null);
  protected readonly progress = signal<ProgressEvent | null>(null);
  protected readonly result = signal<ToolCallResult | null>(null);
  protected readonly failure = signal<string | null>(null);
  protected readonly showRaw = signal(false);
  protected readonly parsedResult = computed(() => {
    const r = this.result();
    return r && !r.cancelled ? parseToolResult(r.result) : null;
  });
  protected readonly running = computed(() => this.callId() !== null);
  protected readonly progressPercent = computed(() => {
    const p = this.progress();
    return p?.total ? Math.min(100, Math.round(((p.progress ?? 0) / p.total) * 100)) : null;
  });

  constructor() {
    effect(() => {
      const id = this.id();
      const name = this.name();
      this.tabs.open({
        id: `tool:${id}:${name}`,
        title: name,
        route: `/servers/${id}/tools/${name}`,
      });
      untracked(() => {
        if (!this.explorer.snapshot(id).details) void this.explorer.load(id);
      });
    });

    // Start from the schema's defaults whenever another tool is shown.
    effect(() => {
      const tool = this.tool();
      if (!tool) return;
      const saved = this.savedRequest();
      untracked(() => {
        const initial = saved
          ? saved.arguments
          : (defaultValue(this.schema(), this.schema()) ?? {});
        this.value.set(initial);
        this.rawText.set(toRawText(initial));
        this.rawError.set(null);
        this.result.set(null);
        this.failure.set(null);
      });
    });

    void this.ipc
      .listen<ProgressEvent>("mcp://progress", (event) => {
        if (event.callId === this.callId()) this.progress.set(event);
      })
      .then((stop) => this.destroyRef.onDestroy(stop));
  }

  protected openSave(): void {
    this.saveName.set(this.savedRequest()?.name ?? this.tool()?.title ?? this.name());
    this.saveFolder.set(
      this.savedRequest()?.collectionId ?? this.collections.options()[0]?.id ?? "",
    );
    this.saveOpen.set(true);
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected async newFolder(): Promise<void> {
    const name = await this.dialogs.prompt("Name of the new collection", "", "Create");
    if (!name) return;
    try {
      this.saveFolder.set((await this.collections.createFolder(null, name)).id);
    } catch (error) {
      this.toasts.fail("Could not create the collection", error);
    }
  }

  /** Saves the current input; with `replace` the loaded request is updated instead of duplicated. */
  protected async save(replace: boolean): Promise<void> {
    const collectionId = this.saveFolder();
    const name = this.saveName().trim();
    if (!collectionId || !name) {
      this.toasts.error("Choose a collection and give the request a name.");
      return;
    }
    const input = {
      collectionId,
      serverId: this.id(),
      method: "tools/call",
      name,
      toolName: this.name(),
      arguments: toArguments(this.schema(), this.value()),
      notes: this.savedRequest()?.notes ?? "",
    };
    try {
      const existing = this.savedRequest();
      if (replace && existing) {
        await this.collections.updateRequest(existing.id, input);
      } else {
        await this.collections.saveRequest(input);
      }
      this.saveOpen.set(false);
      this.toasts.success(`Saved "${name}"`);
    } catch (error) {
      this.toasts.fail("Could not save the request", error);
    }
  }

  protected setMode(mode: "form" | "json"): void {
    this.mode.set(mode);
  }

  protected onForm(next: unknown): void {
    this.value.set(next);
    this.rawText.set(toRawText(next));
    this.rawError.set(null);
  }

  protected onRaw(text: string): void {
    this.rawText.set(text);
    const parsed = parseRaw(text);
    if (parsed.ok) {
      this.value.set(parsed.value);
      this.rawError.set(null);
    } else {
      this.rawError.set(parsed.error);
    }
  }

  protected async run(): Promise<void> {
    if (!this.readiness().ready || this.running()) return;
    const callId = crypto.randomUUID();
    this.callId.set(callId);
    this.progress.set(null);
    this.result.set(null);
    this.failure.set(null);
    try {
      this.result.set(
        await this.ipc.toolCall({
          serverId: this.id(),
          toolName: this.name(),
          arguments: toArguments(this.schema(), this.value()),
          environmentId: this.environments.activeId(),
          callId,
        }),
      );
    } catch (error) {
      this.failure.set(error instanceof Error ? error.message : String(error));
    } finally {
      this.callId.set(null);
    }
  }

  protected async cancel(): Promise<void> {
    const callId = this.callId();
    if (!callId) return;
    try {
      await this.ipc.requestCancel(callId);
    } catch (error) {
      this.toasts.fail("Could not cancel the call", error);
    }
  }
}
