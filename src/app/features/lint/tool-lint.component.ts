import {
  ChangeDetectionStrategy,
  Component,
  computed,
  effect,
  inject,
  input,
  signal,
} from "@angular/core";
import type { LintReport } from "../../core/bindings";
import { TauriIpcService } from "../../core/tauri-ipc.service";
import { ExplorerStore } from "../explorer/explorer.store";
import { groupByTool, ruleTitle, summarize } from "./lint.model";

/** How good the tool definitions of a connected server are for a model that has to choose among them. */
@Component({
  selector: "app-tool-lint",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./tool-lint.component.html",
  styleUrl: "./tool-lint.component.scss",
})
export class ToolLintComponent {
  readonly serverId = input.required<string>();

  private readonly ipc = inject(TauriIpcService);
  private readonly explorer = inject(ExplorerStore);

  protected readonly report = signal<LintReport | null>(null);
  protected readonly groups = computed(() => {
    const report = this.report();
    return report ? groupByTool(report) : [];
  });
  protected readonly summary = computed(() => {
    const report = this.report();
    return report ? summarize(report) : "";
  });
  protected readonly open = signal(false);
  protected ruleTitle = ruleTitle;

  constructor() {
    effect(() => {
      const tools = this.explorer.snapshot(this.serverId()).tools;
      this.ipc.toolsLint(tools).then(
        (report) => this.report.set(report),
        () => this.report.set(null),
      );
    });
  }
}
