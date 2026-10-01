import { shellQuote } from "./proxy.model";

export type ClientKind = "claude" | "github" | "opencode";

export const CLIENTS: readonly { id: ClientKind; label: string }[] = [
  { id: "claude", label: "Claude" },
  { id: "github", label: "GitHub Copilot" },
  { id: "opencode", label: "OpenCode" },
];

export interface SnippetInput {
  serverName: string;
  transport: "stdio" | "http";
  /** Absolute path of the `mcp-studio-proxy` program (stdio servers). */
  proxyBinary: string | null;
  /** Local HTTP proxy URL for the server (HTTP servers). */
  proxyUrl: string | null;
}

export interface Snippet {
  /** Short name of the setup variant, e.g. "Claude Code (.mcp.json)". */
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

function json(value: unknown): string {
  return JSON.stringify(value, null, 2);
}

function claudeSnippets(input: SnippetInput, entry: Record<string, unknown>): Snippet[] {
  const name = safeName(input.serverName);
  const file = json({ mcpServers: { [name]: entry } });
  const cli =
    input.transport === "stdio"
      ? ["claude", "mcp", "add", name, "--", input.proxyBinary ?? "", "--server", input.serverName]
      : ["claude", "mcp", "add", "--transport", "http", name, input.proxyUrl ?? ""];
  return [
    {
      title: "Claude Code (command)",
      hint: "Run in a terminal inside your project.",
      language: "shell",
      text: cli.map(shellQuote).join(" "),
    },
    {
      title: "Claude Code (.mcp.json)",
      hint: "Merge into .mcp.json in your project root.",
      language: "json",
      text: file,
    },
    {
      title: "Claude Desktop",
      hint: "Merge into claude_desktop_config.json (Settings → Developer → Edit Config), then restart.",
      language: "json",
      text: file,
    },
  ];
}

function githubSnippets(input: SnippetInput, entry: Record<string, unknown>): Snippet[] {
  const name = safeName(input.serverName);
  const stdio = input.transport === "stdio";
  return [
    {
      title: "VS Code (.vscode/mcp.json)",
      hint: "Merge into .vscode/mcp.json in your project, or into the user mcp.json (MCP: Open User Configuration).",
      language: "json",
      text: json({ servers: { [name]: stdio ? { type: "stdio", ...entry } : entry } }),
    },
    {
      title: "Copilot CLI (mcp-config.json)",
      hint: "Merge into ~/.copilot/mcp-config.json, then restart the CLI.",
      language: "json",
      text: json({
        mcpServers: {
          [name]: stdio ? { type: "local", ...entry, tools: ["*"] } : { ...entry, tools: ["*"] },
        },
      }),
    },
  ];
}

function opencodeSnippets(input: SnippetInput): Snippet[] {
  const name = safeName(input.serverName);
  const entry =
    input.transport === "stdio"
      ? {
          type: "local",
          command: [input.proxyBinary ?? "", "--server", input.serverName],
          enabled: true,
        }
      : { type: "remote", url: input.proxyUrl ?? "", enabled: true };
  return [
    {
      title: "opencode.json",
      hint: "Merge into opencode.json in your project root, or into ~/.config/opencode/opencode.json.",
      language: "json",
      text: json({ $schema: "https://opencode.ai/config.json", mcp: { [name]: entry } }),
    },
  ];
}

/**
 * Configuration text that routes the chosen client through MCP Studio for the given server, or an
 * empty list when the proxy is not available (e.g. the proxy program was not found).
 */
export function snippetsFor(input: SnippetInput, client: ClientKind = "claude"): Snippet[] {
  const entry = serverEntry(input);
  if (!entry) return [];
  switch (client) {
    case "claude":
      return claudeSnippets(input, entry);
    case "github":
      return githubSnippets(input, entry);
    case "opencode":
      return opencodeSnippets(input);
  }
}

/** Where the client config files live, for the hint text. */
export function desktopConfigPath(platform: string): string {
  if (platform.startsWith("Mac"))
    return "~/Library/Application Support/Claude/claude_desktop_config.json";
  if (platform.startsWith("Win")) return "%APPDATA%\\Claude\\claude_desktop_config.json";
  return "~/.config/Claude/claude_desktop_config.json";
}
