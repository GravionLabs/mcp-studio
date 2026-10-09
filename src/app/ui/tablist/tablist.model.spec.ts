import { describe, expect, it } from "vitest";
import { TabElement, TabKeyEvent, handleTabKey, nextTabIndex } from "./tablist.model";

describe("nextTabIndex", () => {
  it("moves right and wraps around", () => {
    expect(nextTabIndex("ArrowRight", 0, 3)).toBe(1);
    expect(nextTabIndex("ArrowRight", 2, 3)).toBe(0);
  });

  it("moves left and wraps around", () => {
    expect(nextTabIndex("ArrowLeft", 2, 3)).toBe(1);
    expect(nextTabIndex("ArrowLeft", 0, 3)).toBe(2);
  });

  it("jumps to the ends", () => {
    expect(nextTabIndex("Home", 2, 4)).toBe(0);
    expect(nextTabIndex("End", 0, 4)).toBe(3);
  });

  it("ignores other keys and empty lists", () => {
    expect(nextTabIndex("a", 0, 3)).toBeNull();
    expect(nextTabIndex("Enter", 0, 3)).toBeNull();
    expect(nextTabIndex("ArrowRight", 0, 0)).toBeNull();
  });
});

describe("handleTabKey", () => {
  function setup(selected = 0) {
    const log: string[] = [];
    const tabs: TabElement[] = ["one", "two", "three"].map((name) => ({
      focus: () => log.push(`focus ${name}`),
      click: () => log.push(`click ${name}`),
    }));
    const press = (
      key: string,
      modifiers: Partial<TabKeyEvent> = {},
      target: unknown = tabs[selected],
    ) => {
      const event: TabKeyEvent = {
        key,
        target,
        altKey: false,
        ctrlKey: false,
        metaKey: false,
        preventDefault: () => log.push("prevented"),
        ...modifiers,
      };
      return handleTabKey(event, tabs);
    };
    return { log, press };
  }

  it("focuses and selects the next tab and stops the browser from scrolling", () => {
    const { log, press } = setup();
    expect(press("ArrowRight")).toBe(true);
    expect(log).toEqual(["prevented", "focus two", "click two"]);
  });

  it("wraps from the last tab to the first and back", () => {
    const last = setup(2);
    last.press("ArrowRight");
    expect(last.log).toEqual(["prevented", "focus one", "click one"]);
    const first = setup(0);
    first.press("ArrowLeft");
    expect(first.log).toEqual(["prevented", "focus three", "click three"]);
  });

  it("goes to the first and last tab with Home and End", () => {
    const home = setup(2);
    home.press("Home");
    expect(home.log).toEqual(["prevented", "focus one", "click one"]);
    const end = setup(0);
    end.press("End");
    expect(end.log).toEqual(["prevented", "focus three", "click three"]);
  });

  it("does nothing for other keys, shortcuts, or events from inside a tab", () => {
    const { log, press } = setup();
    expect(press("Enter")).toBe(false);
    expect(press("ArrowRight", { altKey: true })).toBe(false);
    expect(press("ArrowRight", { ctrlKey: true })).toBe(false);
    expect(press("ArrowRight", { metaKey: true })).toBe(false);
    expect(press("ArrowRight", {}, {})).toBe(false);
    expect(log).toEqual([]);
  });
});
