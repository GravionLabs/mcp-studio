import { describe, expect, it } from "vitest";
import { activeTab, needsAzureLogin, needsTenant, visibleTabs } from "./server-detail.model";

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

describe("needsAzureLogin", () => {
  it("recognizes the messages of a missing Azure login", () => {
    expect(
      needsAzureLogin(
        'Azure sign-in failed: you are not signed in to Azure. Use "Sign in with Azure" on the server page, or run `az login --allow-no-subscriptions`',
      ),
    ).toBe(true);
    expect(needsAzureLogin("Please run 'az login' to set up an account")).toBe(true);
    expect(needsAzureLogin("ERROR: azd auth login needed")).toBe(true);
  });

  it("leaves other errors alone", () => {
    expect(needsAzureLogin(undefined)).toBe(false);
    expect(needsAzureLogin("connection refused")).toBe(false);
  });
});

describe("needsTenant", () => {
  it("is true for multi-factor errors only", () => {
    expect(needsTenant("your tenant requires multi-factor authentication. Use it")).toBe(true);
    expect(needsTenant("AADSTS50076: something")).toBe(true);
    expect(needsTenant("you are not signed in to Azure")).toBe(false);
    expect(needsTenant(undefined)).toBe(false);
  });
});
