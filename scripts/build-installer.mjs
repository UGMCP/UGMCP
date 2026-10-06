#!/usr/bin/env node
/**
 * Build the Unit Agent installer for this operating system and copy it to dist/.
 *
 *   node scripts/build-installer.mjs mac
 *   node scripts/build-installer.mjs win
 *   node scripts/build-installer.mjs all
 *
 * macOS and Windows installers are produced on those systems. This script
 * refuses to pretend a cross-compile succeeded.
 */

import { spawnSync } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, cpSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { platform } from "node:os";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const dist = join(root, "dist");
const mode = process.argv[2] ?? "all";
const host = platform();

function fail(message) {
  console.error(`\n${message}\n`);
  process.exit(1);
}

function assertVersions() {
  const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8")).version;
  const tauri = JSON.parse(readFileSync(join(root, "src-tauri", "tauri.conf.json"), "utf8")).version;
  const cargo = readFileSync(join(root, "src-tauri", "Cargo.toml"), "utf8").match(/^version = "([^"]+)"/m)?.[1];
  if (!pkg || pkg !== tauri || pkg !== cargo) {
    fail(
      `Version mismatch. package.json is ${pkg}, tauri.conf.json is ${tauri}, Cargo.toml is ${cargo}.`,
    );
  }
}

function hostLabel() {
  if (host === "darwin") return "macOS";
  if (host === "win32") return "Windows";
  if (host === "linux") return "Linux";
  return host;
}

const macHelp = `Unit Agent macOS installers are built on a Mac.

Install Xcode, Rust, and Node.js, then run:
  npm run build:mac

A universal build (Apple silicon and Intel) needs both Rust targets:
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  UNIT_AGENT_CARGO_TARGET=universal-apple-darwin npm run build:mac

That writes dist/Unit-Agent.dmg and dist/Unit-Agent-mac.zip.
People who install the app only open the .dmg and drag Unit Agent to Applications.`;

const winHelp = `Unit Agent Windows installers are built on Windows.

Install the Microsoft C++ build tools, Rust, and Node.js, then run:
  npm run build:win

The x64 installer is:
  rustup target add x86_64-pc-windows-msvc
  UNIT_AGENT_CARGO_TARGET=x86_64-pc-windows-msvc npm run build:win

That writes dist/Unit-Agent-Setup.exe.
People who install the app only open that file and follow the wizard.`;

if (mode === "mac" && host !== "darwin") fail(macHelp);
if (mode === "win" && host !== "win32") fail(winHelp);
if (!["mac", "win", "all"].includes(mode)) {
  fail("Usage: node scripts/build-installer.mjs <mac|win|all>");
}

assertVersions();

let bundles;
if (mode === "mac" || (mode === "all" && host === "darwin")) bundles = "app,dmg";
else if (mode === "win" || (mode === "all" && host === "win32")) bundles = "nsis";
else if (mode === "all" && host === "linux") bundles = "deb,rpm,appimage";
else fail(`npm run build:all does not know how to package Unit Agent on ${hostLabel()}.`);

const cargoTarget = process.env.UNIT_AGENT_CARGO_TARGET?.trim();
const tauriBin = join(root, "node_modules", ".bin", host === "win32" ? "tauri.cmd" : "tauri");
if (!existsSync(tauriBin)) {
  fail("The Tauri CLI is not installed. From the repository root, run npm install.");
}

const args = ["build", "--bundles", bundles];
if (cargoTarget) args.push("--target", cargoTarget);

console.log(`Building Unit Agent ${bundles} packages on ${hostLabel()}.`);
const build = spawnSync(tauriBin, args, { cwd: root, stdio: "inherit", env: process.env });
if (build.error) fail(build.error.message);
if (build.status !== 0) process.exit(build.status ?? 1);

function collect(dir, out = []) {
  if (!existsSync(dir)) return out;
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name.endsWith(".app")) out.push(path);
      else collect(path, out);
    } else out.push(path);
  }
  return out;
}

function bundleFiles() {
  const dirs = [];
  if (cargoTarget) dirs.push(join(root, "src-tauri", "target", cargoTarget, "release", "bundle"));
  dirs.push(join(root, "src-tauri", "target", "release", "bundle"));
  for (const dir of dirs) {
    const files = collect(dir);
    if (files.length) return files;
  }
  return [];
}

function newest(files, test) {
  return files
    .filter(test)
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
}

const files = bundleFiles();
rmSync(dist, { recursive: true, force: true });
mkdirSync(dist, { recursive: true });
const written = [];

if (host === "darwin" && (mode === "mac" || mode === "all")) {
  const dmg = newest(files, (path) => path.endsWith(".dmg"));
  const app = newest(files, (path) => path.endsWith(".app"));
  if (!dmg || !app) fail("The macOS build finished without both a .dmg and a .app bundle.");
  const dmgOut = join(dist, "Unit-Agent.dmg");
  const zipOut = join(dist, "Unit-Agent-mac.zip");
  cpSync(dmg, dmgOut);
  const zip = spawnSync("ditto", ["-c", "-k", "--keepParent", app, zipOut], { stdio: "inherit" });
  if (zip.status !== 0) fail("Could not zip the Unit Agent.app bundle.");
  written.push(dmgOut, zipOut);
} else if (host === "win32" && (mode === "win" || mode === "all")) {
  const exe = newest(files, (path) => path.toLowerCase().endsWith("-setup.exe"));
  if (!exe) fail("The Windows build finished without an NSIS setup executable.");
  const out = join(dist, "Unit-Agent-Setup.exe");
  cpSync(exe, out);
  written.push(out);
} else {
  const packages = [
    [".deb", "Unit-Agent.deb"],
    [".rpm", "Unit-Agent.rpm"],
    [".AppImage", "Unit-Agent.AppImage"],
  ];
  for (const [extension, name] of packages) {
    const file = newest(files, (path) => path.endsWith(extension));
    if (!file) continue;
    const out = join(dist, name);
    cpSync(file, out);
    written.push(out);
  }
  if (!written.length) fail("The Linux build finished without a deb, rpm, or AppImage.");
}

console.log(`\nInstallers in dist/:\n${written.map((path) => `  ${path}`).join("\n")}`);
if (host === "linux") {
  console.log(`\nmacOS and Windows installers are built on those systems:\n  npm run build:mac\n  npm run build:win`);
}
