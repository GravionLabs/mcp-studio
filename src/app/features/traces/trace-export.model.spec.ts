import { describe, expect, it } from "vitest";
import type { ExportStatus } from "../../core/bindings";
import { describeExport } from "./trace-export.model";

const status = (overrides: Partial<ExportStatus> = {}): ExportStatus => ({
  lastAttemptAt: null,
  lastSuccessAt: null,
  lastError: null,
  exported: 0,
  ...overrides,
});

describe("describeExport", () => {
  it("says nothing leaves the computer while the export is off", () => {
    expect(describeExport(false, status())).toContain("Nothing leaves this computer");
    // Even a stale error is irrelevant while off.
    expect(describeExport(false, status({ lastError: "boom" }))).toContain("off");
  });

  it("explains that the first export waits for new spans", () => {
    expect(describeExport(true, status())).toContain("Waiting for spans");
  });

  it("shows the error of a failed export", () => {
    expect(describeExport(true, status({ lastError: "the collector answered 500" }))).toBe(
      "The last export failed: the collector answered 500",
    );
  });

  it("reports how many spans were sent", () => {
    const one = describeExport(true, status({ lastSuccessAt: 0, exported: 1 }));
    expect(one).toContain("Sent 1 span;");
    expect(describeExport(true, status({ lastSuccessAt: 0, exported: 12 }))).toContain(
      "Sent 12 spans",
    );
  });
});
