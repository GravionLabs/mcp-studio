import { describe, expect, it } from "vitest";
import type { ServerDefinition } from "../../core/bindings";
import { DEMO_SERVER, findDemoServer, recentServers } from "./welcome.model";

function server(id: string, updatedAt: number, overrides: Partial<ServerDefinition> = {}) {
  return {
    id,
    name: id,
    transport: "stdio",
    command: "x",
    args: [],
    env: {},
    cwd: null,
    url: null,
    headers: {},
    tags: [],
    createdAt: 1,
    updatedAt,
    ...overrides,
  } satisfies ServerDefinition;
}

describe("recentServers", () => {
  it("lists the most recently changed servers first", () => {
    const list = [server("a", 10), server("b", 30), server("c", 20)];
    expect(recentServers(list).map((s) => s.id)).toEqual(["b", "c", "a"]);
  });

  it("cuts the list off and leaves the input alone", () => {
    const list = Array.from({ length: 8 }, (_, i) => server(`s${i}`, i));
    expect(recentServers(list)).toHaveLength(5);
    expect(recentServers(list, 2).map((s) => s.id)).toEqual(["s7", "s6"]);
    expect(list[0].id).toBe("s0");
  });

  it("is empty without servers", () => {
    expect(recentServers([])).toEqual([]);
  });
});

describe("findDemoServer", () => {
  it("finds the demo server the user added before", () => {
    const demo = server("demo", 1, {
      name: DEMO_SERVER.name,
      command: DEMO_SERVER.command,
      args: DEMO_SERVER.args,
    });
    expect(findDemoServer([server("other", 1), demo])).toBe(demo);
  });

  it("does not mistake a server that only has the same name", () => {
    expect(findDemoServer([server("x", 1, { name: DEMO_SERVER.name })])).toBeUndefined();
    expect(findDemoServer([])).toBeUndefined();
  });
});
