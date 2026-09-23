// Assert the demo sends exactly the instruction string the agent sends.
//
// Reads the agent's literal straight out of the Rust source and the demo's export
// straight out of its module, then compares them — a cross-implementation check
// rather than two tests each asserting their own copy. The demo stands in for what
// ships, so a wording change on one side alone means the demo lies.
//
//   node bench/check-instructions.mjs
//   AGENT_SKILL_RECO=/path/to/agent/src/skill_reco/mod.rs node bench/check-instructions.mjs
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { INSTRUCTIONS } from "../suggest.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));
// The agent lives in the product worktree; allow an override for other layouts.
const agentSource =
  process.env.AGENT_SKILL_RECO ??
  path.join(here, "..", "..", "..", "..", "agent", "src", "skill_reco", "mod.rs");

let rust;
try {
  rust = fs.readFileSync(agentSource, "utf8");
} catch {
  console.error(`cannot read the agent's source at ${agentSource}`);
  console.error("set AGENT_SKILL_RECO to agent/src/skill_reco/mod.rs");
  process.exit(2);
}

// The `instructions` value in `build_request`, as a single Rust string literal.
const match = rust.match(/"instructions":\s*"((?:[^"\\]|\\.)*)"/);
if (!match) {
  console.error("could not find the `instructions` literal in the agent's build_request");
  process.exit(2);
}
// Rust escapes it wrote: `\"` for a quote, `\\` for a backslash.
const agent = match[1].replace(/\\"/g, '"').replace(/\\\\/g, "\\");

const same = agent === INSTRUCTIONS;
console.log(`agent : ${agent}`);
console.log(`demo  : ${INSTRUCTIONS}`);
console.log(same ? "\nOK — identical" : "\nDRIFT — the demo no longer matches production");
if (!same) {
  for (let i = 0; i < Math.max(agent.length, INSTRUCTIONS.length); i += 1) {
    if (agent[i] !== INSTRUCTIONS[i]) {
      console.log(
        `first difference at ${i}: agent=${JSON.stringify(agent[i])} demo=${JSON.stringify(INSTRUCTIONS[i])}`,
      );
      break;
    }
  }
}
process.exit(same ? 0 : 1);
