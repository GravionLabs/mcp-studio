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

/** Explains how to route a real client through MCP Studio so its traffic shows up in the inspector. */
@Component({
  selector: "app-proxy-panel",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./proxy-panel.component.html",
  styleUrl: "./proxy-panel.component.scss",
})
export class ProxyPanelComponent implements OnInit {
  readonly serverName = input.required<string>();

  private readonly ipc = inject(TauriIpcService);
  private readonly toasts = inject(ToastService);

  protected readonly info = signal<ProxyInfo | null>(null);
  protected readonly command = computed(() => {
    const binary = this.info()?.proxyBinary;
    return binary ? proxyCommand(binary, this.serverName()) : null;
  });

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
