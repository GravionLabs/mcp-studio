import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  effect,
  inject,
  viewChild,
} from "@angular/core";
import { DialogService } from "../../core/dialog.service";

/** Renders the dialog requested through {@link DialogService}. */
@Component({
  selector: "app-dialog-host",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./dialog-host.html",
  styleUrl: "./dialog-host.scss",
})
export class DialogHost {
  protected readonly dialogs = inject(DialogService);
  private readonly input = viewChild<ElementRef<HTMLInputElement>>("input");
  private readonly confirmButton = viewChild<ElementRef<HTMLButtonElement>>("confirmButton");

  constructor() {
    effect(() => {
      // Focus the text field of a prompt, or the confirm button, when a dialog opens.
      const current = this.dialogs.current();
      const input = this.input();
      const button = this.confirmButton();
      if (current) queueMicrotask(() => (input ?? button)?.nativeElement.focus());
    });
  }

  protected value(): string {
    return this.input()?.nativeElement.value ?? "";
  }

  protected accept(): void {
    const current = this.dialogs.current();
    this.dialogs.respond(current?.kind === "prompt" ? this.value() : true);
  }

  protected cancel(): void {
    this.dialogs.respond(this.dialogs.current()?.kind === "prompt" ? null : false);
  }
}
