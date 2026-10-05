import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  input,
  signal,
} from "@angular/core";
import type { ProxyInfo } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { proxyCommand } from "./proxy.model";
import { CLIENTS, ClientKind, Snippet, snippetsFor } from "./snippets";

/** Explains how to route a real client through MCP Studio so its traffic shows up in the inspector. */
@Component({
  selector: "app-proxy-panel",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./proxy-panel.html",
  styleUrl: "./proxy-panel.scss",
})
export class ProxyPanel implements OnInit {
  readonly serverName = input.required<string>();
  readonly transport = input.required<"stdio" | "http">();

  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);

  protected readonly info = signal<ProxyInfo | null>(null);
  protected readonly command = computed(() => {
    const binary = this.info()?.proxyBinary;
    return binary ? proxyCommand(binary, this.serverName()) : null;
  });
  protected readonly url = computed(() => {
    const info = this.info();
    return info
      ? `http://127.0.0.1:${info.httpPort}/mcp/${encodeURIComponent(this.serverName())}`
      : null;
  });
  protected readonly clients = CLIENTS;
  protected readonly client = signal<ClientKind>("claude");
  protected readonly snippets = computed<Snippet[]>(() =>
    snippetsFor(
      {
        serverName: this.serverName(),
        transport: this.transport(),
        proxyBinary: this.info()?.proxyBinary ?? null,
        proxyUrl: this.url(),
      },
      this.client(),
    ),
  );
  protected readonly selected = signal(0);
  protected readonly current = computed(
    () => this.snippets()[this.selected()] ?? this.snippets()[0],
  );

  protected selectClient(event: Event): void {
    const id = (event.target as HTMLSelectElement).value;
    const known = CLIENTS.find((c) => c.id === id);
    if (known) {
      this.client.set(known.id);
      this.selected.set(0);
    }
  }

  ngOnInit(): void {
    this.ipc.proxyInfo().then(
      (info) => this.info.set(info),
      (error: unknown) => this.toasts.fail("Could not read the proxy status", error),
    );
  }

  protected async copy(text: string): Promise<void> {
    try {
      await navigator.clipboard.writeText(text);
      this.toasts.success("Copied");
    } catch (error) {
      this.toasts.fail("Could not copy", error);
    }
  }
}
