#!/usr/bin/env node
// One-call decision-model run over the FROZEN 141-skill roster of the published evaluation
// (docs/internals/skill_reco/evaluation.md), so another decision model can be scored against
// Jev on the exact same options, questions and gold answers.
//
// It deliberately reuses the shipped pipeline (../suggest.mjs → Suggester.rank): one Choice
// over the whole roster plus the none_of_these gate. Nothing here re-implements the prompt,
// so a change to the serving path changes this run with it.
//
//   node decider-run.mjs run <run-name> [--model M] [--base URL] [--limit N] [--concurrency N]
//   node decider-run.mjs score <run-name>
//   node decider-run.mjs compare <runA> <runB> [...]
//
// Smoke-test a new endpoint with `--limit 5` before the full set: the usual failure with a
// new model is not a wrong answer but an illegal one (free text, several options, no
// none_of_these), and finding that out after 100 questions wastes the whole run.
//
// Records land in runs/<run-name>/<id>.json (gitignored). A record carries the config that
// produced it, so a re-run resumes only on a matching config — the failure this guards
// against is scoring stale answers as if they were new (evaluation.md §2.2.6).
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { loadRoster } from "../roster.mjs";
import {
  Suggester,
  INSTRUCTIONS,
  NONE_OF_THESE,
  NONE_GATE_THRESHOLD,
  STAGE1_REVISION,
  CHUNK_SIZE,
} from "../suggest.mjs";
import { USD_PER_MTOK_INPUT } from "../jev.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.dirname(here); // scripts/skill_reco
const readJson = (file) => JSON.parse(fs.readFileSync(file, "utf8"));
const dataset = (file) => readJson(path.join(ROOT, "dataset", file));

const FROZEN_ROSTER = dataset("roster.json").skills;
const QUESTIONS = dataset("questions.json").questions;
// gold-v1.json stores `answers` as an array (one row per question, with its own id).
const GOLD = new Map(dataset("gold-v1.json").answers.map((row) => [row.id, row]));

/**
 * Materialise the frozen roster as a skills tree so `loadRoster` (the shipped reader) can be
 * reused unchanged: same collapsing to one line, same 220-character truncation, same order.
 * The live skills/ submodule has grown past 141 skills; those extra options would change the
 * task and invalidate the gold answers, so a run comparable to the published numbers needs
 * the roster the numbers were produced on. Re-running the *current* catalogue is a separate
 * exercise: it needs a fresh gold pass, and the two models must be run in the same batch.
 */
function frozenSkillsRoot() {
  const dir = path.join(os.tmpdir(), "skill-reco-frozen-141");
  const stamp = path.join(dir, ".stamp");
  if (fs.existsSync(stamp)) return dir;
  fs.rmSync(dir, { recursive: true, force: true });
  for (const skill of FROZEN_ROSTER) {
    const folder = path.join(dir, skill.builtin ? "builtin" : "third-party", skill.name);
    fs.mkdirSync(folder, { recursive: true });
    const description = String(skill.description ?? "").replace(/\s+/g, " ").trim();
    fs.writeFileSync(
      path.join(folder, "SKILL.md"),
      `---\nname: ${skill.name}\ndescription: >-\n  ${description}\n---\n\n(frozen roster stub)\n`,
    );
  }
  fs.writeFileSync(stamp, new Date().toISOString());
  return dir;
}

const params = () => {
  const argv = process.argv.slice(2);
  const command = argv[0];
  const rest = argv.slice(1);
  const flag = (name) => {
    const at = rest.indexOf(`--${name}`);
    return at === -1 ? undefined : rest[at + 1];
  };
  const positional = rest.filter((value, index) => !value.startsWith("--") && !rest[index - 1]?.startsWith("--"));
  return { command, positional, flag };
};

function makeSuggester({ model, baseUrl, apiKey }) {
  const roster = loadRoster(frozenSkillsRoot());
  if (roster.skills.length !== FROZEN_ROSTER.length) {
    throw new Error(`frozen roster materialised ${roster.skills.length} skills, expected ${FROZEN_ROSTER.length}`);
  }
  return { suggester: new Suggester(roster, { apiKey, baseUrl, model }), roster };
}

const median = (values) => {
  const sorted = values.filter((v) => typeof v === "number").sort((a, b) => a - b);
  return sorted.length ? sorted[Math.floor(sorted.length / 2)] : null;
};
const percentile = (values, p) => {
  const sorted = values.filter((v) => typeof v === "number").sort((a, b) => a - b);
  return sorted.length ? sorted[Math.min(sorted.length - 1, Math.floor(sorted.length * p))] : null;
};
const mean = (values) => (values.length ? values.reduce((sum, v) => sum + v, 0) / values.length : null);
const pct = (value) => (value === null || value === undefined ? "  —   " : `${(value * 100).toFixed(1).padStart(5)}%`);

/**
 * The first request of the run is also its format check.
 *
 * A decision model that answers a Choice with free text, with several options, or without the
 * none_of_these key does not throw: `Suggester.rank` reads the missing probabilities as an
 * empty distribution, so every question would be recorded as a confident refusal and the run
 * would look like a model that refuses everything. One call up front turns that into an error.
 */
async function legalChoice(suggester, roster, text) {
  const criteria = Object.fromEntries(roster.skills.map((skill) => [skill.name, skill.indexLine]));
  criteria[NONE_OF_THESE] = "No skill in this list would help with the request";
  const questions = { chunk_0: { type: "choice", instructions: INSTRUCTIONS, criteria } };
  const call = await suggester.client.systemOne({ state: { request: text }, questions });
  const answer = call.answers?.chunk_0;
  const probabilities = answer?.probabilities;
  const problems = [];
  if (!answer) problems.push("no answers.chunk_0 in the response");
  if (answer && typeof answer.choice !== "string") problems.push("answers.chunk_0.choice is not a single string");
  if (!probabilities || typeof probabilities !== "object") problems.push("answers.chunk_0 has no probabilities object");
  const notes = [];
  if (probabilities && typeof probabilities === "object") {
    if (!(NONE_OF_THESE in probabilities)) problems.push(`probabilities has no ${NONE_OF_THESE} key`);
    const unknown = Object.keys(probabilities).filter((name) => name !== NONE_OF_THESE && !roster.byName.has(name));
    if (unknown.length) problems.push(`${unknown.length} option name(s) not in the roster: ${unknown.slice(0, 3).join(", ")}`);
    const sum = Object.values(probabilities).reduce((total, p) => total + (typeof p === "number" ? p : 0), 0);
    if (Math.abs(sum - 1) > 0.05) notes.push(`probabilities sum to ${sum.toFixed(3)}`);
    if (!roster.skills.some((skill) => (probabilities[skill.name] ?? 0) > 0)) notes.push("no skill carries any probability");
  }
  if (problems.length) {
    throw new Error(
      `the model did not answer with a legal Choice — do not run the full set.\n  ${problems.join("\n  ")}\n` +
        `  raw: ${JSON.stringify(call.raw).slice(0, 600)}`,
    );
  }
  return { call, answer, notes };
}

async function run() {
  const { positional, flag } = params();
  const name = positional[0];
  if (!name) throw new Error("usage: decider-run.mjs run <run-name> [--model M] [--base URL] [--limit N]");
  const model = flag("model") || process.env.FUTURE_MODEL;
  const baseUrl = flag("base") || process.env.FUTURE_BASE_URL;
  // Sequential by default: under concurrency this client measures queueing at the gateway,
  // not the model, so a parallel run's latency is not comparable with a published number.
  const concurrency = Number(flag("concurrency") || 1);
  const limit = flag("limit") ? Number(flag("limit")) : QUESTIONS.length;

  const { suggester, roster } = makeSuggester({ model, baseUrl, apiKey: process.env.FUTURE_API_KEY });
  const config = {
    model: suggester.client.model,
    baseUrl: suggester.client.baseUrl,
    roster: roster.skills.length,
    stage1: STAGE1_REVISION,
    chunk: CHUNK_SIZE,
    gate: NONE_GATE_THRESHOLD,
  };
  console.log(`run "${name}" → ${config.baseUrl}/v1/systemone · model ${config.model} · roster ${config.roster}`);
  console.log(`config ${JSON.stringify(config)}`);

  const legal = await legalChoice(suggester, roster, QUESTIONS[0].text);
  const probe = await suggester.rank(QUESTIONS[0].text);
  if (probe.backend !== "gateway") throw new Error(`probe fell back to local ranking: ${suggester.note}`);
  if (JSON.stringify(legal.call.request) !== JSON.stringify(probe.request)) {
    throw new Error("the format check sent a different request than the serving path — it no longer covers this run");
  }
  for (const note of legal.notes) console.log(`warning: ${note}`);
  console.log(
    `probe ok: choice=${legal.answer.choice} none=${probe.noneProbability} top=${probe.top[0]?.name ?? "-"} ` +
      `(${probe.ms} ms, ${probe.stage1_tokens} tok)`,
  );

  const dir = path.join(ROOT, "runs", name);
  fs.mkdirSync(dir, { recursive: true });
  const isCurrent = (id) => {
    const file = path.join(dir, `${id}.json`);
    if (!fs.existsSync(file)) return false;
    try {
      return JSON.stringify(readJson(file).config) === JSON.stringify(config);
    } catch {
      return false;
    }
  };

  const queue = QUESTIONS.slice(0, limit).filter((q) => !isCurrent(q.id));
  const total = queue.length;
  console.log(`${QUESTIONS.slice(0, limit).length - queue.length} cached · ${queue.length} to run`);
  let done = 0;

  const workers = Array.from({ length: Math.min(concurrency, queue.length || 1) }, async () => {
    for (;;) {
      const question = queue.shift();
      if (!question) return;
      let result;
      try {
        result = await suggester.rank(question.text);
      } catch (error) {
        throw new Error(`${question.id}: ${error.message}`);
      }
      if (result.backend !== "gateway") {
        throw new Error(`${question.id}: fell back to local ranking (${suggester.note})`);
      }
      const gate = result.gate.noneProbability < NONE_GATE_THRESHOLD ? result.top.map((entry) => entry.name) : [];
      fs.writeFileSync(
        path.join(dir, `${question.id}.json`),
        `${JSON.stringify(
          {
            id: question.id,
            config,
            gate,
            answer: gate.slice(0, 1),
            none_probability: result.gate.noneProbability,
            top: result.ranked.slice(0, 10).map((entry) => ({ name: entry.name, p: entry.p })),
            model_answered: result.model,
            stage1_tokens: result.stage1_tokens,
            stage1_ms: result.ms,
            answers_seen: result.choices_seen,
          },
          null,
          2,
        )}\n`,
      );
      done += 1;
      process.stdout.write(`\r${name} ${done}/${total}   `);
    }
  });
  await Promise.all(workers);
  console.log("");
  score(name);
}

function loadRun(name) {
  const dir = path.join(ROOT, "runs", name);
  if (!fs.existsSync(dir)) throw new Error(`no such run: ${name}`);
  const rows = fs
    .readdirSync(dir)
    .filter((file) => file.endsWith(".json"))
    .map((file) => readJson(path.join(dir, file)));
  return new Map(rows.map((row) => [row.id, row]));
}

function evaluate(rows, ids) {
  const goldOf = (id) => GOLD.get(id)?.gold ?? [];
  const agree = (pred, reference) => (reference.length === 0 ? pred.length === 0 : pred[0] === reference[0]);
  const scored = ids.map((id) => {
    const reference = goldOf(id);
    const pred = rows.get(id)?.answer ?? [];
    const question = QUESTIONS.find((q) => q.id === id);
    return {
      id,
      text: question?.text ?? "",
      source_skill: question?.source_skill ?? null,
      gold: reference,
      pred,
      agree: agree(pred, reference),
      top1: reference.length ? pred[0] === reference[0] : null,
      inReference: reference.length ? reference.includes(pred[0]) : null,
      falseRefuse: reference.length > 0 && pred.length === 0,
      falseSuggest: reference.length === 0 && pred.length > 0,
      sourceTop1: question?.source_skill ? pred[0] === question.source_skill : null,
      tokens: rows.get(id)?.stage1_tokens ?? null,
      ms: rows.get(id)?.stage1_ms ?? null,
      none: rows.get(id)?.none_probability ?? null,
    };
  });
  const on = (bucket) => scored.filter((row) => bucket.includes(row.id));
  const filled = ids.filter((id) => goldOf(id).length > 0);
  const empty = ids.filter((id) => goldOf(id).length === 0);
  const provenance = ids.filter((id) => QUESTIONS.find((q) => q.id === id)?.source_skill);
  const flat = (rows2) => rows2.map((row) => Number(row));
  const tokens = scored.map((row) => row.tokens);
  const ms = scored.map((row) => row.ms);
  return {
    rows: scored,
    n: ids.length,
    agree: mean(flat(on(ids).map((row) => row.agree))),
    top1: mean(flat(on(filled).map((row) => row.top1))),
    inReference: mean(flat(on(filled).map((row) => row.inReference))),
    refusal: mean(on(empty).map((row) => Number(!row.falseSuggest))),
    falseRefuse: mean(on(filled).map((row) => Number(row.falseRefuse))),
    sourceTop1: mean(on(provenance).map((row) => Number(row.sourceTop1))),
    tokens: median(tokens),
    tokensTotal: tokens.reduce((sum, value) => sum + (value ?? 0), 0),
    ms: median(ms),
    msMean: mean(ms.filter((v) => typeof v === "number")),
    msP90: percentile(ms, 0.9),
  };
}

function score(name) {
  const rows = loadRun(name);
  const ids = QUESTIONS.filter((q) => rows.has(q.id)).map((q) => q.id);
  const missing = QUESTIONS.filter((q) => !rows.has(q.id));
  const config = rows.values().next().value?.config;
  const r = evaluate(rows, ids);
  const usdPerQ = ((r.tokens ?? 0) * USD_PER_MTOK_INPUT) / 1e6;
  console.log(`\n=== ${name} · ${config?.model ?? "?"} ===`);
  console.log(`config ${JSON.stringify(config)}`);
  if (missing.length) console.log(`INCOMPLETE: ${missing.length} questions missing (${missing.slice(0, 5).map((q) => q.id).join(", ")})`);
  console.log(`  决策一致 ${pct(r.agree)} · 首答正确 ${pct(r.top1)} · 首答∈参照集 ${pct(r.inReference)}`);
  console.log(`  正确拒答 ${pct(r.refusal)} · 误拒 ${pct(r.falseRefuse)} · 首答=出处技能 ${pct(r.sourceTop1)}`);
  console.log(
    `  输入 token 中位 ${r.tokens}（${ids.length} 题 ${r.tokensTotal}）· 延迟中位 ${r.ms} ms · p90 ${r.msP90} ms · 均值 ${Math.round(r.msMean ?? 0)} ms`,
  );
  console.log(`  按 $0.042/Mtok 每题 $${usdPerQ.toFixed(6)}（Jev 公示价；另一家模型的价目要按其 own 费率折算）`);
  return r;
}

function compare(...names) {
  const runs = names.map((name) => ({ name, rows: loadRun(name) }));
  const ids = QUESTIONS.map((q) => q.id);
  for (const entry of runs) {
    const missing = ids.filter((id) => !entry.rows.has(id));
    if (missing.length) throw new Error(`${entry.name} is incomplete: ${missing.length} missing`);
  }
  const results = runs.map(({ name, rows }) => ({ name, ...evaluate(rows, ids) }));
  const gold = (id) => (GOLD.get(id)?.gold ?? []);
  console.log(
    `\n=== 对比（${ids.length} 题：${ids.filter((id) => gold(id).length).length} 有答案 / ${ids.filter((id) => !gold(id).length).length} 应拒答）===`,
  );
  console.log(`  ${"run".padEnd(22)} 决策一致 首答正确 首答∈参照集 正确拒答 误拒  token/题  p50      p90`);
  for (const r of results) {
    console.log(
      `  ${r.name.padEnd(22)} ${pct(r.agree)} ${pct(r.top1)} ${pct(r.inReference).padStart(9)} ${pct(r.refusal).padStart(8)} ${pct(r.falseRefuse).padStart(6)} ${String(r.tokens).padStart(7)} ${String(r.ms).padStart(6)}ms ${String(r.msP90).padStart(6)}ms`,
    );
  }
  if (results.length < 2) return;
  // The paired flip list is the part that survives the ±2-question noise of a single run.
  const [a, b] = results;
  const flips = a.rows.filter((row) => {
    const other = b.rows.find((entry) => entry.id === row.id);
    return JSON.stringify(row.pred) !== JSON.stringify(other?.pred);
  });
  const tag = (pred, reference) =>
    reference.length === 0 ? (pred.length ? "误推" : "拒答") : pred.length ? (pred[0] === reference[0] ? "对" : "错") : "误拒";
  console.log(`\n=== 逐题差异（${a.name} → ${b.name}）：${flips.length} 道 ===`);
  for (const row of flips) {
    const other = b.rows.find((entry) => entry.id === row.id);
    console.log(
      `  ${row.id} ${tag(row.pred, row.gold).padEnd(3)} ${(row.pred[0] ?? "—").padEnd(26)} → ${tag(other.pred, other.gold).padEnd(3)} ${(other.pred[0] ?? "—").padEnd(26)} gold=${JSON.stringify(row.gold)}  ${row.text.slice(0, 46)}`,
    );
  }
  const fixed = flips.filter((row) => {
    const other = b.rows.find((entry) => entry.id === row.id);
    return row.gold.length ? other.pred[0] === row.gold[0] : other.pred.length === 0;
  }).length;
  const broke = flips.filter((row) => {
    const other = b.rows.find((entry) => entry.id === row.id);
    const otherRight = row.gold.length ? other.pred[0] === row.gold[0] : other.pred.length === 0;
    return !otherRight && (row.gold.length ? row.pred[0] === row.gold[0] : row.pred.length === 0);
  }).length;
  console.log(`  ${b.name} 改对 ${fixed} 道、改错 ${broke} 道、其余 ${flips.length - fixed - broke} 道两边都错`);
}

const { command } = params();
if (command === "run") await run();
else if (command === "score") score(params().positional[0]);
else if (command === "compare") compare(...params().positional);
else throw new Error("usage: decider-run.mjs run <name> | score <name> | compare <runA> <runB>");
