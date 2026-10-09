import { describe, expect, it } from "vitest";
import { activeTab, visibleTabs } from "./server-detail.model";

describe("visibleTabs", () => {
  it("shows every tab while connected", () => {
    expect(visibleTabs(true).map((tab) => tab.id)).toEqual([
      "explorer",
      "context",
      "quality",
      "docs",
      "settings",
      "client",
      "logs",
    ]);
  });

  it("hides the tabs that read the server's tools while disconnected", () => {
    expect(visibleTabs(false).map((tab) => tab.id)).toEqual(["settings", "client", "logs"]);
  });
});

describe("activeTab", () => {
  it("opens the explorer when connected and nothing is picked", () => {
    expect(activeTab(null, true)).toBe("explorer");
  });

  it("opens the settings when disconnected and nothing is picked", () => {
    expect(activeTab(null, false)).toBe("settings");
  });

  it("keeps the picked tab while it is shown", () => {
    expect(activeTab("logs", false)).toBe("logs");
    expect(activeTab("quality", true)).toBe("quality");
  });

  it("falls back to the settings when the picked tab needs a connection that is gone", () => {
    expect(activeTab("quality", false)).toBe("settings");
  });
});
