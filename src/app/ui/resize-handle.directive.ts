import { Directive, ElementRef, HostListener, inject, input, output } from "@angular/core";

/**
 * Turns an element into a drag handle. Emits the new pane width while dragging.
 * `edge` says on which side of the pane the handle sits: dragging right grows a pane whose handle is
 * on its right edge and shrinks one whose handle is on its left edge.
 */
@Directive({
  selector: "[appResizeHandle]",
  host: { class: "resize-handle", role: "separator", "aria-orientation": "vertical" },
})
export class ResizeHandleDirective {
  readonly width = input.required<number>({ alias: "appResizeHandle" });
  readonly edge = input<"left" | "right">("right");
  readonly resized = output<number>();

  private readonly element = inject<ElementRef<HTMLElement>>(ElementRef);

  @HostListener("pointerdown", ["$event"])
  protected start(event: PointerEvent): void {
    event.preventDefault();
    const startX = event.clientX;
    const startWidth = this.width();
    const direction = this.edge() === "right" ? 1 : -1;
    const target = this.element.nativeElement;
    target.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) =>
      this.resized.emit(startWidth + (e.clientX - startX) * direction);
    const end = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", end);
      target.removeEventListener("pointercancel", end);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", end);
    target.addEventListener("pointercancel", end);
  }
}
