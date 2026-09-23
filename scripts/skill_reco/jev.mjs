// Thin HTTP client for the System One endpoint (the Jev API), as served by the
// Future account's gateway.
//
//   POST https://future-os.cn/api/v1/systemone
//   Authorization: Bearer <the account key from ~/.future/agent/auth.json>
//   { state, model, questions: { id: NoulQuestion | ChoiceQuestion | ScoreQuestion } }
//
// The gateway speaks TypeSafe's question types but not its request shape: the
// question goes in `instructions` and `criteria` *is* the option map.
//
// Upstream docs: https://docs.typesafe.ai/api
import fs from "node:fs";
import path from "node:path";

export class JevError extends Error {
  constructor(status, body, path = JEV_PATH) {
    super(`System One gateway ${status} on ${path}: ${typeof body === "string" ? body : JSON.stringify(body)}`);
    this.name = "JevError";
    this.status = status;
    this.body = body;
  }
}

const RETRYABLE = new Set([429, 500, 502, 503, 529]);

/** The gateway's origin when the account has no `base_url` of its own. */
export const DEFAULT_FUTURE_BASE = "https://future-os.cn/api";
/** The model id the gateway resolves to a Jev build. */
export const DEFAULT_JEV_MODEL = "jev";
/** The `auth.json` entry that authorises the call — the Future account itself. */
export const FUTURE_PROVIDER = "future";
/** The system-one endpoint under the gateway origin. */
export const JEV_PATH = "/v1/systemone";

/**
 * Cost model, the same numbers the agent prices with. Exported so every script
 * that prints money derives it from one place *and* so `bench/check-parity.mjs`
 * can hold it against the agent's constants — a report quoting a stale rate is
 * worse than one quoting no rate at all.
 */
export const USD_PER_MTOK_INPUT = 0.042;
export const USD_TO_CNY = 7.2;

/**
 * The Future account's `auth.json` entry, or null when it cannot be read.
 *
 * The demo authenticates the same way the agent does — with the account's own
 * credential — so there is no separate Jev key to configure. `FUTURE_API_KEY`
 * overrides it for a throwaway key.
 */
function futureAccount() {
  if (process.env.FUTURE_API_KEY) {
    return { key: process.env.FUTURE_API_KEY, base: process.env.FUTURE_BASE_URL };
  }
  const home = process.env.HOME || process.env.USERPROFILE;
  if (!home) return null;
  const candidates = [
    path.join(home, ".future", "agent", "auth.json"),
    path.join(home, ".future", "agent-app", "auth.json"),
  ];
  for (const file of candidates) {
    try {
      const entry = JSON.parse(fs.readFileSync(file, "utf8"))?.[FUTURE_PROVIDER];
      if (entry?.key) return { key: entry.key, base: entry.base_url };
    } catch {
      // Missing or unreadable: try the next location.
    }
  }
  return null;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export class JevClient {
  constructor({ apiKey, baseUrl, model, timeoutMs = 90_000, maxRetries = 2 } = {}) {
    // `||`, not `??`: callers signal "not configured" with an empty string, and
    // `??` would let that empty value win instead of reaching the account.
    const account = apiKey ? null : futureAccount();
    this.apiKey = apiKey || account?.key || "";
    // The account's own base URL wins; the documented origin is the fallback.
    this.baseUrl = (baseUrl || account?.base || DEFAULT_FUTURE_BASE).replace(/\/+$/, "");
    this.model = model || process.env.FUTURE_MODEL || DEFAULT_JEV_MODEL;
    this.timeoutMs = timeoutMs;
    this.maxRetries = maxRetries;
  }

  async #call(pathname, { method = "GET", body } = {}) {
    let attempt = 0;
    for (;;) {
      const started = Date.now();
      let response;
      try {
        response = await fetch(`${this.baseUrl}${pathname}`, {
          method,
          headers: {
            authorization: `Bearer ${this.apiKey ?? ""}`,
            ...(body ? { "content-type": "application/json" } : {}),
          },
          body: body ? JSON.stringify(body) : undefined,
          signal: AbortSignal.timeout(this.timeoutMs),
        });
      } catch (error) {
        if (attempt < this.maxRetries) {
          await sleep(600 * 2 ** attempt);
          attempt += 1;
          continue;
        }
        throw new JevError(0, `network error: ${error.message}`, pathname);
      }

      const text = await response.text();
      let parsed;
      try {
        parsed = text ? JSON.parse(text) : null;
      } catch {
        parsed = text;
      }

      if (response.ok) {
        return { data: parsed, ms: Date.now() - started, attempts: attempt + 1 };
      }

      if (RETRYABLE.has(response.status) && attempt < this.maxRetries) {
        const retryAfter = Number(response.headers.get("retry-after"));
        await sleep(Number.isFinite(retryAfter) && retryAfter > 0 ? retryAfter * 1000 : 700 * 2 ** attempt);
        attempt += 1;
        continue;
      }
      throw new JevError(response.status, parsed, pathname);
    }
  }

  listModels() {
    return this.#call("/v1/models");
  }

  /** `questions` is a plain map of id -> typed question. Returns the typed answers. */
  async systemOne({ state, questions, model }) {
    const payload = { state, model: model || this.model, questions };
    const { data, ms, attempts } = await this.#call(JEV_PATH, { method: "POST", body: payload });
    return { answers: data?.answers ?? {}, model: data?.model, usage: data?.usage, ms, attempts, request: payload, raw: data };
  }
}
