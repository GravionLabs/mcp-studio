import { describe, expect, it } from "vitest";
import type { ProviderTestResult } from "../../core/bindings";
import { GITHUB_MODELS_URL, describeTest, qualifiedModel } from "./providers.model";

describe("qualifiedModel", () => {
  it("leaves Anthropic models without a prefix", () => {
    expect(qualifiedModel("anthropic", " claude-sonnet-5-5 ")).toBe("claude-sonnet-5-5");
  });

  it("prefixes the other providers and keeps colons of Ollama tags", () => {
    expect(qualifiedModel("openai", "gpt-4o")).toBe("openai:gpt-4o");
    expect(qualifiedModel("ollama", "llama3.1:8b")).toBe("ollama:llama3.1:8b");
  });

  it("is empty without a model", () => {
    expect(qualifiedModel("ollama", "  ")).toBe("");
  });
});

describe("describeTest", () => {
  const result = (overrides: Partial<ProviderTestResult> = {}): ProviderTestResult => ({
    model: "llama3.1",
    reply: " OK ",
    usage: { inputTokens: 21, outputTokens: 2, cacheReadTokens: null, cacheWriteTokens: null },
    ...overrides,
  });

  it("shows the reply and the tokens", () => {
    expect(describeTest(result())).toBe('llama3.1 answered "OK" (21 tokens in, 2 out)');
  });

  it("copes with an empty reply or model", () => {
    expect(describeTest(result({ reply: "", model: "" }))).toBe(
      "The model answered nothing (21 tokens in, 2 out)",
    );
  });
});

describe("GitHub Models", () => {
  it("names its models with the openai prefix", () => {
    expect(qualifiedModel("openai", "openai/gpt-4o")).toBe("openai:openai/gpt-4o");
    expect(GITHUB_MODELS_URL).toBe("https://models.github.ai/inference");
  });
});
