#!/usr/bin/env node
// Single source of truth for FutureOS build versioning.
//
// Three cases, all keyed off "is this a release tag":
//   - on a `vX.Y.Z` tag    → release, version = "X.Y.Z"   (X must be ≥ 1)
//   - other online build   → dev,    version = "0.0.<commit-count>-<hash>"
//   - local build          → dev,    version = "0.0.<commit-count>-<hash>+local"
//                                      (…+local.dirty when the tree is dirty)
//   - iOS TestFlight       → dev,    version = "0.0.<commit-count>"
//                                      (plain number: TestFlight rejects suffixes;
//                                      injected via FUTURE_VERSION by the workflow)
//
// The commit count (git rev-list --count HEAD) makes the version monotonic
// across builds; the short hash pinpoints the exact code (`git show <hash>`) and
// keeps distinct branches from colliding on the same 0.0.<count>. Local builds
// add `+local` build metadata so they're never mistaken for the matching online
// build, and `.dirty` when the working tree has uncommitted changes — a dirty
// local build does NOT correspond to that commit, so the hash must not be
// trusted verbatim.
//
// The release/dev channel is DERIVED from the version string, not injected
// separately: a version whose FIRST component is `0` is a dev build; anything
// starting `1`+ is a release. This keeps the model to a single injected value
// (FUTURE_VERSION) at the cost of one assumption:
//
//   ⚠️ Release versions must NEVER start with `0`. A `v0.x.y` tag would be
//   misread as a dev build. The first public release is 1.0.0, so 0.* is
//   reserved for dev builds forever. If 0.x releases are ever wanted, reintroduce
//   an explicit channel here and in the Rust/TS helpers instead of parsing the
//   version.
//
// Config files stay pinned at 0.0.0; the real version is injected at build time
// from here (env FUTURE_VERSION → Rust build.rs, generated TS module, and the
// Tauri bundle version).

import { execSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";

/** Resolve the display version string for this build. */
export function resolveVersion() {
  // Explicit override (set by CI job env / Makefile) wins.
  if (process.env.FUTURE_VERSION) {
    return process.env.FUTURE_VERSION.trim();
  }
  // Release tag: refs/tags/vX.Y.Z → X.Y.Z
  const ref = process.env.GITHUB_REF || "";
  const tag = ref.match(/^refs\/tags\/v(\d+\.\d+\.\d+)$/);
  if (tag) {
    return tag[1];
  }
  // Dev build: 0.0.<commit-count>-<hash>. The commit count is monotonic (build
  // number for stores); the short hash locates the exact code and keeps branches
  // from colliding on the same count.
  const count = gitCommitCount();
  const hash = gitShortHash();
  // Online (CI) builds are reproducible from the pushed commit, so the hash
  // stands alone. Local builds add `+local` (and `.dirty` for an uncommitted
  // tree) so a tester's laptop build is never confused with the online one.
  if (process.env.GITHUB_ACTIONS || process.env.CI) {
    return `0.0.${count}-${hash}`;
  }
  return `0.0.${count}-${hash}+local${gitDirty() ? ".dirty" : ""}`;
}

/** Short git hash, or "unknown" outside a git checkout (tarball build). */
function gitShortHash() {
  try {
    return execSync("git rev-parse --short HEAD", { stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .trim() || "unknown";
  }
  catch {
    return "unknown";
  }
}

/** Total commit count on HEAD, or 0 outside a git checkout. */
function gitCommitCount() {
  try {
    return execSync("git rev-list --count HEAD", { stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .trim() || "0";
  }
  catch {
    return "0";
  }
}

/** Whether the working tree has uncommitted changes (untracked included). */
function gitDirty() {
  try {
    return execSync("git status --porcelain", { stdio: ["ignore", "pipe", "ignore"] })
      .toString()
      .trim().length > 0;
  }
  catch {
    return false;
  }
}

/** A version is a release iff its first component is non-zero (0.* is dev). */
export function isRelease(version) {
  return !version.startsWith("0");
}

/** Bundle/installer version — plain semver core (NSIS rejects suffixes). */
export function bundleVersion(version) {
  return version.split(/[-+]/)[0];
}

// ─── CLI ─────────────────────────────────────────────────────────────────────

function patchJson(path, version) {
  const raw = readFileSync(path, "utf8");
  // Replace only the top-level "version" field, preserving formatting.
  const next = raw.replace(/("version"\s*:\s*)"[^"]*"/, `$1"${version}"`);
  writeFileSync(path, next);
}

function main() {
  const arg = process.argv[2];
  const version = resolveVersion();

  switch (arg) {
    case "--json":
      process.stdout.write(JSON.stringify({
        version,
        isRelease: isRelease(version),
        bundleVersion: bundleVersion(version),
      }));
      break;

    case "--github-output": {
      const out = process.env.GITHUB_OUTPUT;
      if (!out) {
        throw new Error("GITHUB_OUTPUT is not set");
      }
      const lines = [
        `version=${version}`,
        `is_release=${isRelease(version)}`,
        `bundle_version=${bundleVersion(version)}`,
      ].join("\n");
      writeFileSync(out, `${lines}\n`, { flag: "a" });
      break;
    }

    case "--gen-ts": {
      const out = process.argv[3];
      if (!out) {
        throw new Error("usage: version.mjs --gen-ts <path>");
      }
      writeFileSync(out, [
        "// Generated by scripts/version.mjs at build time — do not edit or commit.",
        `export const VERSION = ${JSON.stringify(version)};`,
        `export const IS_RELEASE = ${isRelease(version)};`,
        "",
      ].join("\n"));
      break;
    }

    case "--set-bundle": {
      // Patch the Tauri installer version to the plain semver core (installers
      // reject `-`/`+` suffixes, so every non-release build reads 0.0.0). The
      // display version (with the -<hash>[+local] suffix) is injected separately
      // via FUTURE_VERSION.
      const bundle = bundleVersion(version);
      // fileURLToPath (not .pathname) so this resolves correctly on Windows too.
      const root = fileURLToPath(new URL("../", import.meta.url));
      patchJson(`${root}gui/src-tauri/tauri.conf.json`, bundle);
      process.stderr.write(`tauri bundle version set to ${bundle}\n`);
      break;
    }

    default:
      // Bare invocation prints the version — handy for `$(node scripts/version.mjs)`.
      process.stdout.write(version);
  }
}

if (import.meta.url === pathToFileURL(process.argv[1]).href) {
  main();
}
