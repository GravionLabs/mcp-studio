import {
  ChangeDetectionStrategy,
  Component,
  OnInit,
  computed,
  inject,
  signal,
} from "@angular/core";
import { Router } from "@angular/router";
import type { ConfigSource, ImportCandidate } from "../../core/bindings";
import { FileDialogService } from "../../core/file-dialog.service";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ToastService } from "../../core/toast.service";
import { WorkspaceTabsService } from "../../ui/tabs/workspace-tabs.service";
import { ServersStore } from "../servers/servers.store";
import {
  defaultSelection,
  describeCandidate,
  secretCount,
  selectable,
  selectedInputs,
} from "./import.model";

/** Import servers from Claude Desktop, Claude Code, or any file with an `mcpServers` section. */
@Component({
  selector: "app-import-page",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./import-page.component.html",
  styleUrl: "./import-page.component.scss",
})
export class ImportPageComponent implements OnInit {
  private readonly ipc = inject(TauriIpcService);
  private readonly files = inject(FileDialogService);
  private readonly toasts = inject(ToastService);
  private readonly router = inject(Router);
  private readonly tabs = inject(WorkspaceTabsService);
  private readonly servers = inject(ServersStore);

  protected readonly sources = signal<ConfigSource[]>([]);
  protected readonly path = signal<string | null>(null);
  protected readonly candidates = signal<ImportCandidate[]>([]);
  protected readonly selected = signal<ReadonlySet<number>>(new Set());
  protected readonly loading = signal(false);
  protected readonly importing = signal(false);

  protected readonly rows = computed(() =>
    this.candidates().map((candidate, index) => ({
      candidate,
      index,
      summary: describeCandidate(candidate),
      secrets: secretCount(candidate),
    })),
  );
  protected readonly selectedCount = computed(() => this.selected().size);
  protected readonly pickable = computed(() => selectable(this.candidates()));

  ngOnInit(): void {
    this.tabs.open({ id: "import", title: "Import servers", route: "/servers/import" });
    this.ipc.importSources().then(
      (sources) => this.sources.set(sources),
      (error: unknown) => this.toasts.fail("Could not look for client configurations", error),
    );
  }

  protected async choose(): Promise<void> {
    try {
      const path = await this.files.pickOpenPath();
      if (path) await this.preview(path);
    } catch (error) {
      this.toasts.fail("Could not open the file dialog", error);
    }
  }

  protected async preview(path: string): Promise<void> {
    this.loading.set(true);
    try {
      const candidates = await this.ipc.importPreview(path);
      this.path.set(path);
      this.candidates.set(candidates);
      this.selected.set(defaultSelection(candidates));
    } catch (error) {
      this.candidates.set([]);
      this.path.set(null);
      this.toasts.fail("Could not read servers from this file", error);
    } finally {
      this.loading.set(false);
    }
  }

  protected toggle(index: number): void {
    this.selected.update((set) => {
      const next = new Set(set);
      if (!next.delete(index)) next.add(index);
      return next;
    });
  }

  protected selectAll(): void {
    this.selected.set(new Set(this.pickable()));
  }

  protected selectNone(): void {
    this.selected.set(new Set());
  }

  protected async run(): Promise<void> {
    this.importing.set(true);
    try {
      const summary = await this.ipc.importApply(
        selectedInputs(this.candidates(), this.selected()),
      );
      await this.servers.load();
      const parts = [`Imported ${summary.created} server(s)`];
      if (summary.renamed > 0) parts.push(`${summary.renamed} renamed`);
      if (summary.secretsMoved > 0)
        parts.push(`${summary.secretsMoved} secret(s) moved to the keyring`);
      this.toasts.success(parts.join(", "));
      if (summary.failed.length > 0) {
        this.toasts.error(
          `${summary.failed.length} server(s) could not be imported`,
          summary.failed.join("\n"),
        );
      }
      this.tabs.close("import");
      await this.router.navigateByUrl("/");
    } catch (error) {
      this.toasts.fail("Import failed", error);
    } finally {
      this.importing.set(false);
    }
  }
}
