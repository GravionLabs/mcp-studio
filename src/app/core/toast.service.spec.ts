import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ToastService } from "./toast.service";

describe("ToastService", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("adds toasts with increasing ids", () => {
    const service = new ToastService();
    const a = service.info("one");
    const b = service.success("two");
    expect(b).toBeGreaterThan(a);
    expect(service.toasts().map((t) => t.message)).toEqual(["one", "two"]);
  });

  it("dismisses a toast by id", () => {
    const service = new ToastService();
    const id = service.info("gone");
    service.info("stays");
    service.dismiss(id);
    expect(service.toasts().map((t) => t.message)).toEqual(["stays"]);
  });

  it("expires toasts, errors later than infos", () => {
    const service = new ToastService();
    service.info("short");
    service.error("long", "details");
    vi.advanceTimersByTime(4001);
    expect(service.toasts().map((t) => t.message)).toEqual(["long"]);
    vi.advanceTimersByTime(6000);
    expect(service.toasts()).toEqual([]);
  });

  it("keeps details of failures", () => {
    const service = new ToastService();
    service.fail("Could not connect", new Error("ENOENT"));
    service.fail("Odd", "plain string");
    const [a, b] = service.toasts();
    expect(a).toMatchObject({ kind: "error", detail: "ENOENT" });
    expect(b?.detail).toBe("plain string");
  });
});
