import { Injector, runInInjectionContext } from "@angular/core";
import { describe, expect, it } from "vitest";
import { KEY_VALUE_STORE, memoryStore } from "./storage";
import { ThemeService } from "./theme.service";

function create(initial: Record<string, string> = {}) {
  const store = memoryStore(initial);
  const injector = Injector.create({ providers: [{ provide: KEY_VALUE_STORE, useValue: store }] });
  return { store, service: runInInjectionContext(injector, () => new ThemeService()) };
}

describe("ThemeService", () => {
  it("defaults to dark", () => {
    expect(create().service.theme()).toBe("dark");
  });

  it("restores a saved light theme", () => {
    expect(create({ "mcp-studio.theme": "light" }).service.theme()).toBe("light");
  });

  it("ignores garbage in storage", () => {
    expect(create({ "mcp-studio.theme": "purple" }).service.theme()).toBe("dark");
  });

  it("toggles and persists", () => {
    const { service, store } = create();
    service.toggle();
    expect(service.theme()).toBe("light");
    expect(store.get("mcp-studio.theme")).toBe("light");
    service.toggle();
    expect(service.isDark()).toBe(true);
  });
});
