/**
 * Asks for suggestions while the user types: waits for a pause, and drops an answer that arrives
 * after the user typed on. Each field (`key`) is tracked on its own.
 */
export class CompletionSource {
  private readonly timers = new Map<string, ReturnType<typeof setTimeout>>();
  private readonly latest = new Map<string, number>();
  private counter = 0;

  constructor(
    private readonly fetch: (key: string, value: string) => Promise<string[]>,
    private readonly apply: (key: string, values: string[]) => void,
    private readonly delayMs = 250,
  ) {}

  /** The field `key` now holds `value`. */
  request(key: string, value: string): void {
    clearTimeout(this.timers.get(key));
    const ticket = ++this.counter;
    this.latest.set(key, ticket);
    this.timers.set(
      key,
      setTimeout(() => {
        this.timers.delete(key);
        this.fetch(key, value).then(
          (values) => {
            if (this.latest.get(key) === ticket) this.apply(key, values);
          },
          // A server that cannot complete is not an error the user needs to see while typing.
          () => undefined,
        );
      }, this.delayMs),
    );
  }

  /** Forgets pending requests; answers that are still on their way are dropped. */
  cancel(): void {
    this.timers.forEach((timer) => clearTimeout(timer));
    this.timers.clear();
    this.latest.clear();
  }
}
