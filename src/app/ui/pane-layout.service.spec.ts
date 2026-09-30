import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import { KEY_VALUE_STORE, memoryStore } from "../core/storage";
import {
  INSPECTOR_LIMITS,
  PaneLayoutService,
  SIDEBAR_LIMITS,
  clampWidth,
} from "./pane-layout.service";

function create(initial: Record<string, string> = {}) {
  const store = memoryStore(initial);
  const injector = Injector.create({ providers: [{ provide: KEY_VALUE_STORE, useValue: store }] });
  return { store, service: runInInjectionContext(injector, () => new PaneLayoutService()) };
}

describe("clampWidth", () => {
  it("keeps values inside the range", () => {
    expect(clampWidth(300, SIDEBAR_LIMITS)).toBe(300);
  });
  it("clamps below and above", () => {
    expect(clampWidth(10, SIDEBAR_LIMITS)).toBe(SIDEBAR_LIMITS.min);
    expect(clampWidth(9999, SIDEBAR_LIMITS)).toBe(SIDEBAR_LIMITS.max);
  });
  it("handles NaN and rounds fractions", () => {
    expect(clampWidth(Number.NaN, INSPECTOR_LIMITS)).toBe(INSPECTOR_LIMITS.min);
    expect(clampWidth(300.6, SIDEBAR_LIMITS)).toBe(301);
  });
});

describe("PaneLayoutService", () => {
  it("starts with defaults and an open inspector", () => {
    const { service } = create();
    expect(service.sidebarWidth()).toBe(SIDEBAR_LIMITS.initial);
    expect(service.inspectorOpen()).toBe(true);
  });

  it("persists and restores widths and visibility", () => {
    const first = create();
    first.service.setSidebarWidth(300);
    first.service.setInspectorWidth(500);
    first.service.toggleInspector();
    const second = create({ "mcp-studio.layout": first.store.get("mcp-studio.layout") ?? "" });
    expect(second.service.sidebarWidth()).toBe(300);
    expect(second.service.inspectorWidth()).toBe(500);
    expect(second.service.inspectorOpen()).toBe(false);
  });

  it("falls back on corrupt storage", () => {
    const { service } = create({ "mcp-studio.layout": "{not json" });
    expect(service.sidebarWidth()).toBe(SIDEBAR_LIMITS.initial);
  });
});
