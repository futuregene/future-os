// Shared plumbing for the 100-question comparison.
//
// Models are reached through the local Future agent (`future run`), embeddings through the
// omlx server. Every step is resume-safe: it writes one JSON file per item under
// `runs/<name>/`, so a re-run only does the work that is missing.
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
export const DATASET_DIR = path.join(ROOT, "dataset");
export const RUNS_DIR = path.join(ROOT, "runs");

export const MODELS = {
  gold: process.env.GOLD_MODEL || "future/kimi-k3",
  llm: process.env.LLM_MODEL || "future/deepseek-flash",
  embed: process.env.EMBED_MODEL || "Qwen3-Embedding-0.6B-4bit-DWQ",
  embedBaseUrl: process.env.EMBED_BASE_URL || "http://127.0.0.1:8000/v1",
};

export const ensureDir = (dir) => {
  mkdirSync(dir, { recursive: true });
  return dir;
};

/**
 * The LLM passes run with this as their working directory, on purpose.
 *
 * `future run --system-prompt` replaces only the identity section of the agent's system
 * prompt; the project context (CLAUDE.md) and the FUTURE.md workspace-memory index are still
 * appended, and both mention skill names in passing. Running from an empty directory keeps
 * the prompt to the router instruction plus an environment block, so the comparison measures
 * the routing decision rather than what the repo happens to say about its own skills.
 * (`--no-tools` is what keeps the skill catalog itself out: the agent only injects the
 * <available_skills> block when the `read` tool is present.)
 */
export const NEUTRAL_CWD = ensureDir(path.join(tmpdir(), "jev-skill-bench-cwd"));

export const readJson = (file, fallback = null) => (existsSync(file) ? JSON.parse(readFileSync(file, "utf8")) : fallback);
export const writeJson = (file, value) => {
  ensureDir(path.dirname(file));
  writeFileSync(file, `${JSON.stringify(value, null, 2)}\n`);
};

/** `future run` one-shot: no tools, no session, replaced system prompt, JSON envelope. */
export function llm({ model, prompt, systemPrompt, thinking, cwd = NEUTRAL_CWD, timeoutMs = 600_000 }) {
  const args = ["run", "--model", model, "--no-tools", "--no-session", "--mode", "json", "--cwd", cwd];
  if (thinking) args.push("--thinking", thinking);
  if (systemPrompt) args.push("--system-prompt", systemPrompt);
  args.push(prompt);

  return new Promise((resolve, reject) => {
    const started = Date.now();
    const child = spawn("future", args, { stdio: ["ignore", "pipe", "pipe"] });
    let out = "";
    let err = "";
    const timer = setTimeout(() => {
      child.kill("SIGKILL");
      reject(new Error(`timeout after ${timeoutMs} ms`));
    }, timeoutMs);
    child.stdout.on("data", (chunk) => (out += chunk));
    child.stderr.on("data", (chunk) => (err += chunk));
    child.on("error", reject);
    child.on("close", (code) => {
      clearTimeout(timer);
      if (code !== 0) return reject(new Error(`future run exit ${code}: ${err.slice(-400)}`));
      let envelope;
      try {
        envelope = JSON.parse(out);
      } catch {
        return reject(new Error(`unparseable future run output: ${out.slice(-400)}`));
      }
      // The envelope's `messages` are the run's event stream; the usage event carries the
      // model's token counts and the platform's own credit cost for the run.
      const usage = (envelope.messages ?? [])
        .filter((event) => event?.type === "usage" && event.usage)
        .reduce(
          (acc, event) => ({
            prompt_tokens: (acc.prompt_tokens ?? 0) + (event.usage.prompt_tokens ?? 0),
            completion_tokens: (acc.completion_tokens ?? 0) + (event.usage.completion_tokens ?? 0),
            cache_read_tokens: (acc.cache_read_tokens ?? 0) + (event.usage.cache_read_tokens ?? 0),
            reasoning_tokens: (acc.reasoning_tokens ?? 0) + (event.usage.reasoning_tokens ?? 0),
            credit_cost: (acc.credit_cost ?? 0) + (event.usage.credit_cost ?? 0),
          }),
          {},
        );
      resolve({
        text: envelope.text ?? "",
        model: envelope.model,
        usage: Object.keys(usage).length ? usage : null,
        durationMs: Date.now() - started,
        stderr: err.slice(-400),
      });
    });
  });
}

/** Models like to wrap JSON in prose or fences; take the outermost object. */
export function extractJson(text) {
  if (!text) return null;
  const fenced = /```(?:json)?\s*([\s\S]*?)```/.exec(text);
  const body = fenced ? fenced[1] : text;
  const start = body.indexOf("{");
  const end = body.lastIndexOf("}");
  if (start === -1 || end <= start) return null;
  try {
    return JSON.parse(body.slice(start, end + 1));
  } catch {
    return null;
  }
}

const escapeRe = (text) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/**
 * Reads skill names out of a free-form answer. The models sometimes ignore the JSON-only
 * instruction and reply with markdown ("**Load: `future-account`** — it's the exact match");
 * the names are still unambiguous, so parse them rather than counting the answer as a failure.
 * Matched on word boundaries, ordered by first mention, capped at 3.
 */
export function extractSkillNames(text, known) {
  if (!text) return [];
  const found = [];
  for (const name of known) {
    const re = new RegExp(`(?<![A-Za-z0-9_-])${escapeRe(name)}(?![A-Za-z0-9_-])`);
    const match = re.exec(text);
    if (match) found.push({ name, index: match.index });
  }
  return found.sort((a, b) => a.index - b.index).map((entry) => entry.name).slice(0, 3);
}

export function cosine(a, b) {
  let dot = 0;
  let na = 0;
  let nb = 0;
  for (let i = 0; i < a.length; i += 1) {
    dot += a[i] * b[i];
    na += a[i] * a[i];
    nb += b[i] * b[i];
  }
  return dot / (Math.sqrt(na) * Math.sqrt(nb) || 1);
}

export async function embed(texts) {
  const response = await fetch(`${MODELS.embedBaseUrl}/embeddings`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ model: MODELS.embed, input: texts }),
  });
  if (!response.ok) throw new Error(`embeddings ${response.status}: ${(await response.text()).slice(0, 200)}`);
  const body = await response.json();
  return {
    vectors: body.data.map((entry) => entry.embedding),
    // omlx reports the prompt token count; worth recording so the embedding cost is honest.
    tokens: body.usage?.prompt_tokens ?? null,
  };
}

/** Runs `worker` over `items` with bounded concurrency, retrying failures once. */
export async function pool(items, worker, { concurrency = 4, label = "" } = {}) {
  const queue = [...items];
  let done = 0;
  const results = [];
  const runners = Array.from({ length: Math.min(concurrency, queue.length) }, async () => {
    for (;;) {
      const item = queue.shift();
      if (item === undefined) return;
      let result;
      try {
        result = await worker(item);
      } catch (error) {
        await new Promise((r) => setTimeout(r, 1500));
        try {
          result = await worker(item);
        } catch (retryError) {
          result = { error: retryError.message };
        }
      }
      results.push({ item, result });
      done += 1;
      if (label) process.stdout.write(`\r${label} ${done}/${items.length}   `);
    }
  });
  await Promise.all(runners);
  if (label) process.stdout.write("\n");
  return results;
}

/** Per-item cache directory: `runs/<name>/<id>.json`. */
export const cacheFor = (name) => {
  const dir = ensureDir(path.join(RUNS_DIR, name));
  return {
    dir,
    has: (id) => existsSync(path.join(dir, `${id}.json`)),
    get: (id) => readJson(path.join(dir, `${id}.json`)),
    put: (id, value) => writeJson(path.join(dir, `${id}.json`), value),
    all: () => readdirSync(dir).filter((f) => f.endsWith(".json")).map((f) => readJson(path.join(dir, f))),
  };
};

/**
 * The roster as every system sees it: one line per skill, grouped by category. Identical text
 * for the gold pass and for the LLM baseline, so the comparison is about the decision, not the
 * prompt. (Embeddings and Jev do not use this block — they read the same fields their own way.)
 */
export function rosterIndexBlock(skills) {
  const byCategory = new Map();
  for (const skill of skills) {
    if (!byCategory.has(skill.category)) byCategory.set(skill.category, []);
    byCategory.get(skill.category).push(skill);
  }
  const lines = [];
  for (const category of [...byCategory.keys()].sort()) {
    lines.push(`${category}:`);
    for (const skill of byCategory.get(category).sort((a, b) => a.name.localeCompare(b.name))) {
      lines.push(`- ${skill.name}: ${skill.description}`);
    }
  }
  return lines.join("\n");
}

/** Drops hallucinated names, de-duplicates, keeps order, caps at 3. */
export function normalizeSkills(names, known) {
  const out = [];
  for (const raw of names ?? []) {
    const name = String(raw).trim();
    if (!known.has(name) || out.includes(name)) continue;
    out.push(name);
    if (out.length === 3) break;
  }
  return out;
}

export const QUESTIONS_FILE = path.join(DATASET_DIR, "questions.json");
export const ROSTER_FILE = path.join(DATASET_DIR, "roster.json");
