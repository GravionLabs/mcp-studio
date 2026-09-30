import { describe, expect, it } from "vitest";
import { isAtBottom, visibleRange } from "./virtual-range";

describe("visibleRange", () => {
  it("renders only the rows in view plus overscan", () => {
    const range = visibleRange(0, 280, 28, 10_000, 4);
    expect(range).toEqual({ start: 0, end: 15, offsetTop: 0, offsetBottom: (10_000 - 15) * 28 });
  });

  it("moves with the scroll position", () => {
    const range = visibleRange(28 * 500, 280, 28, 10_000, 4);
    expect(range.start).toBe(496);
    expect(range.end).toBe(515);
    expect(range.offsetTop).toBe(496 * 28);
  });

  it("clamps at the end of the list", () => {
    const range = visibleRange(28 * 99, 280, 28, 100, 4);
    expect(range.end).toBe(100);
    expect(range.offsetBottom).toBe(0);
  });

  it("handles empty lists and bad input", () => {
    expect(visibleRange(0, 100, 28, 0)).toEqual({
      start: 0,
      end: 0,
      offsetTop: 0,
      offsetBottom: 0,
    });
    expect(visibleRange(-50, -10, 28, 5)).toMatchObject({ start: 0 });
    expect(visibleRange(0, 100, 0, 5).end).toBe(0);
  });

  it("keeps total height constant", () => {
    const total = 1234;
    const range = visibleRange(9000, 300, 28, total);
    expect(range.offsetTop + (range.end - range.start) * 28 + range.offsetBottom).toBe(total * 28);
  });
});

describe("isAtBottom", () => {
  it("detects the end of the content", () => {
    expect(isAtBottom(700, 300, 1000)).toBe(true);
    expect(isAtBottom(690, 300, 1000)).toBe(true);
    expect(isAtBottom(500, 300, 1000)).toBe(false);
  });
});
