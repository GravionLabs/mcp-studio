import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
} from "@angular/core";
import { Router } from "@angular/router";
import type { TransportKind } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { KeyValueEditor } from "../../ui/key-value-editor/key-value-editor";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import {
  AZURE_DEVOPS_URL,
  ServerFormState,
  emptyForm,
  formToInputWithSecrets,
  inputToForm,
  validateForm,
} from "./server-form.model";
import { ServersStore } from "./servers.store";

/** Creates a server (`/servers/new`) or edits one (`/servers/:id/edit`). */
@Component({
  selector: "app-server-form",
  imports: [KeyValueEditor],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./server-form.html",
  styleUrl: "./server-form.scss",
})
export class ServerForm {
  /** Route parameter; absent when creating. */
  readonly id = input<string>();

  private readonly store = inject(ServersStore);
  private readonly router = inject(Router);
  private readonly toasts = inject(ToastService);
  private readonly ipc = inject(TauriIpcService);
  private readonly tabs = inject(WorkspaceTabsService);

  protected readonly form = signal<ServerFormState>(emptyForm());
  protected readonly submitted = signal(false);
  protected readonly saving = signal(false);
  protected readonly problems = computed(() => validateForm(this.form()));
  protected readonly editing = computed(() => this.id() !== undefined);

  constructor() {
    effect(() => {
      const id = this.id();
      const existing = id ? this.store.byId().get(id) : undefined;
      this.form.set(existing ? inputToForm(existing) : emptyForm());
      this.tabs.open({
        id: id ? `server-edit:${id}` : "server-new",
        title: existing ? `Edit ${existing.name}` : "New server",
        route: id ? `/servers/${id}/edit` : "/servers/new",
      });
    });
  }

  protected patch(partial: Partial<ServerFormState>): void {
    this.form.update((f) => ({ ...f, ...partial }));
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLTextAreaElement).value;
  }

  /** OAuth sign-in and the Azure login are alternatives; turning one on turns the other off. */
  protected setOauth(oauth: boolean): void {
    this.patch(oauth ? { oauth, azureCredentials: false } : { oauth });
  }

  protected setAzureCredentials(azureCredentials: boolean): void {
    this.patch(azureCredentials ? { azureCredentials, oauth: false } : { azureCredentials });
  }

  /** Fills in what an Azure DevOps MCP server needs; the user adds the organization name. */
  protected useAzureDevOps(): void {
    this.patch({ url: AZURE_DEVOPS_URL, oauth: false, azureCredentials: true });
  }

  protected setTransport(transport: TransportKind): void {
    this.patch({ transport });
  }

  protected async save(): Promise<void> {
    this.submitted.set(true);
    if (this.problems().length > 0) return;
    this.saving.set(true);
    try {
      const { input, writes } = formToInputWithSecrets(this.form());
      for (const write of writes) await this.ipc.secretSet(write.name, write.value);
      const id = this.id();
      const saved = id ? await this.store.update(id, input) : await this.store.add(input);
      this.toasts.success(`Saved ${saved.name}`);
      this.tabs.close(id ? `server-edit:${id}` : "server-new");
      await this.router.navigateByUrl(`/servers/${saved.id}`);
    } catch (error) {
      this.toasts.fail("Could not save the server", error);
    } finally {
      this.saving.set(false);
    }
  }

  protected cancel(): void {
    const id = this.id();
    this.tabs.close(id ? `server-edit:${id}` : "server-new");
    void this.router.navigateByUrl(id ? `/servers/${id}` : "/");
  }
}
