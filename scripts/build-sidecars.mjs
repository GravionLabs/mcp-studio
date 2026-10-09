#!/usr/bin/env node
// Builds the programs that ship next to the app in release mode and places them where Tauri's
// `externalBin` expects them:
//   src-tauri/binaries/<program>-<target-triple>[.exe]
// The programs are the stdio proxy for recording real clients (mcp-studio-proxy) and the reference
// server that the welcome page offers to try (mcp-studio-testserver).
// Usage: node scripts/build-sidecars.mjs [target-triple]
// The triple defaults to $TAURI_ENV_TARGET_TRIPLE, then to the host triple reported by rustc.
import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const programs = ["mcp-studio-proxy", "mcp-studio-testserver"];

function hostTriple() {
  const output = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  const match = /^host: (.+)$/m.exec(output);
  if (!match) throw new Error("could not determine the host target triple from `rustc -vV`");
  return match[1];
}

const triple = process.argv[2] || process.env.TAURI_ENV_TARGET_TRIPLE || hostTriple();
const extension = triple.includes("windows") ? ".exe" : "";

execFileSync(
  "cargo",
  ["build", "--release", ...programs.flatMap((p) => ["-p", p]), "--target", triple],
  { cwd: root, stdio: "inherit" },
);

const targetDir = join(root, "src-tauri", "binaries");
mkdirSync(targetDir, { recursive: true });
for (const program of programs) {
  const source = join(root, "target", triple, "release", `${program}${extension}`);
  const destination = join(targetDir, `${program}-${triple}${extension}`);
  copyFileSync(source, destination);
  console.log(`sidecar ready: ${destination}`);
}
