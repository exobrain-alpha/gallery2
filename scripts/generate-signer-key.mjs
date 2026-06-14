#!/usr/bin/env node

import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdir } from "node:fs/promises";
import { dirname, isAbsolute, resolve } from "node:path";

const ENV_FILE_NAME = ".env";
const ENV_SIGNING_KEY_PATH = "GALLERY_SIGNING_KEY_PATH";

const args = parseArgs(process.argv.slice(2));
const envFile = loadEnvFile(resolve(process.cwd(), ENV_FILE_NAME));

if (args.help) {
  printHelp();
  process.exit(0);
}

const configuredKeyPath = getConfiguredSigningKeyPath(envFile);
const explicitKeyPath = args.out || args.writeKey;
const keyPathValue = explicitKeyPath || configuredKeyPath;
if (!keyPathValue) {
  fail(`Missing signing key path. Set ${ENV_SIGNING_KEY_PATH} in ${ENV_FILE_NAME}, or pass --out <path>.`);
}
const keyPath = explicitKeyPath ? resolve(explicitKeyPath) : keyPathValue;
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

function loadEnvFile(path) {
  if (!existsSync(path)) return {};
  const entries = {};
  const lines = readFileSync(path, "utf8").split(/\r?\n/);
  lines.forEach((rawLine, index) => {
    let line = rawLine.trim();
    if (!line || line.startsWith("#")) return;
    if (line.startsWith("export ")) line = line.slice("export ".length).trimStart();
    const separator = line.indexOf("=");
    if (separator === -1) {
      fail(`${ENV_FILE_NAME}:${index + 1} is missing "=".`);
    }
    const key = line.slice(0, separator).trim();
    if (!/^[A-Z0-9_]+$/.test(key)) {
      fail(`${ENV_FILE_NAME}:${index + 1} has invalid key: ${key}`);
    }
    entries[key] = parseEnvValue(line.slice(separator + 1));
  });
  return entries;
}

function getConfiguredSigningKeyPath(envFile) {
  const value = process.env[ENV_SIGNING_KEY_PATH] || envFile[ENV_SIGNING_KEY_PATH];
  if (!value) return undefined;
  if (!isAbsolute(value)) {
    fail(`${ENV_SIGNING_KEY_PATH} must be an absolute path. Do not use relative paths or ~.`);
  }
  return value;
}

function parseEnvValue(value) {
  const trimmed = value.trim();
  if (trimmed.length >= 2) {
    const quote = trimmed[0];
    if ((quote === "\"" || quote === "'") && trimmed.endsWith(quote)) {
      return trimmed.slice(1, -1);
    }
  }
  return trimmed;
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
  npm run signer:generate -- --out /path/to/gallery2.key

Options:
  --out <path>        Key output path. Defaults to ${ENV_FILE_NAME} -> ${ENV_SIGNING_KEY_PATH}.
  --write-key <path>  Alias of --out.

Env:
  ${ENV_SIGNING_KEY_PATH}=/path/to/gallery2.key

Notes:
  ${ENV_SIGNING_KEY_PATH} only supports absolute paths. Do not use relative paths or ~.
`);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
