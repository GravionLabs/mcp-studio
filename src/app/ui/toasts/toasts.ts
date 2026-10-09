import { ChangeDetectionStrategy, Component, inject } from "@angular/core";
import { ToastService } from "../../core/toast.service";

@Component({
  selector: "app-toasts",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./toasts.html",
  styleUrl: "./toasts.scss",
})
export class Toasts {
  protected readonly toasts = inject(ToastService);
}
