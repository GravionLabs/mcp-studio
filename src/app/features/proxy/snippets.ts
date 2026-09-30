import { shellQuote } from "./proxy.model";

export type ClientKind = "claude-code" | "claude-desktop" | "generic";

export interface SnippetInput {
  serverName: string;
  transport: "stdio" | "http";
  /** Absolute path of the `mcp-studio-proxy` program (stdio servers). */
  proxyBinary: string | null;
  /** Local HTTP proxy URL for the server (HTTP servers). */
  proxyUrl: string | null;
}

export interface Snippet {
  /** Short name of the target, e.g. "Claude Code (.mcp.json)". */
  title: string;
  /** Where to put the text. */
  hint: string;
  /** `json` for file contents, `shell` for a command line. */
  language: "json" | "shell";
  text: string;
}

/** A name that is safe as a config key and as a CLI argument. */
export function safeName(name: string): string {
  const cleaned = name
    .trim()
    .replace(/[^A-Za-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return cleaned === "" ? "mcp-server" : cleaned;
}

function serverEntry(input: SnippetInput): Record<string, unknown> | null {
  if (input.transport === "stdio") {
    if (!input.proxyBinary) return null;
    return { command: input.proxyBinary, args: ["--server", input.serverName] };
  }
  if (!input.proxyUrl) return null;
  return { type: "http", url: input.proxyUrl };
}

function configFile(input: SnippetInput): string | null {
  const entry = serverEntry(input);
  if (!entry) return null;
  return JSON.stringify({ mcpServers: { [safeName(input.serverName)]: entry } }, null, 2);
}

/**
 * Configuration text that routes a client through MCP Studio for the given server, or an empty list
 * when the proxy is not available (e.g. the proxy program was not found).
 */
export function snippetsFor(input: SnippetInput): Snippet[] {
  const file = configFile(input);
  if (!file) return [];
  const name = safeName(input.serverName);
  const snippets: Snippet[] = [];

  const cli =
    input.transport === "stdio"
      ? ["claude", "mcp", "add", name, "--", input.proxyBinary ?? "", "--server", input.serverName]
      : ["claude", "mcp", "add", "--transport", "http", name, input.proxyUrl ?? ""];
  snippets.push({
    title: "Claude Code (command)",
    hint: "Run in a terminal inside your project.",
    language: "shell",
    text: cli.map(shellQuote).join(" "),
  });
  snippets.push({
    title: "Claude Code (.mcp.json)",
    hint: "Merge into .mcp.json in your project root.",
    language: "json",
    text: file,
  });
  snippets.push({
    title: "Claude Desktop",
    hint: "Merge into claude_desktop_config.json (Settings → Developer → Edit Config), then restart.",
    language: "json",
    text: file,
  });
  return snippets;
}

/** Where the client config files live, for the hint text. */
export function desktopConfigPath(platform: string): string {
  if (platform.startsWith("Mac"))
    return "~/Library/Application Support/Claude/claude_desktop_config.json";
  if (platform.startsWith("Win")) return "%APPDATA%\\Claude\\claude_desktop_config.json";
  return "~/.config/Claude/claude_desktop_config.json";
}
