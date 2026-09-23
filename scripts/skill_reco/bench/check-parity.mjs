// Assert the demo reproduces the agent's recommendation mechanism.
//
// The demo stands in for what ships: `docs/internals/skill_reco/evaluation.md` quotes
// its numbers as the cost and quality of the shipped path. That claim holds only while
// the two agree, and every way they can disagree is silent — a reworded question, a
// retuned gate, a different truncation width each leave both sides running, with only
// the report going wrong. So the values are read out of *both* sources here and
// compared, rather than each side asserting its own copy in its own test suite (two
// green tests that disagree are exactly the failure this catches).
//
// Why not share one implementation? The ablation scripts (`bench/stage1-prompt.mjs`,
// `bench/desc-cost.mjs`, the chunk sweeps) exist to vary precisely these values — that
// is how 0.15 / 254 / 220 were chosen. A shared library would leave them no knobs, so
// the demo keeps its own code and this check keeps it honest.
//
//   node bench/check-parity.mjs
//   AGENT_SKILL_RECO=/path/to/agent/src/skill_reco/mod.rs node bench/check-parity.mjs
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { DEFAULT_DESC_CHARS } from "../roster.mjs";
import {
  DEFAULT_FUTURE_BASE,
  DEFAULT_JEV_MODEL,
  FUTURE_PROVIDER,
  JEV_PATH,
  USD_PER_MTOK_INPUT,
  USD_TO_CNY,
} from "../jev.mjs";
import {
  INSTRUCTIONS,
  MAX_CHOICE_OPTIONS,
  MAX_CHUNK_SIZE,
  NONE_GATE_THRESHOLD,
  NONE_OF_THESE,
} from "../suggest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
// The agent ships in the same worktree once both branches land in main; the override
// covers a demo-only checkout (see docs/internals/skill_reco/harness.md).
const agentSource =
  process.env.AGENT_SKILL_RECO ??
  path.join(here, "..", "..", "..", "agent", "src", "skill_reco", "mod.rs");

let rust;
try {
  rust = fs.readFileSync(agentSource, "utf8");
} catch {
  console.error(`cannot read the agent's source at ${agentSource}`);
  console.error("set AGENT_SKILL_RECO to agent/src/skill_reco/mod.rs");
  process.exit(2);
}

/** The right-hand side of `const NAME: TY = VALUE;`. */
const raw = (name) => {
  const match = rust.match(new RegExp(`const ${name}\\s*:\\s*[^=;]+=\\s*([^;]+);`));
  return match ? match[1].trim() : undefined;
};
const rustText = (name) => {
  const match = raw(name)?.match(/^"((?:[^"\\]|\\.)*)"$/);
  return match ? match[1].replace(/\\"/g, '"').replace(/\\\\/g, "\\") : undefined;
};
const rustNumber = (name) => {
  const value = raw(name);
  return value === undefined ? undefined : Number(value);
};

// The question text lives inside the request builder's JSON, not in a `const`.
const instructions = rust.match(/"instructions":\s*"((?:[^"\\]|\\.)*)"/)?.[1];
const agentInstructions = instructions?.replace(/\\"/g, '"').replace(/\\\\/g, "\\");
// The path is asserted by the URL builder, which has to recognise one already appended.
const agentPath = rust.match(/ends_with\("([^"]*\/v1\/systemone)"\)/)?.[1];

const rows = [
  ["instructions", agentInstructions, INSTRUCTIONS],
  ["none gate", rustNumber("NONE_GATE_THRESHOLD"), NONE_GATE_THRESHOLD],
  ["candidate cap", rustNumber("MAX_CANDIDATES"), MAX_CHUNK_SIZE],
  ["desc chars", rustNumber("DESC_CHARS"), DEFAULT_DESC_CHARS],
  ["none option", rustText("NONE_OPTION"), NONE_OF_THESE],
  ["model", rustText("JEV_MODEL"), DEFAULT_JEV_MODEL],
  ["base origin", rustText("DEFAULT_FUTURE_BASE"), DEFAULT_FUTURE_BASE],
  ["provider key", rustText("FUTURE_PROVIDER"), FUTURE_PROVIDER],
  ["endpoint path", agentPath, JEV_PATH],
  ["price $/Mtok", rustNumber("USD_PER_MTOK_INPUT"), USD_PER_MTOK_INPUT],
  ["fx ¥/$", rustNumber("USD_TO_CNY"), USD_TO_CNY],
];

const show = (value) => {
  if (value === undefined) return "<not found>";
  const text = typeof value === "string" ? value : String(value);
  return text.length > 44 ? `${text.slice(0, 41)}...` : text;
};

let drift = 0;
console.log(`${"item".padEnd(14)} ${"agent".padEnd(46)} demo`);
for (const [label, agent, demo] of rows) {
  const same = agent !== undefined && agent === demo;
  if (!same) drift += 1;
  console.log(
    `${label.padEnd(14)} ${show(agent).padEnd(46)} ${show(demo)}  ${same ? "ok" : "DRIFT"}`,
  );
}

// Internal invariant: a Choice accepts at most 255 options and None occupies one, so
// the cap must stay one below the ceiling the API enforces.
const capOk = MAX_CHUNK_SIZE + 1 === MAX_CHOICE_OPTIONS;
if (!capOk) drift += 1;
console.log(
  `\ncap invariant  ${MAX_CHUNK_SIZE} candidates + 1 none = ${MAX_CHUNK_SIZE + 1} of ${MAX_CHOICE_OPTIONS} options  ${capOk ? "ok" : "DRIFT"}`,
);

if (drift === 0) {
  console.log("\nOK — the demo runs the agent's mechanism");
  process.exit(0);
}

// For the long one, say where it diverges; "instructions differ" alone is not actionable.
for (const [label, agent, demo] of rows) {
  if (typeof agent !== "string" || typeof demo !== "string") continue;
  if (agent === demo) continue;
  for (let i = 0; i < Math.max(agent.length, demo.length); i += 1) {
    if (agent[i] !== demo[i]) {
      console.log(
        `\n${label}: first difference at ${i} — agent=${JSON.stringify(agent[i])} demo=${JSON.stringify(demo[i])}`,
      );
      break;
    }
  }
}
console.log(`\nDRIFT — ${drift} value(s) differ; the demo no longer matches what ships`);
process.exit(1);
