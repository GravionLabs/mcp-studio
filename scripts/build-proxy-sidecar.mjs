#!/usr/bin/env node
// Builds mcp-studio-proxy in release mode and places it where Tauri's `externalBin` expects it:
//   src-tauri/binaries/mcp-studio-proxy-<target-triple>[.exe]
// Usage: node scripts/build-proxy-sidecar.mjs [target-triple]
// The triple defaults to $TAURI_ENV_TARGET_TRIPLE, then to the host triple reported by rustc.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");

function hostTriple() {
  const output = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = /^host: (.+)$/m.exec(output);
  if (!match) throw new Error("could not determine the host target triple from `rustc -vV`");
  return match[1];
}

const triple = process.argv[2] || process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
const extension = triple.includes("windows") ? ".exe" : "";

execFileSync("cargo", ["build", "--release", "-p", "mcp-studio-proxy", "--target", triple], {
  cwd: root,
  stdio: "inherit",
});

const source = join(root, "target", triple, "release", `mcp-studio-proxy${extension}`);
const targetDir = join(root, "src-tauri", "binaries");
mkdirSync(targetDir, { recursive: true });
const destination = join(targetDir, `mcp-studio-proxy-${triple}${extension}`);
copyFileSync(source, destination);
console.log(`sidecar ready: ${destination}`);
