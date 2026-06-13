#!/usr/bin/env node

import { spawn } from "node:child_process";
import { mkdir } from "node:fs/promises";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";

const args = parseArgs(process.argv.slice(2));

if (args.help) {
  printHelp();
  process.exit(0);
}

const keyPath = expandHome(args.out || args.writeKey || join(homedir(), ".tauri", "gallery2.key"));
await mkdir(dirname(keyPath), { recursive: true });

const exitCode = await runTauri(["signer", "generate", "-w", keyPath]);
process.exit(exitCode);

function parseArgs(argv) {
  const parsed = {};
  const booleanFlags = new Set(["help"]);

  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (!token.startsWith("--")) {
      fail(`Unexpected argument: ${token}`);
    }

    const [rawKey, inlineValue] = token.slice(2).split("=", 2);
    const key = rawKey.replace(/-([a-z])/g, (_, letter) => letter.toUpperCase());

    if (booleanFlags.has(key)) {
      parsed[key] = true;
      continue;
    }

    const value = inlineValue ?? argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      fail(`Missing value for --${rawKey}`);
    }

    parsed[key] = value;
    if (inlineValue === undefined) index += 1;
  }

  return parsed;
}

function expandHome(path) {
  if (path === "~") return homedir();
  if (path.startsWith("~/") || path.startsWith("~\\")) {
    return resolve(homedir(), path.slice(2));
  }
  return resolve(path);
}

function runTauri(tauriArgs) {
  return new Promise((resolveExitCode, reject) => {
    const child = spawn("tauri", tauriArgs, {
      shell: process.platform === "win32",
      stdio: "inherit",
    });

    child.on("error", reject);
    child.on("close", (code) => resolveExitCode(code ?? 1));
  });
}

function printHelp() {
  console.log(`Generate Tauri updater signing keys.

Usage:
  npm run signer:generate
  npm run signer:generate -- --out ~/.tauri/gallery2.key

Options:
  --out <path>        Key output path. Defaults to <home>/.tauri/gallery2.key.
  --write-key <path>  Alias of --out.
`);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
