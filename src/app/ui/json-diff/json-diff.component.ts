import { ChangeDetectionStrategy, Component, computed, input } from "@angular/core";
import { DiffEntry, diffJson, preview, summarizeDiff } from "./json-diff";

/** Table of the differences between two JSON documents. */
@Component({
  selector: "app-json-diff",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./json-diff.component.html",
  styleUrl: "./json-diff.component.scss",
})
export class JsonDiffComponent {
  readonly before = input.required<unknown>();
  readonly after = input.required<unknown>();
  readonly ignoreTopLevel = input<string[]>([]);

  protected readonly entries = computed<DiffEntry[]>(() =>
    diffJson(this.before(), this.after(), { ignoreTopLevel: this.ignoreTopLevel() }),
  );
  protected readonly summary = computed(() => summarizeDiff(this.entries()));
  protected readonly show = preview;
}
