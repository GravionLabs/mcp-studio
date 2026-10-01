import { ChangeDetectionStrategy, Component, OnInit, inject, signal } from "@angular/core";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { ProviderId, describeTest, qualifiedModel } from "./providers.model";

/** Where flows get their models: Anthropic, an OpenAI-compatible endpoint, and local Ollama. */
@Component({
  selector: "app-providers-page",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./providers-page.component.html",
  styleUrl: "./providers-page.component.scss",
})
export class ProvidersPageComponent implements OnInit {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly tabs = inject(WorkspaceTabsService);

  protected readonly anthropicKey = signal("");
  protected readonly openaiKey = signal("");
  protected readonly hasAnthropicKey = signal(false);
  protected readonly hasOpenaiKey = signal(false);
  protected readonly openaiUrl = signal("");
  protected readonly ollamaUrl = signal("");
  protected readonly ollamaModels = signal<string[]>([]);
  protected readonly model = signal<Record<ProviderId, string>>({
    anthropic: "claude-sonnet-5-5",
    openai: "",
    ollama: "",
  });
  protected readonly testing = signal<ProviderId | null>(null);

  constructor() {
    this.tabs.open({ id: "providers", title: "Providers", route: "/providers" });
  }

  ngOnInit(): void {
    this.ipc.providerStatus().then(
      (status) => {
        this.hasAnthropicKey.set(status.anthropicKey);
        this.hasOpenaiKey.set(status.openaiKey);
        this.openaiUrl.set(status.settings.openaiUrl);
        this.ollamaUrl.set(status.settings.ollamaUrl);
      },
      (error: unknown) => this.toasts.fail("Could not load the provider settings", error),
    );
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLSelectElement).value;
  }

  protected setModel(provider: ProviderId, value: string): void {
    this.model.update((m) => ({ ...m, [provider]: value }));
  }

  protected async saveKey(provider: "anthropic" | "openai"): Promise<void> {
    const key = provider === "anthropic" ? this.anthropicKey() : this.openaiKey();
    try {
      if (key.trim() !== "") await this.ipc.providerSetKey(provider, key);
      if (provider === "openai") await this.saveSettings(false);
      const status = await this.ipc.providerStatus();
      this.hasAnthropicKey.set(status.anthropicKey);
      this.hasOpenaiKey.set(status.openaiKey);
      (provider === "anthropic" ? this.anthropicKey : this.openaiKey).set("");
      this.toasts.success("Saved");
    } catch (error) {
      this.toasts.fail("Could not save", error);
    }
  }

  protected async removeKey(provider: "anthropic" | "openai"): Promise<void> {
    try {
      await this.ipc.providerSetKey(provider, null);
      (provider === "anthropic" ? this.hasAnthropicKey : this.hasOpenaiKey).set(false);
      this.toasts.success("Removed the API key");
    } catch (error) {
      this.toasts.fail("Could not remove the key", error);
    }
  }

  /** Saves both addresses; the backend fills in the defaults for empty ones. */
  protected async saveSettings(announce = true): Promise<void> {
    try {
      const saved = await this.ipc.providerSetSettings({
        openaiUrl: this.openaiUrl(),
        ollamaUrl: this.ollamaUrl(),
      });
      this.openaiUrl.set(saved.openaiUrl);
      this.ollamaUrl.set(saved.ollamaUrl);
      if (announce) this.toasts.success("Saved the addresses");
    } catch (error) {
      this.toasts.fail("Could not save the addresses", error);
      throw error;
    }
  }

  protected async loadOllamaModels(): Promise<void> {
    try {
      await this.saveSettings(false);
      const models = await this.ipc.ollamaModels();
      this.ollamaModels.set(models);
      if (models.length === 0) {
        this.toasts.info(
          "Ollama is running but has no models. Pull one, for example: ollama pull llama3.1",
        );
      } else if (this.model().ollama === "") {
        this.setModel("ollama", models[0] ?? "");
      }
    } catch (error) {
      this.toasts.fail("Could not list the Ollama models", error);
    }
  }

  protected async test(provider: ProviderId): Promise<void> {
    const name = qualifiedModel(provider, this.model()[provider]);
    if (name === "") {
      this.toasts.info("Enter a model name to test with");
      return;
    }
    this.testing.set(provider);
    try {
      if (provider !== "anthropic") await this.saveSettings(false);
      this.toasts.success(describeTest(await this.ipc.providerTest(name)));
    } catch (error) {
      this.toasts.fail(`The ${provider} test failed`, error);
    } finally {
      this.testing.set(null);
    }
  }
}
