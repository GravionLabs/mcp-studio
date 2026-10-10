import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CompletionSource } from "./completion-source";

describe("CompletionSource", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("asks once after a pause, with the last value", async () => {
    const fetch = vi.fn(async (_key: string, value: string) => [value + "!"]);
    const applied: [string, string[]][] = [];
    const source = new CompletionSource(fetch, (k, v) => applied.push([k, v]), 100);
    source.request("name", "a");
    source.request("name", "al");
    await vi.advanceTimersByTimeAsync(99);
    expect(fetch).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(fetch).toHaveBeenCalledTimes(1);
    expect(fetch).toHaveBeenCalledWith("name", "al");
    expect(applied).toEqual([["name", ["al!"]]]);
  });

  it("drops an answer that is out of date", async () => {
    const resolvers: ((values: string[]) => void)[] = [];
    const fetch = () => new Promise<string[]>((resolve) => resolvers.push(resolve));
    const applied: string[][] = [];
    const source = new CompletionSource(fetch, (_k, v) => applied.push(v), 10);
    source.request("name", "a");
    await vi.advanceTimersByTimeAsync(10);
    source.request("name", "al");
    await vi.advanceTimersByTimeAsync(10);
    resolvers[1]?.(["alice"]);
    resolvers[0]?.(["stale"]);
    await vi.advanceTimersByTimeAsync(0);
    expect(applied).toEqual([["alice"]]);
  });

  it("tracks fields separately and ignores failures", async () => {
    const fetch = async (key: string) => {
      if (key === "bad") throw new Error("nope");
      return [key];
    };
    const applied: string[] = [];
    const source = new CompletionSource(fetch, (k) => applied.push(k), 10);
    source.request("a", "x");
    source.request("bad", "x");
    source.request("b", "x");
    await vi.advanceTimersByTimeAsync(10);
    expect(applied.sort()).toEqual(["a", "b"]);
  });

  it("forgets everything on cancel", async () => {
    const fetch = vi.fn(async () => ["x"]);
    const source = new CompletionSource(fetch, () => undefined, 10);
    source.request("a", "x");
    source.cancel();
    await vi.advanceTimersByTimeAsync(50);
    expect(fetch).not.toHaveBeenCalled();
  });
});
