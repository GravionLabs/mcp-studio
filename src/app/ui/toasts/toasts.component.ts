import { ChangeDetectionStrategy, Component, inject } from "@angular/core";
import { ToastService } from "../../core/toast.service";

@Component({
  selector: "app-toasts",
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./toasts.component.html",
  styleUrl: "./toasts.component.scss",
})
export class ToastsComponent {
  protected readonly toasts = inject(ToastService);
}
