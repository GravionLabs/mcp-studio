import {
  ChangeDetectionStrategy,
  Component,
  ElementRef,
  OnDestroy,
  TemplateRef,
  afterNextRender,
  computed,
  contentChild,
  effect,
  input,
  output,
  signal,
  viewChild,
} from "@angular/core";
import { NgTemplateOutlet } from "@angular/common";
import { isAtBottom, visibleRange } from "./virtual-range";

export interface RowContext<T> {
  $implicit: T;
  index: number;
}

/** Renders long lists of equally tall rows by creating DOM only for the rows in view. */
@Component({
  selector: "app-virtual-list",
  imports: [NgTemplateOutlet],
  changeDetection: ChangeDetectionStrategy.OnPush,
  templateUrl: "./virtual-list.html",
  styleUrl: "./virtual-list.scss",
})
export class VirtualList<T> implements OnDestroy {
  readonly items = input.required<readonly T[]>();
  readonly rowHeight = input(28);
  /** Keep the newest row in view while items are appended. */
  readonly follow = input(false);
  readonly trackBy = input<(item: T, index: number) => unknown>((_, index) => index);
  readonly atBottomChange = output<boolean>();

  protected readonly row = contentChild.required<TemplateRef<RowContext<T>>>("row");
  private readonly viewport = viewChild.required<ElementRef<HTMLElement>>("viewport");

  private readonly scrollTop = signal(0);
  private readonly height = signal(400);
  private observer: ResizeObserver | null = null;

  protected readonly range = computed(() =>
    visibleRange(this.scrollTop(), this.height(), this.rowHeight(), this.items().length),
  );
  protected readonly slice = computed(() => {
    const { start, end } = this.range();
    return this.items()
      .slice(start, end)
      .map((item, offset) => ({ item, index: start + offset }));
  });

  constructor() {
    afterNextRender(() => {
      const element = this.viewport().nativeElement;
      this.height.set(element.clientHeight);
      this.observer = new ResizeObserver(() => this.height.set(element.clientHeight));
      this.observer.observe(element);
    });

    // When following, jump to the end whenever the list grows.
    effect(() => {
      const count = this.items().length;
      if (this.follow() && count > 0) {
        queueMicrotask(() => this.scrollToEnd());
      }
    });
  }

  ngOnDestroy(): void {
    this.observer?.disconnect();
  }

  protected onScroll(): void {
    const element = this.viewport().nativeElement;
    this.scrollTop.set(element.scrollTop);
    this.atBottomChange.emit(
      isAtBottom(element.scrollTop, element.clientHeight, element.scrollHeight),
    );
  }

  scrollToEnd(): void {
    const element = this.viewport().nativeElement;
    element.scrollTop = element.scrollHeight;
    this.scrollTop.set(element.scrollTop);
  }

  /** Scrolls so that the row is visible (used by keyboard navigation). */
  scrollToIndex(index: number): void {
    const element = this.viewport().nativeElement;
    const top = index * this.rowHeight();
    if (top < element.scrollTop) element.scrollTop = top;
    else if (top + this.rowHeight() > element.scrollTop + element.clientHeight) {
      element.scrollTop = top + this.rowHeight() - element.clientHeight;
    }
    this.scrollTop.set(element.scrollTop);
  }
}
