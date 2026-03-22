#!/usr/bin/env node

const { execFileSync } = require("child_process");
const path = require("path");

const isWindows = process.platform === "win32";
const binPath = path.join(__dirname, isWindows ? "share.exe" : "share");

try {
  execFileSync(binPath, process.argv.slice(2), { stdio: "inherit" });
} catch (err) {
  if (err.status !== null) {
    process.exit(err.status);
  }
  console.error(`Failed to run share: ${err.message}`);
  process.exit(1);
}
