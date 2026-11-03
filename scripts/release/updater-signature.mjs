#!/usr/bin/env node
// Installer versions can omit nightly suffixes. Updater signatures must bind
// the full version announced by latest.json instead of the installer version.
import { Buffer } from "node:buffer";
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { resolve } from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

/**
 * @param {string} signature
 * @param {string} version
 */
export function checkSignatureVersion(signature, version) {
  if (typeof version !== "string" || !version || /[\t\r\n]/.test(version)) {
    throw new Error("A non-empty release version without tabs or newlines is required");
  }
  if (typeof signature !== "string" || !signature.trim()) {
    throw new Error("Missing updater signature");
  }
  // This is a release consistency check. The updater authenticates the trusted
  // comment together with the artifact using minisign before installation.
  const comment = Buffer.from(signature.trim(), "base64").toString("utf8").split(/\r?\n/).find(line => line.startsWith("trusted comment: "));
  const versions = comment?.slice("trusted comment: ".length).split("\t").filter(field => field.startsWith("version:")).map(field => field.slice("version:".length)) ?? [];
  if (versions.length !== 1 || versions[0] !== version) {
    throw new Error(`Updater signature version ${JSON.stringify(versions)} does not match release ${version}`);
  }
}

/** @param {{ version: string, platforms?: Record<string, { signature: string }> }} manifest */
export function checkManifestSignatures(manifest) {
  const platforms = Object.entries(manifest.platforms ?? {});
  if (!platforms.length)
    throw new Error("Update manifest has no platform signatures");
  for (const [platform, asset] of platforms) {
    try {
      checkSignatureVersion(asset.signature, manifest.version);
    }
    catch (error) {
      const message = error instanceof Error
        ? error.message
        : String(error);
      throw new Error(`${platform}: ${message}`, { cause: error });
    }
  }
}

function main() {
  const [command, path, version, ...extra] = process.argv.slice(2);
  if (command === "sign" && path && version && !extra.length) {
    const artifact = resolve(path);
    const require = createRequire(import.meta.url);
    const cli = require.resolve("@tauri-apps/cli/tauri.js");
    const result = spawnSync(process.execPath, [
      cli,
      "signer",
      "sign",
      "--app-version",
      version,
      artifact,
    ], { stdio: "inherit" });
    if (result.error)
      throw result.error;
    if (result.status !== 0)
      process.exit(result.status ?? 1);
    checkSignatureVersion(readFileSync(`${artifact}.sig`, "utf8"), version);
  }
  else if (command === "check-manifest" && path && !version) {
    checkManifestSignatures(JSON.parse(readFileSync(path, "utf8")));
  }
  else {
    throw new Error("Usage: updater-signature.mjs sign <artifact> <version> | check-manifest <latest.json>");
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href)
  main();
