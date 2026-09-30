import { describe, expect, it } from "vitest";
import { fileNameFor } from "./file-dialog.service";

describe("fileNameFor", () => {
  it("makes a safe lowercase file name", () => {
    expect(fileNameFor("Smoke tests / Echo")).toBe("smoke-tests-echo.json");
    expect(fileNameFor("  ../../etc/passwd ")).toBe(
      "..-..-etc-passwd.json".replace("..-..-", "..-..-"),
    );
  });

  it("falls back for empty titles", () => {
    expect(fileNameFor("///")).toBe("collection.json");
    expect(fileNameFor("", "txt")).toBe("collection.txt");
  });
});
