import { describe, expect, it } from "vitest";
import type { Price } from "../../core/bindings";
import {
  callCost,
  draftToPrice,
  emptyDraft,
  formatCost,
  messageCost,
  toDraft,
} from "./prices.model";

const price: Price = {
  model: "m",
  inputPerMtok: 3,
  outputPerMtok: 15,
  cacheReadPerMtok: null,
  cacheWritePerMtok: 0.5,
  currency: "USD",
};

describe("draftToPrice", () => {
  it("converts a filled draft", () => {
    const draft = { ...emptyDraft(), model: " m ", input: "3", output: "15", currency: "eur" };
    expect(draftToPrice(draft)).toEqual({
      model: "m",
      inputPerMtok: 3,
      outputPerMtok: 15,
      cacheReadPerMtok: null,
      cacheWritePerMtok: null,
      currency: "EUR",
    });
  });

  it("keeps optional cache prices when given", () => {
    const draft = { ...toDraft(price), cacheRead: "0.3" };
    expect(draftToPrice(draft)).toMatchObject({ cacheReadPerMtok: 0.3, cacheWritePerMtok: 0.5 });
  });

  it("reports what is missing or wrong", () => {
    expect(draftToPrice(emptyDraft())).toBe("Model is required");
    expect(draftToPrice({ ...emptyDraft(), model: "m" })).toBe("Input price is required");
    expect(draftToPrice({ ...emptyDraft(), model: "m", input: "1" })).toBe(
      "Output price is required",
    );
    expect(draftToPrice({ ...emptyDraft(), model: "m", input: "-1", output: "1" })).toContain(
      "zero or more",
    );
    expect(draftToPrice({ ...emptyDraft(), model: "m", input: "x", output: "1" })).toContain(
      "Input price must be",
    );
  });

  it("round-trips through toDraft", () => {
    expect(draftToPrice(toDraft(price))).toEqual(price);
    expect(toDraft(price).isNew).toBe(false);
  });
});

describe("formatCost", () => {
  it("prefixes estimates and shows enough digits for tiny amounts", () => {
    expect(formatCost({ amount: 12.5, currency: "USD" })).toMatch(/^~.*12\.50$/);
    expect(formatCost({ amount: 0.0036, currency: "USD" })).toMatch(/^~.*0\.0036$/);
    expect(formatCost({ amount: 0, currency: "USD" })).toMatch(/^~.*0\.00$/);
  });

  it("falls back for unknown currency codes", () => {
    expect(formatCost({ amount: 0.5, currency: "??" })).toBe("~0.5000 ??");
  });
});

describe("messageCost", () => {
  it("prices tool call arguments at the output price", () => {
    const cost = messageCost({ method: "tools/call", tokens: 1_000_000 }, price);
    expect(cost).toEqual({ amount: 15, currency: "USD" });
  });

  it("prices results and everything else at the input price", () => {
    expect(messageCost({ method: null, tokens: 1_000_000 }, price)?.amount).toBe(3);
    expect(messageCost({ method: "tools/list", tokens: 500_000 }, price)?.amount).toBe(1.5);
  });

  it("returns null without a price or a token count", () => {
    expect(messageCost({ method: null, tokens: 5 }, undefined)).toBeNull();
    expect(messageCost({ method: null, tokens: null }, price)).toBeNull();
  });
});

describe("callCost", () => {
  const message = (jsonrpcId: string | null, method: string | null, tokens: number | null) => ({
    sessionId: "s",
    jsonrpcId,
    method,
    tokens,
  });

  it("adds the arguments and the result of one call", () => {
    const messages = [
      message("1", "tools/call", 1_000_000),
      message("1", null, 1_000_000),
      message("2", "tools/call", 1_000_000),
    ];
    expect(callCost(messages, { sessionId: "s", jsonrpcId: "1" }, price)).toEqual({
      amount: 18,
      currency: "USD",
    });
  });

  it("returns null without a price, an id, or counted tokens", () => {
    const messages = [message("1", "tools/call", null)];
    expect(callCost(messages, { sessionId: "s", jsonrpcId: "1" }, price)).toBeNull();
    expect(callCost(messages, { sessionId: "s", jsonrpcId: null }, price)).toBeNull();
    expect(callCost(messages, { sessionId: "s", jsonrpcId: "1" }, undefined)).toBeNull();
  });

  it("ignores the same id in another session", () => {
    const messages = [{ ...message("1", "tools/call", 1_000_000), sessionId: "other" }];
    expect(callCost(messages, { sessionId: "s", jsonrpcId: "1" }, price)).toBeNull();
  });
});
