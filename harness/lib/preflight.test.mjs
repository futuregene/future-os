import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, writeFileSync, chmodSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

// preflight reads its binaries from env, so each test can hand it a stand-in.
async function preflightWith(binPath, { needsTui = false, needsCli = true } = {}) {
  const saved = { ...process.env };
  process.env.FUTURE_BIN = binPath;
  process.env.FUTURE_DEMO_AGENT = binPath;
  const { preflight } = await import("./preflight.mjs?t=" + Math.random());
  const problems = preflight({ needsTui, needsCli });
  process.env = saved;
  return problems;
}

function fakeBinary(version) {
  const dir = mkdtempSync(join(tmpdir(), "preflight-"));
  const bin = join(dir, "future");
  writeFileSync(bin, `#!/bin/sh\necho "future v${version}"\n`);
  chmodSync(bin, 0o755);
  return bin;
}

test("a stale binary is reported with the rebuild command", async () => {
  const expected = execFileSync("node", ["scripts/version.mjs"], { encoding: "utf8" }).trim()
    .replace(/\.dirty$/, "");
  const problems = await preflightWith(fakeBinary("0.0.1-nope+local"));
  assert.equal(problems.length, 1);
  assert.match(problems[0], /不一致/);
  assert.ok(problems[0].includes(expected), "提示里给出工作树的版本，方便直接复制重建命令");
});

test("an unmodified tree does not warn, even though version.mjs marks it dirty", async () => {
  const expected = execFileSync("node", ["scripts/version.mjs"], { encoding: "utf8" }).trim()
    .replace(/\.dirty$/, "");
  const problems = await preflightWith(fakeBinary(expected));
  assert.deepEqual(problems, [], "`.dirty` must not be treated as a version difference");
});

test("closing over a missing binary names the build command", async () => {
  const problems = await preflightWith(join(tmpdir(), "definitely-not-here", "future"));
  assert.equal(problems.length, 1);
  assert.match(problems[0], /cargo build -p future-cli/);
});
