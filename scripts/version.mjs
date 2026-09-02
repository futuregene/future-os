#!/usr/bin/env node
// Single source of truth for FutureOS build versioning.
//
// Coordinated and standalone cases:
//   - coordinated test     → dev,     version = "0.0.2-<run_number>+test"
//   - coordinated nightly  → dev,     version = "0.0.2-<run_number>+nightly"
//   - standalone dev build → dev,     version = "0.0.2-<hash>+dev"
//   - on a `vX.Y.Z` tag    → release, version = "X.Y.Z"   (X must be ≥ 1)
//   - other online build   → dev,     version = "0.0.2-<hash>"
//   - local build          → dev,     version = "0.0.2-<hash>+local"
//                                       (…+local.dirty when the tree is dirty)
//   - iOS TestFlight       → dev,     version = "0.0.2"
//                                       (plain number: TestFlight rejects suffixes;
//                                       injected via FUTURE_VERSION by the workflow)
//
// The dev minor is pinned at `0.0.2` (DEV_VERSION below), NOT derived from git.
// CI uses a shallow checkout (fetch-depth: 1), so `git rev-list --count HEAD`
// always returns 1 there and a commit-count version would be frozen at 0.0.1.
// Standalone builds retain the short hash; coordinated test/nightly builds are
// traceable through their workflow run number. Local builds add `+local` (and
// `.dirty` when the tree is dirty) so a tester's laptop build is never mistaken
// for the matching online build. Store build numbers (Android versionCode / iOS
// CFBundleVersion) are injected separately by the workflows via github.run_number.
// Coordinated test/nightly display versions reuse that counter so desktop
// updater versions are monotonic within each channel.
//
// Bump DEV_VERSION (0.0.2 → 0.0.3) whenever the test-build version should
// advance — e.g. TestFlight already accepted a larger build number under the
// current marketing version, so a new marketing version must reset the sequence.
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

/** Dev-build SemVer core used by hash and coordinated channel versions. */
const DEV_VERSION = "0.0.2";

/** Resolve the internal and public artifact versions together. */
export function resolveVersions() {
  // Explicit overrides (set by CI job env / Makefile) win. Callers that need a
  // distinct public version must provide both instead of relying on parsing.
  if (process.env.FUTURE_VERSION) {
    const version = process.env.FUTURE_VERSION.trim();
    const artifactVersion =
      process.env.FUTURE_ARTIFACT_VERSION?.trim() || version;
    return { version, artifactVersion };
  }
  // Release tag: refs/tags/vX.Y.Z → X.Y.Z. Keep this ahead of every dev
  // channel so a repository-level channel variable can never alter a release.
  const ref = process.env.GITHUB_REF || "";
  const tag = ref.match(/^refs\/tags\/v(\d+\.\d+\.\d+)$/);
  if (tag) {
    const version = tag[1];
    return { version, artifactVersion: version };
  }
  // Build Test supplies both values so test and nightly updater versions are
  // monotonically ordered within their respective channels.
  const buildChannel = process.env.FUTURE_BUILD_CHANNEL?.trim();
  const runNumber = process.env.FUTURE_RUN_NUMBER?.trim();
  if (buildChannel || runNumber) {
    if (!new Set(["test", "nightly", "dev"]).has(buildChannel)) {
      throw new Error(
        `FUTURE_BUILD_CHANNEL must be test, nightly, or dev, got ${buildChannel || "empty"}`,
      );
    }
    if (buildChannel === "dev") {
      if (runNumber) {
        throw new Error("FUTURE_RUN_NUMBER is not used by the dev channel");
      }
      const artifactVersion = `${DEV_VERSION}-${gitShortHash()}`;
      return { version: `${artifactVersion}+dev`, artifactVersion };
    }
    if (!/^[1-9]\d*$/.test(runNumber || "")) {
      throw new Error(
        `FUTURE_RUN_NUMBER must be a positive integer, got ${runNumber || "empty"}`,
      );
    }
    const artifactVersion = `${DEV_VERSION}-${runNumber}`;
    const version = `${artifactVersion}+${buildChannel}`;
    return { version, artifactVersion };
  }
  // Dev build: pinned 0.0.2 + short hash. The hash locates the exact code and
  // keeps distinct branches from colliding on the same display version. A
  // commit-count scheme is useless in CI (shallow checkout → always 1), so the
  // minor is fixed and advanced manually via DEV_VERSION.
  const hash = gitShortHash();
  // Online (CI) builds are reproducible from the pushed commit, so the hash
  // stands alone. Local builds add `+local` (and `.dirty` for an uncommitted
  // tree) so a tester's laptop build is never confused with the online one.
  const artifactVersion = `${DEV_VERSION}-${hash}`;
  if (process.env.GITHUB_ACTIONS || process.env.CI) {
    return { version: artifactVersion, artifactVersion };
  }
  const version = `${artifactVersion}+local${gitDirty() ? ".dirty" : ""}`;
  return { version, artifactVersion };
}

/** Resolve only the internal display version for existing callers. */
export function resolveVersion() {
  return resolveVersions().version;
}

/** Short git hash, or "unknown" outside a git checkout (tarball build). */
function gitShortHash() {
  try {
    return (
      execSync("git rev-parse --short HEAD", {
        stdio: ["ignore", "pipe", "ignore"],
      })
        .toString()
        .trim() || "unknown"
    );
  } catch {
    return "unknown";
  }
}

/** Whether the working tree has uncommitted changes (untracked included). */
function gitDirty() {
  try {
    return (
      execSync("git status --porcelain", {
        stdio: ["ignore", "pipe", "ignore"],
      })
        .toString()
        .trim().length > 0
    );
  } catch {
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
  const { version, artifactVersion } = resolveVersions();

  switch (arg) {
    case "--json":
      process.stdout.write(
        JSON.stringify({
          version,
          isRelease: isRelease(version),
          bundleVersion: bundleVersion(version),
          artifactVersion,
        }),
      );
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
        `artifact_version=${artifactVersion}`,
      ].join("\n");
      writeFileSync(out, `${lines}\n`, { flag: "a" });
      break;
    }

    case "--gen-ts": {
      const out = process.argv[3];
      if (!out) {
        throw new Error("usage: version.mjs --gen-ts <path>");
      }
      writeFileSync(
        out,
        [
          "// Generated by scripts/version.mjs at build time — do not edit or commit.",
          `export const VERSION = ${JSON.stringify(version)};`,
          `export const IS_RELEASE = ${isRelease(version)};`,
          "",
        ].join("\n"),
      );
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
      patchJson(`${root}desktop/src-tauri/tauri.conf.json`, bundle);
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
