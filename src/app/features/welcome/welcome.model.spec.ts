import { describe, expect, it } from "vitest";
import type { ServerDefinition } from "../../core/bindings";
import {
  NPX_DEMO_SERVER,
  bundledDemoServer,
  demoServerInput,
  findDemoServer,
  recentServers,
} from "./welcome.model";

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

describe("demoServerInput", () => {
  it("runs the bundled test server directly when the app found it", () => {
    const input = demoServerInput("/opt/mcp-studio/mcp-studio-testserver");
    expect(input).toEqual(bundledDemoServer("/opt/mcp-studio/mcp-studio-testserver"));
    expect(input.command).toBe("/opt/mcp-studio/mcp-studio-testserver");
    expect(input.args).toEqual([]);
    expect(input.transport).toBe("stdio");
  });

  it("falls back to the reference server through npx", () => {
    expect(demoServerInput(null)).toBe(NPX_DEMO_SERVER);
    expect(NPX_DEMO_SERVER.command).toBe("npx");
  });
});

describe("findDemoServer", () => {
  it("finds the demo server the user added before", () => {
    const demo = bundledDemoServer("/opt/test-server");
    const added = server("demo", 1, { name: demo.name, command: demo.command, args: demo.args });
    expect(findDemoServer([server("other", 1), added], demo)).toBe(added);
  });

  it("does not mistake a server that only has the same name or another location", () => {
    const demo = bundledDemoServer("/opt/test-server");
    expect(findDemoServer([server("x", 1, { name: demo.name })], demo)).toBeUndefined();
    const elsewhere = server("y", 1, {
      name: demo.name,
      command: "/old/location/test-server",
      args: [],
    });
    expect(findDemoServer([elsewhere], demo)).toBeUndefined();
    expect(findDemoServer([], demo)).toBeUndefined();
  });
});
