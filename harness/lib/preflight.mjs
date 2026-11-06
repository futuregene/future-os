/**
 * Preflight: are the binaries this run will use actually built from this tree?
 *
 * A full pass once reported ten TUI failures that all read "text not found:
 * 描述你要做的事". The real cause was one line of stderr — the TUI refusing to
 * attach to a demo agent stamped `0.0.0` while the client was `0.0.2-…` — and each
 * of the ten cases failed with its own unrelated assertion message.
 *
 * So: compare the version compiled into `target/debug/future` with what this tree
 * would compile now, and say so before the cases run. A warning, not a gate: the
 * run continues, but the first lines of the transcript name the real problem.
 */
import { execFileSync } from "node:child_process";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";

const REPO_ROOT = resolve(import.meta.dirname, "..", "..");
const BIN = process.env.FUTURE_BIN || join(REPO_ROOT, "target", "debug", "future");
const AGENT = process.env.FUTURE_DEMO_AGENT ||
  join(REPO_ROOT, "target", "debug", "examples", "demo_agent");

function treeVersion() {
  try {
    return execFileSync("node", [join(REPO_ROOT, "scripts", "version.mjs")], {
      encoding: "utf8", timeout: 10_000,
    }).trim();
  } catch { return null; }
}

/** `scripts/version.mjs` appends `.dirty` for an edited tree; the stamp baked into a
 *  binary never has it, so compare without it — otherwise every uncommitted edit
 *  would raise a false alarm. */
function normalise(version) {
  return version ? version.replace(/\.dirty$/, "") : version;
}

function binaryVersion(bin) {
  try {
    const out = execFileSync(bin, ["version"], { encoding: "utf8", timeout: 10_000 });
    const m = out.match(/v?(\d+\.\d+\.\d+[^\s]*)/);
    return m ? m[1] : null;
  } catch { return null; }
}

/** Human-readable problems; empty means the run is consistent. */
export function preflight({ needsTui = false, needsCli = false } = {}) {
  const problems = [];
  if (!needsTui && !needsCli) return problems;

  if (!existsSync(BIN)) {
    problems.push(`找不到 ${BIN} —— 先构建：FUTURE_VERSION=$(node scripts/version.mjs) cargo build -p future-cli`);
    return problems;
  }
  const expected = treeVersion();
  const actual = binaryVersion(BIN);
  if (expected && actual && normalise(actual) !== normalise(expected)) {
    problems.push(
      `二进制与工作树版本不一致：${BIN} = ${actual}，工作树 = ${expected}。` +
      "这会让 TUI/CLI 面出现一批看起来互不相关的失败。用同一个 FUTURE_VERSION 重建后再跑：" +
      `\n      FUTURE_VERSION=${expected} cargo build -p future-cli -p future-tui --example demo_agent`);
  }
  if (needsTui && !existsSync(AGENT)) {
    problems.push(`找不到 demo agent：${AGENT} —— 构建它：cargo build -p future-tui --example demo_agent`);
  }
  return problems;
}

export function reportPreflight(problems) {
  if (!problems.length) return;
  process.stdout.write("\n\x1b[33m⚠ preflight\x1b[0m\n");
  for (const p of problems) process.stdout.write(`  · ${p}\n`);
  process.stdout.write(
    "  这是提示，不是失败：用例照常跑。但上面的不一致通常解释了一批看起来无关的失败。\n\n");
}
