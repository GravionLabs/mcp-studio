import { describe, expect, it } from "vitest";
import { recordToRows, rowsToRecord, SecretWrite } from "./key-value-rows";

describe("rowsToRecord", () => {
  it("skips blank names and keeps plain values", () => {
    const writes: SecretWrite[] = [];
    expect(
      rowsToRecord(
        [
          { key: " a ", value: "1" },
          { key: "  ", value: "x" },
        ],
        () => "n",
        writes,
      ),
    ).toEqual({ a: "1" });
    expect(writes).toEqual([]);
  });

  it("turns secrets into references and reports writes", () => {
    const writes: SecretWrite[] = [];
    const record = rowsToRecord(
      [
        { key: "K", value: "v", secret: true },
        { key: "L", value: "", secret: true, stored: "keyring:old" },
      ],
      () => "new",
      writes,
    );
    expect(record).toEqual({ K: "keyring:new", L: "keyring:old" });
    expect(writes).toEqual([{ name: "new", value: "v" }]);
  });
});

describe("recordToRows", () => {
  it("masks references", () => {
    expect(recordToRows({ A: "keyring:x", B: "plain" })).toEqual([
      { key: "A", value: "", secret: true, stored: "keyring:x" },
      { key: "B", value: "plain" },
    ]);
  });
});
