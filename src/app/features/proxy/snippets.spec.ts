import { describe, expect, it } from "vitest";
import { desktopConfigPath, safeName, snippetsFor } from "./snippets";

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

describe("snippetsFor", () => {
  const stdio = {
    serverName: "My Server",
    transport: "stdio" as const,
    proxyBinary: "/opt/mcp studio/mcp-studio-proxy",
    proxyUrl: null,
  };

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

describe("desktopConfigPath", () => {
  it("returns the platform specific location", () => {
    expect(desktopConfigPath("MacIntel")).toContain("Library/Application Support");
    expect(desktopConfigPath("Win32")).toContain("%APPDATA%");
    expect(desktopConfigPath("Linux x86_64")).toContain(".config/Claude");
  });
});
