import {
  ChangeDetectionStrategy,
  Component,
  OnDestroy,
  computed,
  effect,
  inject,
  signal,
  untracked,
} from "@angular/core";
import type { ClientAnswer, ClientRequest, ClientRequestDone } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { SchemaForm } from "../playground/schema-form";
import { defaultValue } from "../playground/schema-form.model";
import { toArguments } from "../playground/playground.model";
import { ServersStore } from "../servers/servers.store";
import {
  dequeue,
  elicitationProblems,
  elicitationView,
  enqueue,
  samplingBlocker,
  samplingView,
} from "./client-requests.model";

/**
 * Shows what a server asks of the client: a model answer (sampling) or input from the user
 * (elicitation). Nothing is answered without the user; a withdrawn question disappears.
 */
@Component({
  selector: "app-client-requests",
  imports: [SchemaForm],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./client-requests.html",
  styleUrl: "./client-requests.scss",
})
export class ClientRequests implements OnDestroy {
  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);
  private readonly servers = inject(ServersStore);
  private readonly stops: (() => void)[] = [];
  private destroyed = false;

  protected readonly queue = signal<ClientRequest[]>([]);
  protected readonly current = computed(() => this.queue()[0] ?? null);
  protected readonly serverName = computed(() => {
    const current = this.current();
    return current ? (this.servers.byId().get(current.serverId)?.name ?? "A server") : "";
  });
  protected readonly sampling = computed(() => {
    const current = this.current();
    return current?.kind === "sampling" ? samplingView(current.params) : null;
  });
  protected readonly elicitation = computed(() => {
    const current = this.current();
    return current?.kind === "elicitation" ? elicitationView(current.params) : null;
  });

  protected readonly answerText = signal("");
  protected readonly model = signal("claude-sonnet-5-5");
  protected readonly drafting = signal(false);
  protected readonly formValue = signal<unknown>({});
  protected readonly blocker = computed(() => samplingBlocker(this.answerText()));
  protected readonly problems = computed(() => {
    const view = this.elicitation();
    return view ? elicitationProblems(view.schema, this.formValue()) : [];
  });

  constructor() {
    this.subscribe<ClientRequest>("mcp://client-request", (request) =>
      this.queue.update((list) => enqueue(list, request)),
    );
    this.subscribe<ClientRequestDone>("mcp://client-request-done", (done) =>
      this.queue.update((list) => dequeue(list, done.id)),
    );
    // Questions that arrived before the window was listening.
    void this.ipc
      .clientRequestsPending()
      .then((pending) => this.queue.update((list) => pending.reduce(enqueue, list)))
      .catch(() => undefined);

    // Each question starts with an empty answer and a form with its defaults.
    effect(() => {
      this.current();
      untracked(() => {
        this.answerText.set("");
        const view = this.elicitation();
        this.formValue.set(view ? (defaultValue(view.schema, view.schema) ?? {}) : {});
      });
    });
  }

  ngOnDestroy(): void {
    this.destroyed = true;
    this.stops.forEach((stop) => stop());
  }

  private subscribe<T>(event: string, handler: (payload: T) => void): void {
    void this.ipc.listen<T>(event, handler).then((stop) => {
      if (this.destroyed) stop();
      else this.stops.push(stop);
    });
  }

  protected text(event: Event): string {
    return (event.target as HTMLInputElement | HTMLTextAreaElement).value;
  }

  /** Lets a model write a draft. The user reads and sends it; nothing goes to the server yet. */
  protected async draft(request: ClientRequest): Promise<void> {
    this.drafting.set(true);
    try {
      const suggestion = await this.ipc.clientRequestSuggest(request.id, this.model());
      if (this.current()?.id === request.id) this.answerText.set(suggestion.text);
    } catch (error) {
      this.toasts.fail("The model could not answer", error);
    } finally {
      this.drafting.set(false);
    }
  }

  protected respond(request: ClientRequest): Promise<void> {
    return this.answer(request, { action: "respond", text: this.answerText(), model: null });
  }

  protected accept(request: ClientRequest): Promise<void> {
    const view = elicitationView(request.params);
    return this.answer(request, {
      action: "accept",
      content: toArguments(view.schema, this.formValue()),
    });
  }

  protected async answer(request: ClientRequest, answer: ClientAnswer): Promise<void> {
    this.queue.update((list) => dequeue(list, request.id));
    try {
      await this.ipc.clientRequestAnswer(request.id, answer);
    } catch (error) {
      this.toasts.fail("Could not send the answer", error);
    }
  }
}
