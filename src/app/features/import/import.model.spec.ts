import { describe, expect, it } from "vitest";
import type { ImportCandidate } from "../../core/bindings";
import {
  defaultSelection,
  describeCandidate,
  secretCount,
  selectable,
  selectedInputs,
} from "./import.model";

const candidate = (overrides: Partial<ImportCandidate> = {}, input = {}): ImportCandidate => ({
  input: {
    name: "srv",
    transport: "stdio",
    command: "npx",
    args: ["-y", "pkg"],
    env: {},
    cwd: null,
    url: null,
    headers: {},
    tags: [],
    oauth: false,
    oauthClientId: null,
    oauthScopes: null,
    oauthCallbackPort: null,
    azureCredentials: false,
    ...input,
  },
  origin: "top level",
  duplicate: false,
  unsupported: null,
  ...overrides,
});

describe("selection", () => {
  const list = [
    candidate(),
    candidate({ duplicate: true }),
    candidate({ unsupported: "SSE" }),
    candidate(),
  ];

  it("checks new, supported entries by default", () => {
    expect([...defaultSelection(list)]).toEqual([0, 3]);
  });

  it("allows selecting duplicates but not unsupported entries", () => {
    expect(selectable(list)).toEqual([0, 1, 3]);
  });

  it("returns the inputs of the selected entries in order", () => {
    const inputs = selectedInputs(list, new Set([3, 0]));
    expect(inputs).toHaveLength(2);
    expect(selectedInputs(list, new Set())).toEqual([]);
  });
});

describe("describeCandidate", () => {
  it("shows the command line or the URL", () => {
    expect(describeCandidate(candidate())).toBe("npx -y pkg");
    expect(
      describeCandidate(
        candidate({}, { transport: "http", url: "https://x.test/mcp", command: null }),
      ),
    ).toBe("https://x.test/mcp");
  });
});

describe("secretCount", () => {
  it("counts secret-looking variables and headers", () => {
    expect(
      secretCount(
        candidate({}, { env: { API_KEY: "x", LOG: "y" }, headers: { Authorization: "z" } }),
      ),
    ).toBe(2);
    expect(secretCount(candidate())).toBe(0);
  });
});
