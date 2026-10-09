/** The tab to move to for a key press, or null when the key does not move between tabs. */
export function nextTabIndex(key: string, current: number, count: number): number | null {
  if (count === 0) return null;
  switch (key) {
    case "ArrowRight":
    case "ArrowDown":
      return (current + 1) % count;
    case "ArrowLeft":
    case "ArrowUp":
      return (current - 1 + count) % count;
    case "Home":
      return 0;
    case "End":
      return count - 1;
    default:
      return null;
  }
}

/** What the keyboard handler needs from a tab element. */
export interface TabElement {
  focus(): void;
  click(): void;
}

export interface TabKeyEvent {
  key: string;
  target: unknown;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  preventDefault(): void;
}

/**
 * Moves focus to another tab and selects it (by clicking it) when `event` is an arrow key, Home or
 * End pressed on one of `tabs`. Returns whether the event was handled.
 */
export function handleTabKey(event: TabKeyEvent, tabs: TabElement[]): boolean {
  if (event.altKey || event.ctrlKey || event.metaKey) return false;
  const current = tabs.findIndex((tab) => tab === event.target);
  if (current < 0) return false;
  const next = nextTabIndex(event.key, current, tabs.length);
  if (next === null) return false;
  event.preventDefault();
  tabs[next].focus();
  tabs[next].click();
  return true;
}
