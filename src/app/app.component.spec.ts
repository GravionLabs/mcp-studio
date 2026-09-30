import { describe, expect, it } from "vitest";
import { routes } from "./app.routes";

describe("app routes", () => {
  it("exports a route table", () => {
    expect(Array.isArray(routes)).toBe(true);
  });
});
