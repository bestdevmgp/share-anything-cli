#!/usr/bin/env node

const { execSync } = require("child_process");
const fs = require("fs");
const path = require("path");
const https = require("https");

const VERSION = require("./package.json").version;
const REPO = "bestdevmgp/share-anything-cli";

function getPlatformBinary() {
  const platform = process.platform;
  const arch = process.arch;

  const map = {
    "darwin-x64": "share-macos-x86_64",
    "darwin-arm64": "share-macos-aarch64",
    "linux-x64": "share-linux-x86_64",
    "linux-arm64": "share-linux-aarch64",
    "win32-x64": "share-windows-x86_64.exe",
  };

  const key = `${platform}-${arch}`;
  const binary = map[key];

  if (!binary) {
    console.error(`Unsupported platform: ${platform}-${arch}`);
    process.exit(1);
  }

  return binary;
}

function getBinPath() {
  const isWindows = process.platform === "win32";
  return path.join(__dirname, isWindows ? "share.exe" : "share");
}

function follow(url) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { "User-Agent": "share-anything-npm" } }, (res) => {
      if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
        follow(res.headers.location).then(resolve).catch(reject);
      } else if (res.statusCode === 200) {
        resolve(res);
      } else {
        reject(new Error(`HTTP ${res.statusCode} for ${url}`));
      }
    }).on("error", reject);
  });
}

async function download(url, dest) {
  const res = await follow(url);
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    res.pipe(file);
    file.on("finish", () => {
      file.close(resolve);
    });
    file.on("error", (err) => {
      fs.unlink(dest, () => {});
      reject(err);
    });
  });
}

async function main() {
  const binaryName = getPlatformBinary();
  const url = `https://github.com/${REPO}/releases/download/v${VERSION}/${binaryName}`;
  const binPath = getBinPath();

  console.log(`Downloading share v${VERSION} for ${process.platform}-${process.arch}...`);

  try {
    await download(url, binPath);
    if (process.platform !== "win32") {
      fs.chmodSync(binPath, 0o755);
    }
    console.log("share installed successfully!");
  } catch (err) {
    console.error(`Failed to download binary: ${err.message}`);
    console.error(`URL: ${url}`);
    process.exit(1);
  }
}

main();
