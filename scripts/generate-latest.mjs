#!/usr/bin/env node

import { mkdir, readdir, readFile, stat, writeFile } from "node:fs/promises";
import { basename, dirname, join, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = parseArgs(process.argv.slice(2));

if (args.help) {
  printHelp();
  process.exit(0);
}

const configPath = resolve(repoRoot, args.config || "src-tauri/tauri.conf.json");
const config = JSON.parse(await readFile(configPath, "utf8"));
const version = args.version || config.version;
const productName = config.productName || "Gallery";
const targetDir = resolve(repoRoot, args.targetDir || "src-tauri/target");
const outputPath = resolve(repoRoot, args.out || join("src-tauri/target/release/bundle", "latest.json"));
const baseUrl = trimTrailingSlash(
  args.baseUrl || `https://w200lab.com/website/gallery/releases/${version}`,
);
const notes = args.notes || `${productName} ${version}`;
const pubDate = args.pubDate || new Date().toISOString();
const windowsInstaller = args.windowsInstaller || "nsis";

if (!version) {
  fail("Missing app version. Set src-tauri/tauri.conf.json version or pass --version.");
}

if (windowsInstaller !== "nsis" && windowsInstaller !== "msi") {
  fail("--windows-installer must be nsis or msi.");
}

const artifacts = args.artifact
  ? [await artifactFromArgs(args)]
  : await discoverArtifacts(args.bundleDir, targetDir);

if (artifacts.length === 0) {
  fail(`No updater artifacts found under ${relative(repoRoot, targetDir)}.`);
}

if (args.platform && artifacts.length > 1) {
  fail("--platform can only be used when one artifact is selected with --artifact.");
}

const selectedArtifacts = selectArtifacts(artifacts, windowsInstaller);
if (args.url && selectedArtifacts.length > 1) {
  fail("--url can only be used when a single platform is generated.");
}

const platforms = {};
for (const item of selectedArtifacts) {
  const signature = (await readFile(item.artifact.signaturePath, "utf8")).trim();
  if (!signature) {
    fail(`Signature file is empty: ${relative(repoRoot, item.artifact.signaturePath)}`);
  }
  platforms[item.platform] = {
    signature,
    url: args.url || `${baseUrl}/${encodeURIComponent(basename(item.artifact.path))}`,
  };
}

const latest = {
  version,
  notes,
  pub_date: pubDate,
  platforms,
};

await mkdir(dirname(outputPath), { recursive: true });
await writeFile(outputPath, `${JSON.stringify(latest, null, 2)}\n`);

console.log(`Generated ${relative(repoRoot, outputPath)}`);
for (const [platform, info] of Object.entries(platforms)) {
  console.log(`${platform}: ${info.url}`);
}

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

async function artifactFromArgs(options) {
  const artifactPath = resolve(repoRoot, options.artifact);
  const signaturePath = resolve(repoRoot, options.signature || `${options.artifact}.sig`);
  await requireFile(artifactPath);
  await requireFile(signaturePath);
  return {
    path: artifactPath,
    signaturePath,
    bundleDir: dirname(dirname(artifactPath)),
    rustTarget: rustTargetFromPath(artifactPath),
    kind: inferArtifactKind(artifactPath, dirname(dirname(artifactPath))) || "custom",
  };
}

async function discoverArtifacts(bundleDirArg, targetRoot) {
  const bundleDirs = bundleDirArg
    ? [{
        path: resolve(repoRoot, bundleDirArg),
        rustTarget: rustTargetFromPath(resolve(repoRoot, bundleDirArg)),
      }]
    : await discoverBundleDirs(targetRoot);

  const artifacts = [];
  for (const bundleDir of bundleDirs) {
    const signaturePaths = await findFiles(bundleDir.path, (path) => path.endsWith(".sig"));
    for (const signaturePath of signaturePaths) {
      const artifactPath = signaturePath.slice(0, -".sig".length);
      if (!(await isFile(artifactPath))) continue;

      const kind = inferArtifactKind(artifactPath, bundleDir.path);
      if (!kind) continue;

      artifacts.push({
        path: artifactPath,
        signaturePath,
        bundleDir: bundleDir.path,
        rustTarget: bundleDir.rustTarget,
        kind,
      });
    }
  }
  return artifacts.sort((left, right) => left.path.localeCompare(right.path));
}

async function discoverBundleDirs(targetRoot) {
  const dirs = [];
  const nativeBundleDir = join(targetRoot, "release", "bundle");
  if (await isDirectory(nativeBundleDir)) {
    dirs.push({ path: nativeBundleDir, rustTarget: null });
  }

  if (await isDirectory(targetRoot)) {
    const entries = await readdir(targetRoot, { withFileTypes: true });
    for (const entry of entries) {
      if (!entry.isDirectory() || entry.name === "release" || entry.name === "debug") continue;
      const bundleDir = join(targetRoot, entry.name, "release", "bundle");
      if (await isDirectory(bundleDir)) {
        dirs.push({ path: bundleDir, rustTarget: entry.name });
      }
    }
  }

  return dirs.sort((left, right) => left.path.localeCompare(right.path));
}

async function findFiles(root, predicate) {
  const results = [];
  if (!(await isDirectory(root))) return results;

  const entries = await readdir(root, { withFileTypes: true });
  for (const entry of entries) {
    const path = join(root, entry.name);
    if (entry.isDirectory()) {
      results.push(...(await findFiles(path, predicate)));
    } else if (entry.isFile() && predicate(path)) {
      results.push(path);
    }
  }
  return results;
}

function selectArtifacts(artifacts, windowsInstaller) {
  const selected = new Map();

  for (const artifact of artifacts) {
    const platforms = args.platform ? [args.platform] : inferPlatforms(artifact);
    for (const platform of platforms) {
      const next = { platform, artifact };
      const current = selected.get(platform);
      if (!current || compareArtifactChoice(next, current, windowsInstaller) < 0) {
        selected.set(platform, next);
      }
    }
  }

  return Array.from(selected.values()).sort((left, right) => left.platform.localeCompare(right.platform));
}

function compareArtifactChoice(left, right, windowsInstaller) {
  const kindDiff = artifactKindPriority(left.artifact.kind, windowsInstaller)
    - artifactKindPriority(right.artifact.kind, windowsInstaller);
  if (kindDiff !== 0) return kindDiff;

  const targetDiff = targetSpecificityPriority(left.artifact.rustTarget)
    - targetSpecificityPriority(right.artifact.rustTarget);
  if (targetDiff !== 0) return targetDiff;

  return left.artifact.path.localeCompare(right.artifact.path);
}

function artifactKindPriority(kind, windowsInstaller) {
  if (kind === windowsInstaller) return 0;
  if (kind === "macos" || kind === "appimage") return 0;
  if (kind === "nsis" || kind === "msi") return 1;
  return 2;
}

function targetSpecificityPriority(rustTarget) {
  return rustTarget ? 0 : 1;
}

function inferArtifactKind(artifactPath, bundleDir) {
  const rel = relative(bundleDir, artifactPath).split(sep);
  const folder = rel[0]?.toLowerCase();
  const name = basename(artifactPath).toLowerCase();

  if (folder === "macos" && name.endsWith(".app.tar.gz")) return "macos";
  if (folder === "appimage" && (name.endsWith(".appimage") || name.endsWith(".appimage.tar.gz"))) {
    return "appimage";
  }
  if (folder === "nsis" && (name.endsWith(".exe") || name.endsWith(".nsis.zip"))) return "nsis";
  if (folder === "msi" && (name.endsWith(".msi") || name.endsWith(".msi.zip"))) return "msi";
  return null;
}

function inferPlatforms(artifact) {
  const os = osFromKind(artifact.kind) || osFromRustTarget(artifact.rustTarget) || osFromProcess();
  const universalPlatforms = platformsFromUniversalTarget(artifact.rustTarget, os);
  if (universalPlatforms.length > 0) return universalPlatforms;

  const arch = archFromRustTarget(artifact.rustTarget)
    || archFromName(basename(artifact.path))
    || archName(process.arch);

  return [`${os}-${arch}`];
}

function osFromKind(kind) {
  if (kind === "macos") return "darwin";
  if (kind === "nsis" || kind === "msi") return "windows";
  if (kind === "appimage") return "linux";
  return null;
}

function osFromRustTarget(target) {
  if (!target) return null;
  if (target.includes("windows")) return "windows";
  if (target.includes("darwin")) return "darwin";
  if (target.includes("linux")) return "linux";
  return null;
}

function osFromProcess() {
  if (process.platform === "darwin") return "darwin";
  if (process.platform === "win32") return "windows";
  return "linux";
}

function platformsFromUniversalTarget(target, os) {
  if (target !== "universal-apple-darwin" || os !== "darwin") return [];
  return ["darwin-aarch64", "darwin-x86_64"];
}

function archFromRustTarget(target) {
  if (!target) return null;
  if (target.startsWith("aarch64")) return "aarch64";
  if (target.startsWith("x86_64")) return "x86_64";
  if (target.startsWith("i686")) return "i686";
  if (target.startsWith("armv7")) return "armv7";
  return null;
}

function archFromName(name) {
  const value = name.toLowerCase();
  if (value.includes("aarch64") || value.includes("arm64")) return "aarch64";
  if (value.includes("i686")) return "i686";
  if (value.includes("armv7")) return "armv7";
  if (value.includes("x86_64") || value.includes("x64")) return "x86_64";
  return null;
}

function archName(arch) {
  if (arch === "arm64") return "aarch64";
  if (arch === "x64") return "x86_64";
  if (arch === "ia32") return "i686";
  if (arch === "arm") return "armv7";
  return arch;
}

function rustTargetFromPath(path) {
  const parts = path.split(sep);
  const targetIndex = parts.lastIndexOf("target");
  if (targetIndex === -1) return null;
  const maybeTarget = parts[targetIndex + 1];
  if (!maybeTarget || maybeTarget === "release" || maybeTarget === "debug") return null;
  return maybeTarget;
}

async function requireFile(path) {
  if (!(await isFile(path))) fail(`File not found: ${relative(repoRoot, path)}`);
}

async function isFile(path) {
  try {
    return (await stat(path)).isFile();
  } catch {
    return false;
  }
}

async function isDirectory(path) {
  try {
    return (await stat(path)).isDirectory();
  } catch {
    return false;
  }
}

function trimTrailingSlash(value) {
  return value.replace(/\/+$/u, "");
}

function printHelp() {
  console.log(`Generate Tauri updater latest.json.

Usage:
  npm run updater:latest
  npm run updater:latest -- --base-url https://w200lab.com/website/gallery/releases/0.2.0

Options:
  --base-url <url>              Base URL where updater artifacts are hosted.
  --bundle-dir <path>           Scan one bundle directory instead of src-tauri/target.
  --target-dir <path>           Scan a Tauri target directory. Defaults to src-tauri/target.
  --windows-installer <kind>    Prefer nsis or msi when both exist. Defaults to nsis.
  --out <path>                  Output latest.json path.
  --artifact <path>             Use one artifact explicitly.
  --signature <path>            Signature for --artifact. Defaults to <artifact>.sig.
  --platform <target>           Platform key for --artifact, for example windows-x86_64.
  --version <semver>            Override app version.
  --notes <text>                Release notes.
  --pub-date <iso-date>         RFC 3339 publish date.
`);
}

function fail(message) {
  console.error(message);
  process.exit(1);
}
