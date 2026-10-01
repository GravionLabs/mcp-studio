import { ChangeDetectionStrategy, Component, inject, input, signal } from "@angular/core";
import { FileDialogService, fileNameFor } from "../../core/file-dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { ExplorerStore } from "../explorer/explorer.store";

/** Documentation of a connected server's tools as Markdown, to read, copy, or save as a file. */
@Component({
  selector: "app-tool-docs",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./tool-docs.component.html",
  styleUrl: "./tool-docs.component.scss",
})
export class ToolDocsComponent {
  readonly serverId = input.required<string>();
  readonly serverName = input.required<string>();

  private readonly ipc = inject(TauriIpcService);
  private readonly explorer = inject(ExplorerStore);
  private readonly files = inject(FileDialogService);
  private readonly toasts = inject(ToastService);

  protected readonly markdown = signal<string | null>(null);
  protected readonly busy = signal(false);

  private tools() {
    return this.explorer.snapshot(this.serverId()).tools;
  }

  protected async generate(): Promise<string | null> {
    this.busy.set(true);
    try {
      const text = await this.ipc.serverDocs(this.serverId(), this.tools());
      this.markdown.set(text);
      return text;
    } catch (error) {
      this.toasts.fail("Could not write the documentation", error);
      return null;
    } finally {
      this.busy.set(false);
    }
  }

  protected async copy(): Promise<void> {
    const text = this.markdown() ?? (await this.generate());
    if (text === null) return;
    try {
      await navigator.clipboard.writeText(text);
      this.toasts.success("Copied the Markdown");
    } catch (error) {
      this.toasts.fail("Could not copy", error);
    }
  }

  protected async save(): Promise<void> {
    try {
      const path = await this.files.pickSavePath(fileNameFor(this.serverName(), "md"), "md");
      if (path === null) return;
      this.busy.set(true);
      await this.ipc.serverDocsExport(this.serverId(), this.tools(), path);
      this.toasts.success("Saved the documentation");
    } catch (error) {
      this.toasts.fail("Could not save the documentation", error);
    } finally {
      this.busy.set(false);
    }
  }

  protected hide(): void {
    this.markdown.set(null);
  }
}
