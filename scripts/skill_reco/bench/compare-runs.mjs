// Per-question diff between two recorded pipeline runs, so "the effect did not change" can be
// checked question by question instead of asserted from the aggregate metrics.
//
//   node compare-runs.mjs <dirA> <dirB>
import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import { cacheFor } from "./common.mjs";

const load = (dir) =>
  new Map(
    readdirSync(dir)
      .filter((file) => file.endsWith(".json"))
      .map((file) => [file.replace(/\.json$/, ""), JSON.parse(readFileSync(path.join(dir, file), "utf8"))]),
  );

const [a = "/tmp/jev-before", b = "../runs/jev"] = process.argv.slice(2);
const A = load(a);
const B = load(b);
const gold = new Map(cacheFor("gold-v1").all().map((row) => [row.id, row]));

const answerOf = (row) => row.answer?.[0] ?? null;
const fitOf = (row) => (row.stage2 && row.stage2.length ? Math.max(...row.stage2.map((c) => c.fit ?? 0)) : null);

const ids = [...A.keys()].filter((id) => B.has(id)).sort();
const diffs = [];
let same = 0;
for (const id of ids) {
  const before = answerOf(A.get(id));
  const after = answerOf(B.get(id));
  if (before === after) {
    same += 1;
    continue;
  }
  diffs.push({ id, before, after, gold: gold.get(id)?.gold[0] ?? "拒答", a: A.get(id), b: B.get(id) });
}

console.log(`${a}  →  ${b}   （${ids.length} 题）`);
console.log(`最终答案相同 ${same}/${ids.length}，不同 ${diffs.length}\n`);
for (const d of diffs) {
  console.log(
    `  ${d.id}: ${d.before ?? "拒答"} → ${d.after ?? "拒答"}   （参照 ${d.gold}）\n` +
      `      top-1 noul ${d.a.top1_noul} → ${d.b.top1_noul}；最高 fit ${fitOf(d.a)} → ${fitOf(d.b)}`,
  );
}

const tokensA = ids.map((id) => (A.get(id).stage1_tokens ?? 0) + (A.get(id).stage2_tokens ?? 0)).sort((x, y) => x - y);
const tokensB = ids.map((id) => (B.get(id).stage1_tokens ?? 0) + (B.get(id).stage2_tokens ?? 0)).sort((x, y) => x - y);
const median = (list) => list[Math.floor(list.length / 2)];
console.log(`\n每题 token：${median(tokensA)} → ${median(tokensB)}（${((median(tokensB) / median(tokensA)) * 100).toFixed(0)}%）`);
