import { ResultBlock, parseContent } from "./result.model";

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/** Blocks for a `resources/read` result (`contents: [{uri, mimeType, text | blob}]`). */
export function resourceBlocks(result: unknown): ResultBlock[] {
  if (!isObject(result) || !Array.isArray(result["contents"])) return [];
  return result["contents"].map((entry) => parseContent({ type: "resource", resource: entry }));
}

export interface PromptMessageView {
  role: string;
  block: ResultBlock;
}

/** Messages of a `prompts/get` result, each with its role and normalized content. */
export function promptMessages(result: unknown): {
  description: string | null;
  messages: PromptMessageView[];
} {
  if (!isObject(result)) return { description: null, messages: [] };
  const messages = Array.isArray(result["messages"]) ? result["messages"] : [];
  return {
    description: typeof result["description"] === "string" ? result["description"] : null,
    messages: messages.map((message) => {
      const entry = isObject(message) ? message : {};
      return {
        role: typeof entry["role"] === "string" ? entry["role"] : "unknown",
        block: parseContent(entry["content"]),
      };
    }),
  };
}

/** Names of required prompt arguments that are still empty. */
export function missingArguments(
  definitions: { name: string; required?: boolean }[],
  values: Record<string, string>,
): string[] {
  return definitions
    .filter((d) => d.required && (values[d.name] ?? "").trim() === "")
    .map((d) => d.name);
}
