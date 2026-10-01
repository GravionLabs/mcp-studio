import type { Cost, MessageRecord, Price } from "../../core/bindings";

/** An editable row of the price table; numbers are text while the user types. */
export interface PriceDraft {
  model: string;
  input: string;
  output: string;
  cacheRead: string;
  cacheWrite: string;
  currency: string;
  /** Not saved yet, so the model name can still be edited. */
  isNew: boolean;
}

export const emptyDraft = (): PriceDraft => ({
  model: "",
  input: "",
  output: "",
  cacheRead: "",
  cacheWrite: "",
  currency: "USD",
  isNew: true,
});

export function toDraft(price: Price): PriceDraft {
  const text = (value: number | null) => (value === null ? "" : String(value));
  return {
    model: price.model,
    input: text(price.inputPerMtok),
    output: text(price.outputPerMtok),
    cacheRead: text(price.cacheReadPerMtok),
    cacheWrite: text(price.cacheWritePerMtok),
    currency: price.currency,
    isNew: false,
  };
}

function amount(label: string, text: string, required: boolean): number | null | string {
  const trimmed = text.trim();
  if (trimmed === "") return required ? `${label} is required` : null;
  const value = Number(trimmed);
  if (!Number.isFinite(value) || value < 0) return `${label} must be a number of zero or more`;
  return value;
}

/** Converts a draft to a price, or returns the problem as text. */
export function draftToPrice(draft: PriceDraft): Price | string {
  if (draft.model.trim() === "") return "Model is required";
  const input = amount("Input price", draft.input, true);
  const output = amount("Output price", draft.output, true);
  const cacheRead = amount("Cache read price", draft.cacheRead, false);
  const cacheWrite = amount("Cache write price", draft.cacheWrite, false);
  for (const value of [input, output, cacheRead, cacheWrite]) {
    if (typeof value === "string") return value;
  }
  return {
    model: draft.model.trim(),
    inputPerMtok: input as number,
    outputPerMtok: output as number,
    cacheReadPerMtok: cacheRead as number | null,
    cacheWritePerMtok: cacheWrite as number | null,
    currency: draft.currency.trim().toUpperCase() || "USD",
  };
}

/** Formats money; costs are estimates, so they are prefixed with `~`. */
export function formatCost(cost: Cost): string {
  // JSON cannot carry NaN or infinity, so bindings type every f64 as `number | null`.
  const value = cost.amount ?? 0;
  const digits = value === 0 ? 2 : value < 0.01 ? 6 : value < 1 ? 4 : 2;
  try {
    const text = new Intl.NumberFormat(undefined, {
      style: "currency",
      currency: cost.currency,
      minimumFractionDigits: 2,
      maximumFractionDigits: digits,
    }).format(value);
    return `~${text}`;
  } catch {
    return `~${value.toFixed(digits)} ${cost.currency}`;
  }
}

/**
 * Cost of one message at a price. Tool call arguments are written by the model (output price);
 * everything else, such as results and tool definitions, is read by it (input price). The same
 * rule is used for session totals in `mcp-studio-core` (`metering.rs`).
 */
export function messageCost(
  message: Pick<MessageRecord, "method" | "tokens">,
  price: Price | undefined,
): Cost | null {
  if (!price || message.tokens === null) return null;
  const rate = (message.method === "tools/call" ? price.outputPerMtok : price.inputPerMtok) ?? 0;
  return { amount: (message.tokens * rate) / 1_000_000, currency: price.currency };
}

/** Cost of a whole tool call: its request (arguments) and its response (result). */
export function callCost(
  messages: readonly Pick<MessageRecord, "sessionId" | "jsonrpcId" | "method" | "tokens">[],
  call: Pick<MessageRecord, "sessionId" | "jsonrpcId">,
  price: Price | undefined,
): Cost | null {
  if (!price || call.jsonrpcId === null) return null;
  const parts = messages.filter(
    (m) => m.sessionId === call.sessionId && m.jsonrpcId === call.jsonrpcId,
  );
  const costs = parts.map((m) => messageCost(m, price)).filter((c): c is Cost => c !== null);
  if (costs.length === 0) return null;
  return {
    amount: costs.reduce((sum, c) => sum + (c.amount ?? 0), 0),
    currency: price.currency,
  };
}
