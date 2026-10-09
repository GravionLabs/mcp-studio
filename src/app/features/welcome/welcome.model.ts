import type { ServerDefinition, ServerInput } from "../../core/bindings";

const DEMO_TAGS = ["demo"];

/** The reference server that ships with the app (echo, add and a few more tools to try out). */
export function bundledDemoServer(path: string): ServerInput {
  return {
    name: "MCP Studio test server",
    transport: "stdio",
    command: path,
    args: [],
    env: {},
    cwd: null,
    url: null,
    headers: {},
    tags: DEMO_TAGS,
  };
}

/** The reference server of the MCP project, for installations without the bundled one. */
export const NPX_DEMO_SERVER: ServerInput = {
  name: "Everything (demo)",
  transport: "stdio",
  command: "npx",
  args: ["-y", "@modelcontextprotocol/server-everything"],
  env: {},
  cwd: null,
  url: null,
  headers: {},
  tags: DEMO_TAGS,
};

/** The demo server to add: the bundled one when the app found it, otherwise the one through npx. */
export function demoServerInput(bundledPath: string | null): ServerInput {
  return bundledPath ? bundledDemoServer(bundledPath) : NPX_DEMO_SERVER;
}

/** How many servers the welcome page lists. */
export const RECENT_LIMIT = 5;

/** The servers that were added or edited last, newest first. */
export function recentServers(
  servers: readonly ServerDefinition[],
  limit = RECENT_LIMIT,
): ServerDefinition[] {
  return [...servers].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, limit);
}

/** The server that is already this demo server, if the user added it before. */
export function findDemoServer(
  servers: readonly ServerDefinition[],
  demo: ServerInput,
): ServerDefinition | undefined {
  return servers.find(
    (s) =>
      s.name === demo.name &&
      s.command === demo.command &&
      s.args.join(" ") === demo.args.join(" "),
  );
}
