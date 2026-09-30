import { describe, expect, it } from "vitest";
import { proxyCommand, shellQuote } from "./proxy.model";

describe("shellQuote", () => {
  it("leaves safe words alone", () => {
    expect(shellQuote("/usr/bin/mcp-studio-proxy")).toBe("/usr/bin/mcp-studio-proxy");
    expect(shellQuote("--server")).toBe("--server");
  });

  it("quotes spaces and single quotes", () => {
    expect(shellQuote("My Server")).toBe("'My Server'");
    expect(shellQuote("it's")).toBe(`'it'\\''s'`);
    expect(shellQuote("")).toBe("''");
  });
});

describe("proxyCommand", () => {
  it("builds the command line", () => {
    expect(proxyCommand("/opt/studio/proxy", "GitHub MCP")).toBe(
      "/opt/studio/proxy --server 'GitHub MCP'",
    );
  });
});
