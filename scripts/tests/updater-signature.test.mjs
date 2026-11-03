import assert from "node:assert/strict";
import { Buffer } from "node:buffer";
// CI runs this before installing npm dependencies.
// eslint-disable-next-line test/no-import-node-test
import test from "node:test";
import { checkManifestSignatures, checkSignatureVersion } from "../release/updater-signature.mjs";

// Only the version metadata is inspected here; cryptographic authentication is
// still performed by Tauri on the client, not by the manifest publisher.
/** @param {string} fields */
function signature(fields) {
  return Buffer.from(`untrusted comment: signature\npacket\ntrusted comment: timestamp:1\tfile:FutureOS.app.tar.gz${fields}\nglobal-signature\n`).toString("base64");
}

test("rejects the numeric installer version for a full nightly release", () => {
  assert.throws(() => checkSignatureVersion(signature("\tversion:0.0.2"), "0.0.2-107+nightly"), /does not match/);
});

test("accepts full nightly, test, and plain release versions", () => {
  for (const version of ["0.0.2-107+nightly", "0.0.2-107+test", "1.2.3"]) {
    checkSignatureVersion(signature(`\tversion:${version}`), version);
  }
});

test("requires one unambiguous version in the trusted comment", () => {
  for (const fields of ["", "\tfile:version:1.2.3", "\tversion:1.2.3\tversion:1.2.3"]) {
    assert.throws(() => checkSignatureVersion(signature(fields), "1.2.3"), /does not match/);
  }
  assert.throws(() => checkSignatureVersion("", "1.2.3"), /Missing/);
  assert.throws(() => checkSignatureVersion("invalid", "1.2.3"), /does not match/);
  assert.throws(() => checkSignatureVersion(signature("\tversion:1.2.3"), ""), /release version/);
});

test("blocks publication when any platform signature disagrees with the manifest", () => {
  const manifest = {
    version: "0.0.2-107+nightly",
    platforms: {
      "darwin-aarch64": {
        signature: signature("\tversion:0.0.2-107+nightly"),
      },
      "windows-x86_64": {
        signature: signature("\tversion:0.0.2"),
      },
    },
  };
  assert.throws(() => checkManifestSignatures(manifest), /windows-x86_64.*does not match/);
  manifest.platforms["windows-x86_64"].signature = signature("\tversion:0.0.2-107+nightly");
  checkManifestSignatures(manifest);
  assert.throws(() => checkManifestSignatures({ version: "1.2.3" }), /no platform signatures/);
});
