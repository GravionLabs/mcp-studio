import { ChangeDetectionStrategy, Component, computed, input } from "@angular/core";
import { JsonViewComponent } from "../../ui/json-view/json-view.component";
import { ParsedResult, ResultBlock, prettyJson } from "./result.model";

/** Renders tool results and other MCP content: text, JSON, images, audio, resources, and links. */
@Component({
  selector: "app-result-view",
  imports: [JsonViewComponent],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./result-view.component.html",
  styleUrl: "./result-view.component.scss",
})
export class ResultViewComponent {
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
