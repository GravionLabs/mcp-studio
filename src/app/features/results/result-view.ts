import { ChangeDetectionStrategy, Component, computed, input } from "@angular/core";
import { JsonView } from "../../ui/json-view/json-view";
import { ParsedResult, ResultBlock, prettyJson } from "./result.model";

/** Renders tool results and other MCP content: text, JSON, images, audio, resources, and links. */
@Component({
  selector: "app-result-view",
  imports: [JsonView],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./result-view.html",
  styleUrl: "./result-view.scss",
})
export class ResultView {
  readonly result = input.required<ParsedResult>();
  protected readonly hasStructured = computed(() => this.result().structured !== undefined);

  protected pretty(text: string): string {
    return prettyJson(text);
  }

  protected size(base64: string): string {
    const bytes = Math.floor((base64.length * 3) / 4);
    return bytes < 1024 ? `${bytes} B` : `${(bytes / 1024).toFixed(1)} KiB`;
  }

  protected trackBlock(index: number, block: ResultBlock): string {
    return `${index}:${block.kind}`;
  }
}
