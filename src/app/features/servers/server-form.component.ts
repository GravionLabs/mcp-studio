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
import { ToastService } from "../../core/toast.service";
import { KeyValueEditorComponent } from "../../ui/key-value-editor/key-value-editor.component";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import {
  ServerFormState,
  emptyForm,
  formToInput,
  inputToForm,
  validateForm,
} from "./server-form.model";
import { ServersStore } from "./servers.store";

/** Creates a server (`/servers/new`) or edits one (`/servers/:id/edit`). */
@Component({
  selector: "app-server-form",
  imports: [KeyValueEditorComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./server-form.component.html",
  styleUrl: "./server-form.component.scss",
})
export class ServerFormComponent {
  /** Route parameter; absent when creating. */
  readonly id = input<string>();

  private readonly store = inject(ServersStore);
  private readonly router = inject(Router);
  private readonly toasts = inject(ToastService);
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

  protected setTransport(transport: TransportKind): void {
    this.patch({ transport });
  }

  protected async save(): Promise<void> {
    this.submitted.set(true);
    if (this.problems().length > 0) return;
    this.saving.set(true);
    try {
      const input = formToInput(this.form());
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
