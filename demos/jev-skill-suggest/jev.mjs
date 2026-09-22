// Thin HTTP client for TypeSafe's System One endpoint (the Jev API).
//
//   POST https://api.typesafe.ai/v1/systemone
//   Authorization: Bearer <key>
//   { state, model, questions: { id: NoulQuestion | ChoiceQuestion | ScoreQuestion } }
//
// Docs: https://docs.typesafe.ai/api
export class JevError extends Error {
  constructor(status, body, path = "/v1/systemone") {
    super(`TypeSafe ${status} on ${path}: ${typeof body === "string" ? body : JSON.stringify(body)}`);
    this.name = "JevError";
    this.status = status;
    this.body = body;
  }
}

const RETRYABLE = new Set([429, 500, 502, 503, 529]);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export class JevClient {
  constructor({
    apiKey,
    baseUrl = process.env.TYPESAFE_BASE_URL || "https://api.typesafe.ai",
    model = process.env.TYPESAFE_MODEL || "jev-latest",
    timeoutMs = 90_000,
    maxRetries = 2,
  } = {}) {
    this.apiKey = apiKey;
    this.baseUrl = baseUrl.replace(/\/+$/, "");
    this.model = model;
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
    const { data, ms, attempts } = await this.#call("/v1/systemone", { method: "POST", body: payload });
    return { answers: data?.answers ?? {}, model: data?.model, usage: data?.usage, ms, attempts, request: payload, raw: data };
  }
}
