import { describe, expect, it } from "vitest";
import {
  emptyForm,
  formToInput,
  formToInputWithSecrets,
  formatArgs,
  inputToForm,
  isRoot,
  parseArgs,
  parseRoots,
  validateForm,
} from "./server-form.model";

describe("parseArgs", () => {
  it("splits on whitespace", () => {
    expect(parseArgs("-y  server-everything")).toEqual(["-y", "server-everything"]);
  });
  it("keeps quoted groups together", () => {
    expect(parseArgs(`--name "my server" 'a b'`)).toEqual(["--name", "my server", "a b"]);
  });
  it("supports escapes and empty quoted arguments", () => {
    expect(parseArgs(String.raw`a\ b ""`)).toEqual(["a b", ""]);
    expect(parseArgs(String.raw`"say \"hi\""`)).toEqual([`say "hi"`]);
  });
  it("handles empty input and unterminated quotes", () => {
    expect(parseArgs("   ")).toEqual([]);
    expect(parseArgs(`"open end`)).toEqual(["open end"]);
  });
});

describe("formatArgs", () => {
  it("roundtrips through parseArgs", () => {
    const args = ["-y", "with space", "", `q"uote`, "back\\slash"];
    expect(parseArgs(formatArgs(args))).toEqual(args);
  });
});

describe("form conversion", () => {
  it("builds a stdio input", () => {
    const form = {
      ...emptyForm(),
      name: " Everything ",
      command: "npx",
      args: "-y pkg",
      env: [
        { key: "DEBUG", value: "1" },
        { key: " ", value: "ignored" },
      ],
      tags: "dev, , test",
    };
    expect(formToInput(form)).toMatchObject({
      name: "Everything",
      transport: "stdio",
      command: "npx",
      args: ["-y", "pkg"],
      env: { DEBUG: "1" },
      cwd: null,
      tags: ["dev", "test"],
    });
  });

  it("roundtrips an http input", () => {
    const input = {
      name: "Remote",
      transport: "http" as const,
      command: null,
      args: [],
      env: {},
      cwd: null,
      url: "https://example.com/mcp",
      headers: { Authorization: "Bearer x" },
      tags: ["prod"],
      oauth: true,
      oauthClientId: "app-id",
      oauthScopes: "api://x/.default",
      oauthCallbackPort: 3118,
      azureCredentials: false,
      roots: ["/home/me/app", "file:///srv/data"],
    };
    expect(formToInput(inputToForm(input))).toEqual(input);
  });
});

describe("roots", () => {
  const form = { ...emptyForm(), name: "n", command: "c" };

  it("are one per line, trimmed, without empty lines", () => {
    expect(parseRoots(" /a \r\n\n/b\n")).toEqual(["/a", "/b"]);
    expect(formToInput({ ...form, roots: "/a\n/b" }).roots).toEqual(["/a", "/b"]);
  });

  it("accept absolute paths and file addresses", () => {
    for (const ok of [
      "/home/me",
      "C:\\Users\\me",
      "C:/Users/me",
      "\\\\host\\share",
      "file:///srv",
    ]) {
      expect(isRoot(ok), ok).toBe(true);
    }
  });

  it("refuse relative paths and other addresses", () => {
    for (const bad of ["src", "./src", "https://example.com", "file://"]) {
      expect(isRoot(bad), bad).toBe(false);
    }
    expect(validateForm({ ...form, roots: "/ok\nrelative" })).toEqual([
      'Root "relative" must be an absolute folder path or a file:// address.',
    ]);
  });

  it("default to none for servers stored before the option existed", () => {
    const stored = formToInput(form);
    const legacy = { ...stored } as Record<string, unknown>;
    delete legacy["roots"];
    expect(inputToForm(legacy as unknown as typeof stored).roots).toBe("");
  });
});

describe("azure credentials", () => {
  const azure = {
    ...emptyForm("http"),
    name: "Azure DevOps",
    url: "https://mcp.dev.azure.com/org",
    azureCredentials: true,
    oauthScopes: " https://mcp.dev.azure.com/.default ",
  };

  it("keeps the scope and drops the OAuth client settings", () => {
    expect(formToInput({ ...azure, oauthClientId: "x", oauthCallbackPort: "3118" })).toMatchObject({
      oauth: false,
      azureCredentials: true,
      oauthScopes: "https://mcp.dev.azure.com/.default",
      oauthClientId: null,
      oauthCallbackPort: null,
    });
  });

  it("is only kept for HTTP servers and roundtrips", () => {
    expect(formToInput({ ...azure, transport: "stdio", command: "x" }).azureCredentials).toBe(
      false,
    );
    const input = formToInput(azure);
    expect(inputToForm(input).azureCredentials).toBe(true);
    expect(inputToForm(input).oauthScopes).toBe("https://mcp.dev.azure.com/.default");
  });

  it("cannot be combined with OAuth", () => {
    expect(validateForm({ ...azure, oauth: true })).toHaveLength(1);
    expect(validateForm(azure)).toEqual([]);
  });

  it("defaults to off for servers stored before the option existed", () => {
    const stored = formToInput({ ...emptyForm(), name: "n", command: "c" });
    const legacy = { ...stored } as Record<string, unknown>;
    delete legacy["azureCredentials"];
    expect(inputToForm(legacy as unknown as typeof stored).azureCredentials).toBe(false);
  });
});

describe("oauth", () => {
  it("is only kept for HTTP servers", () => {
    const http = { ...emptyForm("http"), name: "r", url: "https://x.test", oauth: true };
    expect(formToInput(http).oauth).toBe(true);
    expect(formToInput({ ...http, transport: "stdio", command: "x" }).oauth).toBe(false);
  });

  it("keeps the client settings only with OAuth and rejects a bad port", () => {
    const entra = {
      ...emptyForm("http"),
      name: "r",
      url: "https://x.test",
      oauth: true,
      oauthClientId: " app ",
      oauthCallbackPort: "3118",
    };
    expect(formToInput(entra)).toMatchObject({
      oauthClientId: "app",
      oauthScopes: null,
      oauthCallbackPort: 3118,
    });
    expect(formToInput({ ...entra, oauth: false }).oauthClientId).toBeNull();
    expect(validateForm({ ...entra, oauthCallbackPort: "99999" })).toHaveLength(1);
    expect(validateForm(entra)).toEqual([]);
  });

  it("defaults to off for servers stored before the option existed", () => {
    const stored = formToInput({ ...emptyForm(), name: "n", command: "c" });
    const legacy = { ...stored } as Record<string, unknown>;
    delete legacy["oauth"];
    expect(inputToForm(legacy as unknown as typeof stored).oauth).toBe(false);
  });
});

describe("validateForm", () => {
  it("requires name and command for stdio", () => {
    expect(validateForm(emptyForm())).toEqual([
      "Name is required.",
      "Command is required for stdio servers.",
    ]);
  });
  it("accepts a valid stdio form", () => {
    expect(validateForm({ ...emptyForm(), name: "a", command: "node" })).toEqual([]);
  });
  it("checks http URLs", () => {
    const base = { ...emptyForm("http"), name: "r" };
    expect(validateForm(base)).toEqual(["URL is required for HTTP servers."]);
    expect(validateForm({ ...base, url: "ftp://x" })).toHaveLength(1);
    expect(validateForm({ ...base, url: "not a url" })).toHaveLength(1);
    expect(validateForm({ ...base, url: "http://localhost:3000/mcp" })).toEqual([]);
  });
});

describe("secrets in the form", () => {
  it("writes new secret values to the keyring and stores only references", () => {
    const form = {
      ...emptyForm(),
      name: "s",
      command: "x",
      env: [
        { key: "PLAIN", value: "visible" },
        { key: "TOKEN", value: "s3cr3t", secret: true },
      ],
    };
    const { input, writes } = formToInputWithSecrets(form, () => "generated");
    expect(input.env).toEqual({ PLAIN: "visible", TOKEN: "keyring:generated" });
    expect(writes).toEqual([{ name: "generated", value: "s3cr3t" }]);
    expect(JSON.stringify(input)).not.toContain("s3cr3t");
  });

  it("keeps a stored secret when the value is left empty", () => {
    const form = {
      ...emptyForm("http"),
      name: "r",
      url: "https://x.test",
      headers: [{ key: "Authorization", value: "", secret: true, stored: "keyring:abc" }],
    };
    const { input, writes } = formToInputWithSecrets(form);
    expect(input.headers).toEqual({ Authorization: "keyring:abc" });
    expect(writes).toEqual([]);
  });

  it("replaces a stored secret under the same name", () => {
    const form = {
      ...emptyForm("http"),
      headers: [{ key: "A", value: "new", secret: true, stored: "keyring:abc" }],
    };
    expect(formToInputWithSecrets(form).writes).toEqual([{ name: "abc", value: "new" }]);
  });

  it("drops secret rows that have neither a value nor a stored secret", () => {
    const form = { ...emptyForm(), env: [{ key: "EMPTY", value: "", secret: true }] };
    expect(formToInput(form).env).toEqual({});
  });

  it("shows stored references as secret rows without exposing the reference as value", () => {
    const form = inputToForm({
      ...formToInput({ ...emptyForm(), name: "n", command: "c" }),
      env: { TOKEN: "keyring:abc", PLAIN: "v" },
    });
    expect(form.env).toEqual([
      { key: "TOKEN", value: "", secret: true, stored: "keyring:abc" },
      { key: "PLAIN", value: "v" },
    ]);
  });
});
