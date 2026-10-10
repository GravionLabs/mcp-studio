import type { ClientRequest } from "../../core/bindings";
import { JsonSchema, validate } from "../playground/schema-form.model";

export interface SamplingMessageView {
  role: string;
  text: string;
}

export interface SamplingView {
  system: string | null;
  messages: SamplingMessageView[];
  /** Model hints and priorities as one line, or `null` when the server gave none. */
  preferences: string | null;
  maxTokens: number | null;
  temperature: number | null;
}

export interface ElicitationView {
  message: string;
  schema: JsonSchema;
}

function record(value: unknown): Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : {};
}

/** What one content block of a sampling message shows; media is named, not rendered. */
function blockText(block: unknown): string {
  const b = record(block);
  if (b["type"] === "text" && typeof b["text"] === "string") return b["text"];
  return `[${typeof b["type"] === "string" ? b["type"] : "unknown"} content]`;
}

function preferencesLine(value: unknown): string | null {
  const p = record(value);
  const parts: string[] = [];
  const hints = Array.isArray(p["hints"])
    ? p["hints"].map((h) => record(h)["name"]).filter((n): n is string => typeof n === "string")
    : [];
  if (hints.length > 0) parts.push(`hints: ${hints.join(", ")}`);
  for (const [key, label] of [
    ["costPriority", "cost"],
    ["speedPriority", "speed"],
    ["intelligencePriority", "intelligence"],
  ] as const) {
    if (typeof p[key] === "number") parts.push(`${label} ${p[key]}`);
  }
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** The params of a `sampling/createMessage` request, ready to show. */
export function samplingView(params: unknown): SamplingView {
  const p = record(params);
  const messages = Array.isArray(p["messages"]) ? p["messages"] : [];
  return {
    system: typeof p["systemPrompt"] === "string" ? p["systemPrompt"] : null,
    messages: messages.map((m) => {
      const message = record(m);
      const content = Array.isArray(message["content"]) ? message["content"] : [message["content"]];
      return {
        role: typeof message["role"] === "string" ? message["role"] : "user",
        text: content.map(blockText).join("\n"),
      };
    }),
    preferences: preferencesLine(p["modelPreferences"]),
    maxTokens: typeof p["maxTokens"] === "number" ? p["maxTokens"] : null,
    temperature: typeof p["temperature"] === "number" ? p["temperature"] : null,
  };
}

/** The params of an `elicitation/create` request: the message and the schema of the form. */
export function elicitationView(params: unknown): ElicitationView {
  const p = record(params);
  const schema = record(p["requestedSchema"]) as JsonSchema;
  return {
    message: typeof p["message"] === "string" ? p["message"] : "",
    schema: { type: "object", ...schema },
  };
}

/** Problems that keep the form from being submitted; empty when it may be. */
export function elicitationProblems(schema: JsonSchema, value: unknown): string[] {
  return validate(schema, value ?? {}, schema);
}

/** What keeps a sampling answer from being sent, or `null`. */
export function samplingBlocker(text: string): string | null {
  return text.trim() === "" ? "Write the answer first." : null;
}

/** Adds a request unless it is already queued. */
export function enqueue(queue: readonly ClientRequest[], request: ClientRequest): ClientRequest[] {
  return queue.some((q) => q.id === request.id) ? [...queue] : [...queue, request];
}

/** Removes a request that is no longer waiting. */
export function dequeue(queue: readonly ClientRequest[], id: string): ClientRequest[] {
  return queue.filter((q) => q.id !== id);
}
