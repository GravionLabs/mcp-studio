import type { ServerDefinition, ServerInput } from "../../core/bindings";

/** The reference server of the MCP project; it offers tools, resources and prompts to try out. */
export const DEMO_SERVER: ServerInput = {
  name: "Everything (demo)",
  transport: "stdio",
  command: "npx",
  args: ["-y", "@modelcontextprotocol/server-everything"],
  env: {},
  cwd: null,
  url: null,
  headers: {},
  tags: ["demo"],
};

/** How many servers the welcome page lists. */
export const RECENT_LIMIT = 5;

/** The servers that were added or edited last, newest first. */
export function recentServers(
  servers: readonly ServerDefinition[],
  limit = RECENT_LIMIT,
): ServerDefinition[] {
  return [...servers].sort((a, b) => b.updatedAt - a.updatedAt).slice(0, limit);
}

/** The server that is already the demo server, if the user added it before. */
export function findDemoServer(servers: readonly ServerDefinition[]): ServerDefinition | undefined {
  return servers.find(
    (s) =>
      s.name === DEMO_SERVER.name &&
      s.command === DEMO_SERVER.command &&
      s.args.join(" ") === DEMO_SERVER.args.join(" "),
  );
}
