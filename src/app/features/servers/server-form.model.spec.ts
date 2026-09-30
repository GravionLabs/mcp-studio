import { describe, expect, it } from "vitest";
import {
  emptyForm,
  formToInput,
  formatArgs,
  inputToForm,
  parseArgs,
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
    };
    expect(formToInput(inputToForm(input))).toEqual(input);
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
