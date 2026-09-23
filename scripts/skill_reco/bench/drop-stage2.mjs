#!/usr/bin/env node
// What did removing the second call cost, and what does it buy? This is the script behind REPORT
// §2.2.5 and the historical evidence for the removal.
//
// The shipped pipeline answers from ONE call (../suggest.mjs): a Choice over the roster with a
// none_of_these option, gated on that option's probability. The second call — the same question with
// the full description and the SKILL.md opening for each of the top 3, one Noul per candidate — was
// removed. Its probe is still recorded in runs/jev by predict-jev.mjs, so both readings are compared
// on the SAME recorded stage-1 answers. That pairing matters: stage 1 alone moves by ~2 questions
// between two identical runs (bench/stage1-stability.mjs), so an unpaired comparison of two separate
// runs would be measuring run-to-run noise as much as the change.
//
// Reads the cache only — no API calls.
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { QUESTIONS_FILE, RUNS_DIR, readJson } from "./common.mjs";
import { loadRoster } from "../roster.mjs";
import { FITS_THRESHOLD } from "./second-call.mjs";
import { USD_PER_MTOK_INPUT, USD_TO_CNY } from "../jev.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const roster = loadRoster(process.env.SKILLS_ROOT ?? path.join(here, "..", "..", "..", "skills"));
const questions = readJson(QUESTIONS_FILE).questions;

const readAll = (dir) =>
  fs
    .readdirSync(path.join(RUNS_DIR, dir))
    .filter((f) => f.endsWith(".json"))
    .map((f) => JSON.parse(fs.readFileSync(path.join(RUNS_DIR, dir, f), "utf8")));
const pipeline = new Map(readAll("jev").map((row) => [row.id, row]));
const gold = new Map(readAll("gold-v1").map((row) => [row.id, row.gold]));
const textOf = new Map(questions.map((q) => [q.id, q.text]));

const ids = questions.map((q) => q.id);
const goldOf = (id) => gold.get(id) ?? [];
const goldFirst = (id) => goldOf(id)[0] ?? null;
const answerable = ids.filter((id) => goldOf(id).length > 0);
const refusable = ids.filter((id) => goldOf(id).length === 0);

const rec = (id) => pipeline.get(id);
const gatePasses = (id) => (rec(id)?.gate?.length ?? 0) > 0;
// Shipped: the Choice's own top-1 (the record's `answer` field, written by predict-jev.mjs).
const shippedAnswer = (id) => rec(id)?.answer ?? [];
// Removed: the highest fit Noul of the recorded second-call probe.
const rankedFits = (id) => [...(rec(id)?.stage2 ?? [])].sort((a, b) => (b.fit ?? 0) - (a.fit ?? 0));
const twoCallAnswer = (id) => {
  if (!gatePasses(id)) return [];
  const best = rankedFits(id)[0];
  return best && (best.fit ?? 0) >= FITS_THRESHOLD ? [best.name] : [];
};

const pct = (x, n) => `${((x / n) * 100).toFixed(1)}%`;
const quantile = (values, p) => {
  const s = [...values].filter((x) => typeof x === "number").sort((a, b) => a - b);
  return s[Math.min(s.length - 1, Math.floor(s.length * p))];
};
const median = (v) => quantile(v, 0.5);

// ------------------------------------------------------- 1. quality (paired)

const score = (answerOf) => {
  let agree = 0;
  let top1 = 0;
  let inRef = 0;
  let refusal = 0;
  let falseRefuse = 0;
  let falseSuggest = 0;
  for (const id of ids) {
    const g = goldOf(id);
    const pred = answerOf(id).filter(Boolean);
    if (!g.length) {
      if (!pred.length) {
        agree += 1;
        refusal += 1;
      } else falseSuggest += 1;
    } else if (pred[0] === g[0]) {
      agree += 1;
      top1 += 1;
      inRef += 1;
    } else {
      if (pred[0] && g.includes(pred[0])) inRef += 1;
      if (!pred.length) falseRefuse += 1;
    }
  }
  return { agree, top1, inRef, refusal, falseRefuse, falseSuggest };
};

// The middle option: answer from one call, but make the second call when the Choice was not
// confident about its winner. This is what the previous revision shipped (SKIP_VERIFY_ABOVE = 0.95).
const SKIP_AT = 0.95;
const unsure = (id) => (rec(id)?.stage1_top?.[0]?.p ?? 1) < SKIP_AT;
const middleAnswer = (id) => (!gatePasses(id) ? [] : unsure(id) ? twoCallAnswer(id) : shippedAnswer(id));

const one = score(shippedAnswer);
const two = score(twoCallAnswer);
const mid = score(middleAnswer);
console.log("=== 一次调用（发布） vs 两次调用（已移除），同一批记录的配对比较 ===");
console.log("  指标                一次调用        两次调用        差");
const row = (label, x, y, n) =>
  console.log(`  ${label.padEnd(18)} ${pct(x, n).padStart(10)} ${pct(y, n).padStart(14)} ${(y - x >= 0 ? "+" : "") + (y - x)}`);
row("决策一致", one.agree, two.agree, 100);
row("首答正确", one.top1, two.top1, 65);
row("首答∈参照集", one.inRef, two.inRef, 65);
row("正确拒答", one.refusal, two.refusal, 35);
row("误拒", one.falseRefuse, two.falseRefuse, 65);
console.log(`  （差为整数道题：${two.top1 - one.top1} 道首答、${two.inRef - one.inRef} 道按参照集合算）`);

// ------------------------------------------------------- 2. latency and cost (the reason)

const oneMs = ids.map((id) => rec(id)?.stage1_ms ?? 0);
const twoMs = ids.map((id) => (rec(id)?.stage1_ms ?? 0) + (gatePasses(id) ? rec(id)?.stage2_ms ?? 0 : 0));
const oneTok = ids.map((id) => rec(id)?.stage1_tokens ?? 0);
const twoTok = ids.map((id) => (rec(id)?.stage1_tokens ?? 0) + (gatePasses(id) ? rec(id)?.stage2_tokens ?? 0 : 0));

console.log("\n=== 延迟与成本 ===");
console.log("  口径              p50     p90     p95     p99     平均   token/题（均值）  费用/题");
for (const [label, ms, tok] of [
  ["一次调用（发布）", oneMs, oneTok],
  ["两次调用（已移除）", twoMs, twoTok],
]) {
  const meanMs = ms.reduce((a, b) => a + b, 0) / ms.length;
  const meanTok = tok.reduce((a, b) => a + b, 0) / tok.length;
  console.log(
    `  ${label.padEnd(18)}${String(quantile(ms, 0.5)).padStart(5)} ${String(quantile(ms, 0.9)).padStart(7)} ${String(quantile(ms, 0.95)).padStart(7)} ${String(quantile(ms, 0.99)).padStart(7)} ${String(Math.round(meanMs)).padStart(7)} ${String(Math.round(meanTok)).padStart(17)} ${("¥" + ((meanTok * USD_PER_MTOK_INPUT * USD_TO_CNY) / 1e6).toFixed(5)).padStart(9)}`,
  );
}
const dMs = (f) => quantile(twoMs, f) - quantile(oneMs, f);
console.log(
  `  移除第二次调用：p50 −${dMs(0.5)} ms（${pct(dMs(0.5), quantile(twoMs, 0.5))}）· p90 −${dMs(0.9)} ms · p95 −${dMs(0.95)} ms · p99 −${dMs(0.99)} ms`,
);
console.log(`  成本：${pct(twoTok.reduce((a, b) => a + b, 0) - oneTok.reduce((a, b) => a + b, 0), twoTok.reduce((a, b) => a + b, 0))} 减少`);

console.log("\n=== 三种做法的正面对比（同一批记录）===");
console.log(`  中间方案 = 一次调用，但 Choice 对第一名没把握时（top-1 < ${SKIP_AT}）再发第二次`);
console.log("  做法                          首答正确        首答∈参照集      第二次调用    延迟p50   p90    p95    费用/题");
const midMs = ids.map((id) => (rec(id)?.stage1_ms ?? 0) + (gatePasses(id) && unsure(id) ? rec(id)?.stage2_ms ?? 0 : 0));
const midTok = ids.map((id) => (rec(id)?.stage1_tokens ?? 0) + (gatePasses(id) && unsure(id) ? rec(id)?.stage2_tokens ?? 0 : 0));
const midCalls = ids.filter((id) => gatePasses(id) && unsure(id)).length;
const twoCalls = ids.filter((id) => gatePasses(id)).length;
for (const [label, sc, calls, ms, tok] of [
  ["① 只一次调用", one, 0, oneMs, oneTok],
  ["② 没把握时才第二次（上一版）", mid, midCalls, midMs, midTok],
  ["③ 总是两次调用", two, twoCalls, twoMs, twoTok],
]) {
  const meanTok = tok.reduce((a, b) => a + b, 0) / tok.length;
  console.log(
    `  ${label.padEnd(28)} ${String(sc.top1).padStart(2)}/65 = ${pct(sc.top1, 65).padStart(6)}   ${String(sc.inRef).padStart(2)}/65 = ${pct(sc.inRef, 65).padStart(6)}   ${String(calls).padStart(2)}/65       ${String(quantile(ms, 0.5)).padStart(5)} ${String(quantile(ms, 0.9)).padStart(5)} ${String(quantile(ms, 0.95)).padStart(6)}    ¥${((meanTok * USD_PER_MTOK_INPUT * USD_TO_CNY) / 1e6).toFixed(5)}`,
  );
}
const missedByOne = ids.filter((id) => middleAnswer(id)[0] !== shippedAnswer(id)[0]);
console.log(`\n  ① 相对 ② 少的 3 道题里，② 能救回 ${missedByOne.filter((id) => middleAnswer(id)[0] === goldFirst(id)).length} 道；`);
console.log(`  而 ② 只在 ${midCalls}/65 的题上付出第二次调用（比 ③ 少 ${twoCalls - midCalls} 次）。`);

const differ = ids.filter((id) => (shippedAnswer(id)[0] ?? null) !== (twoCallAnswer(id)[0] ?? null));
console.log(`\n  两者答案不同的题：${differ.length} 道`);
for (const id of differ) {
  const g = goldFirst(id) ?? "（拒答）";
  const s1 = shippedAnswer(id)[0] ?? "拒答";
  const s2 = twoCallAnswer(id)[0] ?? "拒答";
  const verdict = s2 === goldFirst(id) ? "→ 两次调用正确" : s1 === goldFirst(id) ? "→ 一次调用正确" : "→ 两者都错";
  console.log(
    `    ${id}  金标题 ${String(g).padEnd(22)} 一次 ${String(s1).padEnd(22)} 两次 ${String(s2).padEnd(22)} ${verdict}` +
      (g === "（拒答）" ? "" : `   参照集合 ${JSON.stringify(goldOf(id))}`),
  );
}


// ------------------------------------------------------- 3. refusal never needed it

const gateRefused = refusable.filter((id) => !gatePasses(id)).length;
const caughtByFit = ids.filter((id) => gatePasses(id) && twoCallAnswer(id).length === 0).length;
console.log("\n=== 拒答需不需要上一次调用 ===");
console.log(`  该拒答的 35 道：门控拒掉 ${gateRefused} 道，fit 阈值额外拒掉 ${caughtByFit} 道`);
console.log(`  → ${caughtByFit === 0 ? "fit 阈值一次都没触发，拒答完全由门控完成" : "fit 阈值有贡献"}`);

// ------------------------------------------------------- 4. is the removed call's win trustworthy?

console.log("\n=== 被移除那次调用赢的题：它是怎么赢的 ===");
const wins = differ.filter((id) => twoCallAnswer(id)[0] === goldFirst(id));
if (!wins.length) console.log("  （本次运行没有它改对的题）");
const words = (text) =>
  new Set(
    (text ?? "")
      .toLowerCase()
      .match(/[a-z][a-z-]{4,}/g)
      ?.filter((w) => !["would", "which", "should", "please", "using", "across", "where", "their", "there"].includes(w)) ?? [],
  );
for (const id of wins) {
  const g = goldFirst(id);
  const w = roster.byName.get(g);
  const q = textOf.get(id).toLowerCase();
  // Words present in the winner's SKILL.md opening but NOT in its description. If the question uses
  // them, then the question and the SKILL.md share vocabulary the description does not have — which
  // is exactly the advantage only the removed call had access to.
  const onlyInExcerpt = [...words(w?.excerpt)].filter((t) => !words(w?.description).has(t));
  const overlap = onlyInExcerpt.filter((t) => q.includes(t));
  console.log(`\n  ${id}: 一次调用选了 ${shippedAnswer(id)[0]}，两次调用改成 ${g}  参照集合 ${JSON.stringify(goldOf(id))}`);
  console.log(`    题面: ${textOf.get(id).slice(0, 120)}`);
  console.log(`    ${g} 的 description: ${String(w?.description).slice(0, 150)}`);
  console.log(`    ${g} 的 SKILL.md 开头: ${String(w?.excerpt).slice(0, 150)}`);
  console.log(
    `    只在 SKILL.md（不在 description）里的实词里，被题面用到的：${overlap.slice(0, 10).join(", ") || "无"}${overlap.length ? "  ← 这是只有被移除的那次调用能看到的信息" : ""}`,
  );
}

// ------------------------------------------------------- 5. could a richer single call replace it?

console.log("\n=== 能不能用更长的描述把那几道题补回来 ===");
const lens = roster.skills.map((s) => s.description.length);
const over = lens.filter((n) => n > 220).length;
const cutChars = roster.skills.reduce((sum, s) => sum + Math.max(0, s.description.length - 220), 0);
console.log(`  ${141} 个技能里 description 被截断到 220 字符的：${over} 个 = ${pct(over, 141)}（中位长度 ${median(lens)}）`);
console.log(`  全部放开会多约 ${Math.round(cutChars / 4).toLocaleString()} token，即当前一次调用的 ${Math.round((cutChars / 4 / median(oneTok)) * 100)}%`);
const wonByExcerptOnly = wins.filter((id) => {
  const w = roster.byName.get(goldFirst(id));
  const q = textOf.get(id).toLowerCase();
  return [...words(w?.excerpt)].filter((t) => !words(w?.description).has(t)).some((t) => q.includes(t));
}).length;
console.log(
  `  但 ${wonByExcerptOnly}/${wins.length} 次胜利靠的是 SKILL.md 正文里的措辞，那部分**不在 description 里**；` +
    `放长 description 补不回来，只有发正文才行（也就是把被移除的那次调用又加回来）。`,
);
