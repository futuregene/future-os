#!/usr/bin/env node
// Isolate GitHub-hosted defaults for rust-cache, but keep self-hosted installs
// outside RUNNER_TEMP so both toolchains and partial downloads survive jobs.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { appendFileSync, chmodSync, copyFileSync, mkdirSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

if (!process.env.RUNNER_TEMP || !process.env.GITHUB_ENV) {
  throw new Error("This setup script requires a GitHub Actions job");
}
const root = fileURLToPath(new URL("../../", import.meta.url));
const selfHosted = process.env.RUNNER_ENVIRONMENT === "self-hosted";
const rustupHome = selfHosted
  ? process.env.RUSTUP_HOME || join(homedir(), ".rustup")
  : join(process.env.RUNNER_TEMP, "futureos-rustup");
const cargoHome = process.env.CARGO_HOME || join(homedir(), ".cargo");
const env = { ...process.env, RUSTUP_HOME: rustupHome, CARGO_HOME: cargoHome };
// Let rustup select the version, components and targets from rust-toolchain.toml.
delete env.RUSTUP_TOOLCHAIN;

function rustup(args, capture = false) {
  const result = spawnSync("rustup", args, {
    cwd: root,
    env,
    encoding: "utf8",
    stdio: capture ? ["inherit", "pipe", "inherit"] : "inherit",
  });
  if (result.error)
    throw result.error;
  if (result.status !== 0)
    process.exit(result.status ?? 1);
  return result.stdout?.trim();
}

console.log(`CI Rust setup: ${selfHosted ? "self-hosted (persistent)" : "GitHub-hosted (job-local)"}`);
console.log(`RUSTUP_HOME: ${rustupHome}`);
// With no toolchain argument, rustup reads the project file and only installs
// missing toolchains/components/targets. A complete install needs no network.
rustup(["toolchain", "install", "--profile", "minimal", "--no-self-update"]);
const toolchain = rustup(["show", "active-toolchain"], true).split(/\s+/)[0];
// GITHUB_ENV selects this job's toolchain without changing the runner user's
// default. Only the disposable GitHub-hosted home needs a default for rust-cache.
if (!selfHosted)
  rustup(["default", toolchain]);
console.log(`CI Rust toolchain: ${toolchain}`);
const settings = {
  RUSTUP_HOME: rustupHome,
  RUSTUP_TOOLCHAIN: toolchain,
  CARGO_HOME: cargoHome,
  CARGO_INCREMENTAL: process.env.CARGO_INCREMENTAL ?? "0",
  CARGO_TERM_COLOR: process.env.CARGO_TERM_COLOR ?? "always",
};
appendFileSync(process.env.GITHUB_ENV, Object.entries(settings).map(([key, value]) => `${key}=${value}\n`).join(""));

// The Rust acceptance lane runs its test step through `cargo nextest`
// (`rust-checks.mjs`), and a workflow step could install it in one line -- but
// editing `.github/workflows/` needs the `workflow` OAuth scope, which the
// maintainer's token does not carry. This pinned-toolchain step is the step every
// Rust job already runs, so the install lives here instead. Only `rust-ci.yml`
// sets CI_PACKAGES: release, packaging and fixture jobs call this script too and
// would otherwise download a tool they never use.
//
// Pinned by version *and* by the digest of the release's own `.tar.gz`, so a
// retagged or tampered asset fails here rather than running in CI. The Apple
// asset is a universal binary, which is why both Apple triples map to it.
const NEXTEST_VERSION = "0.9.152";
const NEXTEST_ASSETS = {
  "x86_64-unknown-linux-gnu": ["x86_64-unknown-linux-gnu", "d039b43c3e0958441b6ad0a39c8884d018b3d2e8fb9928f209074b1100b5e17a"],
  "aarch64-unknown-linux-gnu": ["aarch64-unknown-linux-gnu", "ca1f81a5b86533eb45ab97e0064aa88e925e53aab9a698ca6331a7cad1f85f07"],
  "x86_64-pc-windows-msvc": ["x86_64-pc-windows-msvc", "0384fc89a5a547d4fd5e183fcc101d31dba2837feafd8e64d5ea003a0ddb1941"],
  "aarch64-pc-windows-msvc": ["aarch64-pc-windows-msvc", "dfdfc60a235866dbd5330102194e8f9f4fe06a2c34f6810b79c3c7e768c718e3"],
  "x86_64-apple-darwin": ["universal-apple-darwin", "2fadf57df4243e691f088ee3bff4f4ab9af16da0c95a1db84b5cb1685e098a76"],
  "aarch64-apple-darwin": ["universal-apple-darwin", "2fadf57df4243e691f088ee3bff4f4ab9af16da0c95a1db84b5cb1685e098a76"],
};

function findFile(directory, name) {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    const path = join(directory, entry.name);
    if (entry.isFile() && entry.name === name)
      return path;
    if (entry.isDirectory()) {
      const found = findFile(path, name);
      if (found)
        return found;
    }
  }
  return undefined;
}

async function installNextest() {
  const executable = process.platform === "win32" ? "cargo-nextest.exe" : "cargo-nextest";
  const target = join(cargoHome, "bin", executable);
  const report = () => spawnSync(target, ["--version"], { cwd: root, env, encoding: "utf8" });
  const current = report();
  if (current.status === 0 && current.stdout?.includes(`cargo-nextest ${NEXTEST_VERSION}`)) {
    console.log(`cargo-nextest: ${NEXTEST_VERSION} already installed`);
    return;
  }
  // The digest table is keyed by the host rustc, so an unknown platform fails
  // here with the triple named instead of silently running without nextest.
  const rustc = spawnSync("rustc", ["-vV"], { cwd: root, env, encoding: "utf8" });
  if (rustc.status !== 0)
    throw new Error(`rustc -vV failed: ${rustc.stderr ?? ""}`);
  const host = rustc.stdout.match(/^host: (.+)$/m)?.[1];
  const asset = NEXTEST_ASSETS[host];
  if (!asset)
    throw new Error(`No pinned cargo-nextest ${NEXTEST_VERSION} build for ${host}`);
  const [triple, digest] = asset;
  const name = `cargo-nextest-${NEXTEST_VERSION}-${triple}`;
  const directory = join(process.env.RUNNER_TEMP, "futureos-nextest");
  const archive = join(directory, `${name}.tar.gz`);
  mkdirSync(directory, { recursive: true });
  console.log(`cargo-nextest: downloading ${NEXTEST_VERSION} for ${host}`);
  const response = await fetch(`https://github.com/nextest-rs/nextest/releases/download/cargo-nextest-${NEXTEST_VERSION}/${name}.tar.gz`,
    { redirect: "follow", signal: AbortSignal.timeout(180_000) });
  if (!response.ok)
    throw new Error(`cargo-nextest download failed: HTTP ${response.status}`);
  const bytes = Buffer.from(await response.arrayBuffer());
  const actual = createHash("sha256").update(bytes).digest("hex");
  if (actual !== digest)
    throw new Error(`cargo-nextest checksum mismatch: expected ${digest}, got ${actual}`);
  writeFileSync(archive, bytes);
  // `tar` ships with every GitHub runner (bsdtar on Windows), so one archive
  // needs no unpacking dependency. Extract fresh: the directory lives under
  // RUNNER_TEMP, which a self-hosted runner keeps across jobs.
  const extract = join(directory, "extract");
  rmSync(extract, { recursive: true, force: true });
  mkdirSync(extract, { recursive: true });
  const tar = spawnSync("tar", ["-xzf", archive, "-C", extract], { stdio: "inherit" });
  if (tar.status !== 0)
    throw new Error(`cargo-nextest extraction failed: ${tar.status}`);
  const unpacked = findFile(extract, executable);
  if (!unpacked)
    throw new Error(`${name} did not contain ${executable}`);
  mkdirSync(join(cargoHome, "bin"), { recursive: true });
  copyFileSync(unpacked, target);
  if (process.platform !== "win32")
    chmodSync(target, 0o755);
  const installed = report();
  if (installed.status !== 0 || !installed.stdout?.includes(`cargo-nextest ${NEXTEST_VERSION}`))
    throw new Error(`cargo-nextest is not runnable after installing ${target}: ${installed.stdout ?? ""}${installed.stderr ?? ""}`);
  console.log(`cargo-nextest: ${NEXTEST_VERSION} for ${host}`);
}

if (process.env.CI_PACKAGES)
  await installNextest();
