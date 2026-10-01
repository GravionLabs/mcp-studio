import { describe, expect, it } from "vitest";
import { CLIENTS, desktopConfigPath, safeName, snippetsFor } from "./snippets";

describe("safeName", () => {
  it("keeps names usable as keys and CLI arguments", () => {
    expect(safeName("GitHub MCP")).toBe("GitHub-MCP");
    expect(safeName("  a/b:c  ")).toBe("a-b-c");
    expect(safeName("ok_name-1")).toBe("ok_name-1");
  });

  it("never returns an empty name", () => {
    expect(safeName("///")).toBe("mcp-server");
    expect(safeName("")).toBe("mcp-server");
  });
});

const stdio = {
  serverName: "My Server",
  transport: "stdio" as const,
  proxyBinary: "/opt/mcp studio/mcp-studio-proxy",
  proxyUrl: null,
};

describe("snippetsFor", () => {
  it("builds command and config snippets for stdio servers", () => {
    const [cli, mcpJson, desktop] = snippetsFor(stdio);
    expect(cli?.text).toBe(
      "claude mcp add My-Server -- '/opt/mcp studio/mcp-studio-proxy' --server 'My Server'",
    );
    expect(cli?.language).toBe("shell");
    const parsed = JSON.parse(mcpJson?.text ?? "{}") as Record<string, Record<string, unknown>>;
    expect(parsed["mcpServers"]?.["My-Server"]).toEqual({
      command: "/opt/mcp studio/mcp-studio-proxy",
      args: ["--server", "My Server"],
    });
    expect(desktop?.text).toBe(mcpJson?.text);
  });

  it("points HTTP servers at the local proxy URL", () => {
    const snippets = snippetsFor({
      serverName: "Remote",
      transport: "http",
      proxyBinary: null,
      proxyUrl: "http://127.0.0.1:38465/mcp/Remote",
    });
    expect(snippets[0]?.text).toBe(
      "claude mcp add --transport http Remote http://127.0.0.1:38465/mcp/Remote",
    );
    const parsed = JSON.parse(snippets[1]?.text ?? "{}") as Record<string, Record<string, unknown>>;
    expect(parsed["mcpServers"]?.["Remote"]).toEqual({
      type: "http",
      url: "http://127.0.0.1:38465/mcp/Remote",
    });
  });

  it("returns nothing when the proxy is unavailable", () => {
    expect(snippetsFor({ ...stdio, proxyBinary: null })).toEqual([]);
    expect(
      snippetsFor({ serverName: "x", transport: "http", proxyBinary: null, proxyUrl: null }),
    ).toEqual([]);
  });
});

type Parsed = Record<string, Record<string, unknown>>;
const parse = (text: string | undefined): Parsed => JSON.parse(text ?? "{}") as Parsed;

const http = {
  serverName: "Remote",
  transport: "http" as const,
  proxyBinary: null,
  proxyUrl: "http://127.0.0.1:38465/mcp/Remote",
};

describe("snippetsFor GitHub Copilot", () => {
  it("builds VS Code and Copilot CLI configs for stdio servers", () => {
    const [vscode, cli] = snippetsFor(stdio, "github");
    expect(vscode?.title).toContain("VS Code");
    expect(parse(vscode?.text)["servers"]?.["My-Server"]).toEqual({
      type: "stdio",
      command: "/opt/mcp studio/mcp-studio-proxy",
      args: ["--server", "My Server"],
    });
    expect(cli?.title).toContain("Copilot CLI");
    expect(parse(cli?.text)["mcpServers"]?.["My-Server"]).toEqual({
      type: "local",
      command: "/opt/mcp studio/mcp-studio-proxy",
      args: ["--server", "My Server"],
      tools: ["*"],
    });
  });

  it("points HTTP servers at the local proxy URL", () => {
    const [vscode, cli] = snippetsFor(http, "github");
    expect(parse(vscode?.text)["servers"]?.["Remote"]).toEqual({
      type: "http",
      url: "http://127.0.0.1:38465/mcp/Remote",
    });
    expect(parse(cli?.text)["mcpServers"]?.["Remote"]).toEqual({
      type: "http",
      url: "http://127.0.0.1:38465/mcp/Remote",
      tools: ["*"],
    });
  });
});

describe("snippetsFor OpenCode", () => {
  it("uses a local server with a command array for stdio servers", () => {
    const snippets = snippetsFor(stdio, "opencode");
    expect(snippets).toHaveLength(1);
    const parsed = parse(snippets[0]?.text);
    expect(parsed["$schema"]).toBe("https://opencode.ai/config.json");
    expect(parsed["mcp"]?.["My-Server"]).toEqual({
      type: "local",
      command: ["/opt/mcp studio/mcp-studio-proxy", "--server", "My Server"],
      enabled: true,
    });
  });

  it("uses a remote server for HTTP servers", () => {
    const parsed = parse(snippetsFor(http, "opencode")[0]?.text);
    expect(parsed["mcp"]?.["Remote"]).toEqual({
      type: "remote",
      url: "http://127.0.0.1:38465/mcp/Remote",
      enabled: true,
    });
  });
});

describe("client selection", () => {
  it("defaults to Claude and lists the three clients", () => {
    expect(snippetsFor(stdio)).toEqual(snippetsFor(stdio, "claude"));
    expect(CLIENTS.map((c) => c.id)).toEqual(["claude", "github", "opencode"]);
  });

  it("returns nothing for any client when the proxy is unavailable", () => {
    for (const { id } of CLIENTS) {
      expect(snippetsFor({ ...stdio, proxyBinary: null }, id)).toEqual([]);
    }
  });
});

describe("desktopConfigPath", () => {
  it("returns the platform specific location", () => {
    expect(desktopConfigPath("MacIntel")).toContain("Library/Application Support");
    expect(desktopConfigPath("Win32")).toContain("%APPDATA%");
    expect(desktopConfigPath("Linux x86_64")).toContain(".config/Claude");
  });
});
