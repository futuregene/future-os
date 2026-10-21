#!/usr/bin/env node
// Isolate GitHub-hosted defaults for rust-cache, but keep self-hosted installs
// outside RUNNER_TEMP so both toolchains and partial downloads survive jobs.
import { spawnSync } from "node:child_process";
import { appendFileSync } from "node:fs";
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
