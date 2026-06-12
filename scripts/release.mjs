#!/usr/bin/env node

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const BUMP_ALIASES = {
  major: "major",
  minor: "minor",
  patch: "patch",
};

function usage() {
  console.error(`Usage: npm run release:<major|minor|patch> [-- --no-push]

Examples:
  npm run release:patch
  npm run release:minor
  npm run release:major
  npm run release:patch -- --no-push
`);
}

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: ROOT,
    encoding: "utf8",
    stdio: options.capture ? "pipe" : "inherit",
  });

  if (result.status !== 0) {
    const stderr = result.stderr?.trim();
    throw new Error(stderr || `${command} ${args.join(" ")} failed`);
  }

  return result.stdout ?? "";
}

function readJson(path) {
  return JSON.parse(readFileSync(path, "utf8"));
}

function writeJson(path, value) {
  writeFileSync(path, `${JSON.stringify(value, null, 2)}\n`, "utf8");
}

function parseArgs(argv) {
  const args = argv.slice(2);
  const bumpArg = args.find((arg) => !arg.startsWith("-"));
  const noPush = args.includes("--no-push");

  if (!bumpArg || !(bumpArg in BUMP_ALIASES)) {
    usage();
    process.exit(1);
  }

  return {
    bump: BUMP_ALIASES[bumpArg],
    noPush,
  };
}

function assertCleanWorkingTree() {
  const status = run("git", ["status", "--porcelain"], { capture: true }).trim();
  if (status) {
    throw new Error("Working tree is not clean. Commit or stash changes before releasing.");
  }
}

function bumpPackageVersion(bump) {
  run("npm", ["version", bump, "--no-git-tag-version"]);
}

function syncRustAndTauriVersion(version) {
  const cargoTomlPath = join(ROOT, "src-tauri/Cargo.toml");
  const cargoToml = readFileSync(cargoTomlPath, "utf8");
  const nextCargoToml = cargoToml.replace(
    /^version = "[^"]+"/m,
    `version = "${version}"`,
  );
  if (nextCargoToml === cargoToml) {
    throw new Error("Failed to update src-tauri/Cargo.toml version");
  }
  writeFileSync(cargoTomlPath, nextCargoToml, "utf8");

  const tauriConfPath = join(ROOT, "src-tauri/tauri.conf.json");
  const tauriConf = readJson(tauriConfPath);
  tauriConf.version = version;
  writeJson(tauriConfPath, tauriConf);

  const cargoLockPath = join(ROOT, "src-tauri/Cargo.lock");
  const cargoLock = readFileSync(cargoLockPath, "utf8");
  const nextCargoLock = cargoLock.replace(
    /(name = "zwitch"\nversion = ")[^"]+(")/,
    `$1${version}$2`,
  );
  if (nextCargoLock === cargoLock) {
    throw new Error('Failed to update src-tauri/Cargo.lock "zwitch" version');
  }
  writeFileSync(cargoLockPath, nextCargoLock, "utf8");
}

function commitAndTag(version) {
  const tag = `v${version}`;
  const files = [
    "package.json",
    "package-lock.json",
    "src-tauri/Cargo.toml",
    "src-tauri/Cargo.lock",
    "src-tauri/tauri.conf.json",
  ];

  run("git", ["add", ...files]);
  run("git", ["commit", "-m", `chore: bump version to ${tag}`]);
  run("git", ["tag", tag]);
  return tag;
}

function pushRelease(tag) {
  const branch = run("git", ["rev-parse", "--abbrev-ref", "HEAD"], {
    capture: true,
  }).trim();
  run("git", ["push", "origin", branch]);
  run("git", ["push", "origin", tag]);
}

function main() {
  const { bump, noPush } = parseArgs(process.argv);
  const currentVersion = readJson(join(ROOT, "package.json")).version;

  assertCleanWorkingTree();

  console.log(`Releasing ${currentVersion} -> ${bump} bump`);
  bumpPackageVersion(bump);

  const nextVersion = readJson(join(ROOT, "package.json")).version;
  syncRustAndTauriVersion(nextVersion);

  const tag = commitAndTag(nextVersion);
  console.log(`Created commit and tag ${tag}`);

  if (noPush) {
    console.log("Skipped push (--no-push).");
    return;
  }

  pushRelease(tag);
  console.log(`Pushed branch and tag ${tag}`);
}

try {
  main();
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
}
