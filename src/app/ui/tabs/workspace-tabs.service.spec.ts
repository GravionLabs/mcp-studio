import { describe, expect, it } from "vitest";
import { WorkspaceTabsService } from "./workspace-tabs.service";

const tab = (id: string) => ({ id, title: id.toUpperCase(), route: `/${id}` });

describe("WorkspaceTabsService", () => {
  it("opens and activates tabs", () => {
    const service = new WorkspaceTabsService();
    service.open(tab("a"));
    service.open(tab("b"));
    expect(service.tabs().map((t) => t.id)).toEqual(["a", "b"]);
    expect(service.activeId()).toBe("b");
  });

  it("does not duplicate a tab, but refreshes it", () => {
    const service = new WorkspaceTabsService();
    service.open(tab("a"));
    service.open({ id: "a", title: "Renamed", route: "/x" });
    expect(service.tabs()).toHaveLength(1);
    expect(service.active()).toMatchObject({ title: "Renamed", route: "/x" });
  });

  it("activates the left neighbour when the active tab closes", () => {
    const service = new WorkspaceTabsService();
    ["a", "b", "c"].forEach((id) => service.open(tab(id)));
    service.close("c");
    expect(service.activeId()).toBe("b");
    service.close("a");
    expect(service.activeId()).toBe("b");
    service.close("b");
    expect(service.activeId()).toBeNull();
  });

  it("closing the first active tab activates the next one", () => {
    const service = new WorkspaceTabsService();
    ["a", "b"].forEach((id) => service.open(tab(id)));
    service.activate("a");
    service.close("a");
    expect(service.activeId()).toBe("b");
  });

  it("ignores unknown ids", () => {
    const service = new WorkspaceTabsService();
    service.open(tab("a"));
    service.activate("zzz");
    service.close("zzz");
    expect(service.activeId()).toBe("a");
  });
});
