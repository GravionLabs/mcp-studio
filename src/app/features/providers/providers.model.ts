import type { ProviderTestResult } from "../../core/bindings";

export type ProviderId = "anthropic" | "openai" | "ollama";

/**
 * The model name as a flow's LLM step writes it: Anthropic models have no prefix, the others name
 * their provider (`openai:gpt-4o`, `ollama:llama3.1:8b`). Empty when no model is given.
 */
export function qualifiedModel(provider: ProviderId, model: string): string {
  const name = model.trim();
  if (name === "") return "";
  return provider === "anthropic" ? name : `${provider}:${name}`;
}

/** One line that tells what a test request returned. */
export function describeTest(result: ProviderTestResult): string {
  const { inputTokens, outputTokens } = result.usage;
  const reply = result.reply.trim() === "" ? "nothing" : `"${result.reply.trim()}"`;
  return `${result.model || "The model"} answered ${reply} (${inputTokens} tokens in, ${outputTokens} out)`;
}

/** Address of GitHub Models, which speaks the OpenAI chat completions API. */
export const GITHUB_MODELS_URL = "https://models.github.ai/inference";
