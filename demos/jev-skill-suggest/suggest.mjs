// The skill recommender: ONE call plus one threshold.
//
//   route   one Choice over the whole roster with a none_of_these option, so the model can decline.
//           The whole roster fits in one Choice up to 255 options; past that CHUNK_SIZE splits it,
//           each chunk gets its own none option, and a final Choice re-ranks the survivors on one
//           scale (within-chunk probabilities are not comparable across chunks — all the maxima sit
//           near 1.0).
//   gate    NONE_GATE_THRESHOLD reads how much probability that Choice put on none_of_these.
//
// The threshold reads the model's own answer rather than a probability picked off a ranking,
// because the Choice was asked to pick "none_of_these" when nothing fits. Asking "does this need a
// skill at all" as a separate question is worse — cross-validation shows it costs 8-9 false
// refusals — because a verdict with no candidates on the table to compare against is a much easier
// question to get wrong.
//
// The Choice's probabilities do saturate — a median of 2 options get any probability at all — but
// that turns out not to matter: the winner is usually the right skill. The correct skill is rank 1
// in 61/65 answerable questions and never falls outside the top 3, for both a single Choice and 8
// chunks (bench/onechunk-compare.mjs, bench/stage1-ranking.mjs). What a Choice without a none
// option could NOT do was refuse, which is why an earlier version scored 84.6% with a separate gate
// question: the refusal signal was missing, not the ranking.
//
// There used to be a second call ("stage 2"): the same question with the full description and the
// SKILL.md opening for each of the top 3, one Noul per candidate, with its own threshold. It is NOT
// in the serving path any more, for four measured reasons (bench/drop-stage2.mjs, REPORT §2.2.5):
//
//   * it decided 2 of 65 answers, and only 1 when a prediction that lands in the reference set
//     counts as correct (the reference for p019 is ["benchling-integration", "biopython"], and the
//     route had already picked biopython);
//   * its fit threshold never fired once, so refusal never depended on it;
//   * it extended the latency tail (p90 909 -> 1013 ms, p95 1063 -> 1287, p99 1946 -> 2339) while
//     the ranked list it re-ordered already contains the right answer every time;
//   * it answers a different question than the product asks. The agent that picks a skill to invoke
//     sees the `<name>` and `<description>` in its own skill list, which is exactly the routed
//     payload; only stage 2 could read SKILL.md, and one of its two wins (p019) came from question
//     wording that overlaps the winner's SKILL.md opening ("cloud ... platform ... sciences") but
//     not its description — an advantage a user who has not read the document would not grant.
//
// The removed call now lives in bench/second-call.mjs, which the benchmark imports so the recorded
// comparison in dataset/score.json and REPORT §2.2.5 stays reproducible. What is left here is
// exactly the serving path: no dead branch that could be mistaken for the real one.
//
// Validation (bench/chunked-cv.mjs, 5-fold, thresholds chosen on the training folds only):
// under the shipped operating point (one false suggestion allowed) this design and the previous
// 141-Noul one both score 62/65 held out, while this one uses 53% of the tokens (11,371 vs
// 21,357 per question) and produces fewer false suggestions (1 vs 2). Under a stricter
// zero-false-suggestion budget it is far ahead (87.7% vs 61.5%), because the none option can
// reject a single awkward question that a global threshold cannot separate.
import { JevClient } from "./jev.mjs";
import { buildLocalIndex, buildVocab, localRank } from "./localRank.mjs";

export const SHORTLIST = Number(process.env.SHORTLIST ?? 3);
/**
 * Skills per stage-1 Choice. The default is the largest chunk a Choice can hold (see
 * MAX_CHUNK_SIZE), so the whole roster goes in ONE Choice and nothing is split until the roster
 * itself outgrows the API.
 *
 * Measured against 18/30/47-skill chunks under held-out validation, a single Choice scored the same
 * (63/65) and cost 9% less, because chunking only adds a request and more repeated question
 * templates (bench/onechunk-compare.mjs). A chunked roster is also a *weaker* pipeline, not just a
 * dearer one: past 254 skills the chunks' probabilities are no longer comparable to each other
 * (each chunk's maximum sits near 1.0), so a final Choice has to re-rank the survivors.
 */
export const CHUNK_SIZE = Number(process.env.CHUNK_SIZE ?? 254);
/**
 * A Choice accepts at most 255 options total — not 255 skills. Measured, not assumed:
 * bench/option-limit.mjs sends 253/254/255/256 options and 256 is rejected with
 * 400 "Too many choices. Must have at most 255 choices.". none_of_these is one of the options, so a
 * chunk holds at most 255 - 1 = 254 skills. (The API reference states the 255 limit but does not
 * say whether the none option counts against it; the guard below exists because CHUNK_SIZE=255
 * would build 256 options and fail on every request.)
 */
export const MAX_CHOICE_OPTIONS = 255;
export const MAX_CHUNK_SIZE = MAX_CHOICE_OPTIONS - 1;
if (!Number.isInteger(CHUNK_SIZE) || CHUNK_SIZE < 1 || CHUNK_SIZE > MAX_CHUNK_SIZE) {
  throw new Error(
    `CHUNK_SIZE=${CHUNK_SIZE} would build ${CHUNK_SIZE + 1} options per Choice ` +
      `(${CHUNK_SIZE} skills + none_of_these), but a Choice accepts at most ` +
      `${MAX_CHOICE_OPTIONS} (bench/option-limit.mjs). Use 1–${MAX_CHUNK_SIZE}.`,
  );
}
/** The option name used for "nothing in this list fits", in stage 1's Choice(s). */
export const NONE_OF_THESE = "none_of_these";
/**
 * Refuse when stage 1's final Choice assigns at least this much probability to none_of_these.
 *
 * The floor is what separates a real request from one no skill serves: across the 100-question
 * set the refusable questions give none 0.9+ and the answerable ones 0.0x, so anything in
 * 0.1–0.9 makes no difference at all (bench/stage1-chunked-sweep.mjs). 0.15 is the midpoint of
 * what the cross-validation chose fold by fold (0.11–0.17), which is not coincidence: it is the
 * bottom of the empty band between the two groups. Re-measure it if the roster or the chunk size
 * changes.
 */
export const NONE_GATE_THRESHOLD = Number(process.env.NONE_GATE_THRESHOLD ?? 0.15);

/**
 * Bump this whenever the stage-1 request shape changes (the chunk questions, the none option, the
 * chunk size). `bench/predict-jev.mjs` stores both revisions with every answer and refuses to
 * resume from a cache produced by different ones.
 *
 * Two revisions rather than one because the stages change independently: forgetting to bump on a
 * stage-1 edit silently reuses the previous answers, which is how a criteria rewrite once appeared
 * to change nothing.
 */
export const STAGE1_REVISION = "stage1-one-choice-none-v3";
export const MIN_QUERY_CHARS = Number(process.env.MIN_QUERY_CHARS ?? 6);

/** Typed answers carry their value under a type-specific key: noul / choice / score. */
const number = (answer, key) => (typeof answer?.[key] === "number" ? answer[key] : null);
const round = (value) => (typeof value === "number" ? Number(value.toFixed(4)) : null);

export class Suggester {
  constructor(roster, { apiKey, baseUrl, model, insecureLocalOnly = false } = {}) {
    this.roster = roster;
    this.apiKey = apiKey || "";
    this.client = new JevClient({ apiKey: this.apiKey, baseUrl, model });
    const vocab = buildVocab(roster);
    this.index = buildLocalIndex(roster, { vocab });
    this.deepIndex = buildLocalIndex(roster, { vocab, excerptWeight: 5 });
    this.mode = this.apiKey && !insecureLocalOnly ? "typesafe" : "local";
    this.authChecked = false;
    this.note = this.apiKey
      ? "尚未验证 key"
      : "未提供 TYPESAFE_API_KEY，使用本地启发式排名（非 Jev）";
  }

  status() {
    return {
      mode: this.mode,
      note: this.note,
      baseUrl: this.client.baseUrl,
      model: this.client.model,
      apiKeyPresent: Boolean(this.apiKey),
      roster: {
        total: this.roster.skills.length,
        builtin: this.roster.builtinCount,
        thirdParty: this.roster.thirdPartyCount,
      },
      thresholds: {
        noneGate: NONE_GATE_THRESHOLD,
        chunkSize: CHUNK_SIZE,
        shortlist: SHORTLIST,
        minChars: MIN_QUERY_CHARS,
      },
      pipeline: `one Choice (${CHUNK_SIZE} skills per chunk, none_of_these), gated on the none probability`,
    };
  }

  /** Probe the key once (GET /v1/models); on 401/403 stay on the local fallback. */
  async checkAuth({ force = false } = {}) {
    if (!this.apiKey) return this.status();
    if (this.authChecked && !force) return this.status();
    this.authChecked = true;
    try {
      const { data } = await this.client.listModels();
      const models = Array.isArray(data?.data) ? data.data : Array.isArray(data) ? data : data?.models;
      this.mode = "typesafe";
      this.note = `key 有效；可用模型：${(models || []).map((m) => m?.id || m?.name || m).filter(Boolean).join(", ") || "—"}`;
    } catch (error) {
      this.mode = "local";
      this.note = `Jev API 不可用（${error.status || "网络"}）：${error.body?.detail?.message || error.message} → 已回退本地启发式排名`;
    }
    return this.status();
  }

  #demote(error) {
    if (error?.status === 401 || error?.status === 403) this.mode = "local";
    this.note = `Jev 调用失败（${error?.status || "网络"}）：${error?.body?.detail?.message || error?.message} → 本次回退本地排名`;
  }

  async rank(query) {
    if (this.mode === "typesafe") {
      try {
        return await this.#rankTypesafe(query);
      } catch (error) {
        this.#demote(error);
      }
    }
    return this.#rankLocal(query);
  }


  // ---------------------------------------------------------------- Jev (real)

  /** The roster split into chunks, computed once. */
  #chunks() {
    this._chunks ??= (() => {
      const chunks = [];
      for (let i = 0; i < this.roster.skills.length; i += CHUNK_SIZE) {
        chunks.push(this.roster.skills.slice(i, i + CHUNK_SIZE));
      }
      return chunks;
    })();
    return this._chunks;
  }

  /** One Choice per chunk, each with its own none_of_these option. */
  #chunkQuestions(query) {
    const questions = {};
    this.#chunks().forEach((chunk, index) => {
      const criteria = {};
      for (const skill of chunk) criteria[skill.name] = skill.indexLine;
      criteria[NONE_OF_THESE] = "No skill in this list would help with the request";
      questions[`chunk_${index}`] = {
        type: "choice",
        instructions: {
          question:
            "The request in \`request\` needs a skill from \`criteria\`. Which one, or does none of them help?",
          how_to_judge:
            `Pick the closest match if any is plausible, otherwise choose "${NONE_OF_THESE}". ` +
            "Choosing it is a normal answer here, not a fallback.",
        },
        criteria,
      };
    });
    return questions;
  }

  /**
   * Route the request: chunk Choices, then a final Choice over the survivors.
   *
   * Returns the ranking the gate reads (the final Choice's probabilities, none excluded),
   * the refusal signal, and both requests' usage so the caller can account for the cost honestly.
   */
  async #routeTypesafe(query) {
    const chunks = this.#chunks();
    const first = await this.client.systemOne({ state: { request: query }, questions: this.#chunkQuestions(query) });

    const survivors = [];
    const declinedChunks = [];
    chunks.forEach((chunk, index) => {
      const pick = first.answers?.[`chunk_${index}`]?.choice ?? null;
      if (pick && pick !== NONE_OF_THESE) survivors.push(pick);
      else declinedChunks.push(index);
    });

    const base = {
      chunkCount: chunks.length,
      declinedChunks,
      survivors,
      firstUsage: first.usage,
      model: first.model,
      firstRequest: first.request,
      firstAnswers: first.answers,
      firstMs: first.ms,
    };

    // One chunk means there is nothing to merge: the Choice's own probabilities already rank its
    // candidates, so the final Choice would only add a request (measured: 9% of the cost) and one
    // more refusal decision over a single candidate. Use this Choice as the ranking.
    if (chunks.length === 1) {
      const probabilities = first.answers?.chunk_0?.probabilities ?? {};
      const ranked = Object.entries(probabilities)
        .filter(([name]) => name !== NONE_OF_THESE)
        .map(([name, p]) => {
          const skill = this.roster.byName.get(name);
          return {
            name,
            p: round(p) ?? 0,
            builtin: skill?.builtin ?? null,
            category: skill?.category ?? null,
            description: skill?.description ?? "",
            nameZh: skill?.nameZh ?? "",
          };
        })
        .sort((a, b) => b.p - a.p);
      return {
        ...base,
        ranked,
        noneProbability: round(probabilities[NONE_OF_THESE]) ?? 0,
        secondUsage: null,
        secondRequest: null,
        secondMs: 0,
        topCandidate: first.answers?.chunk_0?.choice ?? null,
        merged: "single-choice",
      };
    }

    // Every chunk declined: no candidate exists, so there is nothing for the final Choice to rank.
    if (!survivors.length) {
      return {
        ...base,
        ranked: [],
        noneProbability: 1,
        secondUsage: null,
        secondRequest: null,
        secondMs: 0,
        topCandidate: null,
      };
    }

    const criteria = {};
    for (const name of survivors) criteria[name] = this.roster.byName.get(name)?.indexLine ?? "";
    criteria[NONE_OF_THESE] = "No skill fits this request; general assistance is enough";
    const second = await this.client.systemOne({
      state: { request: query },
      questions: {
        best_of_all: {
          type: "choice",
          instructions: {
            question: "Of \`criteria\`, which single skill would help most with the request in \`request\`?",
            how_to_judge:
              `Choose "${NONE_OF_THESE}" if none of them really does what the request needs. Rank by how ` +
              "well each skill's own description covers the request.",
          },
          criteria,
        },
      },
    });

    const probabilities = second.answers?.best_of_all?.probabilities ?? {};
    const ranked = Object.entries(probabilities)
      .filter(([name]) => name !== NONE_OF_THESE)
      .map(([name, p]) => {
        const skill = this.roster.byName.get(name);
        return {
          name,
          p: round(p) ?? 0,
          builtin: skill?.builtin ?? null,
          category: skill?.category ?? null,
          description: skill?.description ?? "",
          nameZh: skill?.nameZh ?? "",
        };
      })
      .sort((a, b) => b.p - a.p);

    return {
      ...base,
      ranked,
      noneProbability: round(probabilities[NONE_OF_THESE]) ?? 0,
      secondUsage: second.usage,
      secondRequest: second.request,
      secondMs: second.ms,
      topCandidate: second.answers?.best_of_all?.choice ?? null,
      merged: "final-choice",
    };
  }

  async #rankTypesafe(query) {
    const route = await this.#routeTypesafe(query);
    const totalTokens = (route.firstUsage?.input_tokens ?? 0) + (route.secondUsage?.input_tokens ?? 0);
    const gate = {
      noneProbability: route.noneProbability,
      threshold: NONE_GATE_THRESHOLD,
      chunkCount: route.chunkCount,
      declinedChunks: route.declinedChunks.length,
      survivorCount: route.survivors.length,
      top1: route.ranked[0]?.p ?? null,
      top2: route.ranked[1]?.p ?? null,
      // Kept so the existing UI/debug payloads have one number to show as "the gate reading".
      mean: route.noneProbability,
    };

    return {
      backend: "typesafe",
      model: route.model || this.client.model,
      ms: (route.firstMs ?? 0) + (route.secondMs ?? 0),
      attempts: 1,
      usage: { input_tokens: totalTokens },
      gate,
      ranked: route.ranked,
      top: route.ranked.slice(0, SHORTLIST),
      choices_seen: route.ranked.length,
      request: route.firstRequest,
      secondRequest: route.secondRequest,
      answer_count: Object.keys(route.firstAnswers ?? {}).length,
      declinedChunks: route.declinedChunks,
      survivors: route.survivors,
      noneProbability: route.noneProbability,
      topCandidate: route.topCandidate,
      stage1_tokens: route.firstUsage?.input_tokens ?? null,
      stage1b_tokens: route.secondUsage?.input_tokens ?? null,
    };
  }
  // ------------------------------------------------------- local fallback (lexical)

  #rankLocal(query) {
    const started = Date.now();
    const { ranked, gate } = localRank(this.index, query);
    const enriched = ranked.map((entry) => {
      const skill = this.roster.byName.get(entry.name);
      return {
        ...entry,
        builtin: skill?.builtin ?? null,
        category: skill?.category ?? null,
        description: skill?.description ?? "",
        nameZh: skill?.nameZh ?? "",
      };
    });
    return {
      backend: "local",
      model: null,
      ms: Date.now() - started,
      attempts: 0,
      usage: null,
      // The lexical stand-in has no "none" option to read, so it reports the same shape with the
      // refusal derived from its own coverage score. The UI labels it as the fallback either way.
      gate: {
        noneProbability: round(1 - gate.needs_skill) ?? null,
        threshold: NONE_GATE_THRESHOLD,
        chunkCount: null,
        declinedChunks: null,
        survivorCount: null,
        top1: enriched[0]?.p ?? null,
        top2: enriched[1]?.p ?? null,
        mean: round(1 - gate.needs_skill) ?? null,
      },
      ranked: enriched,
      top: enriched.slice(0, SHORTLIST),
      choices_seen: enriched.length,
      request: null,
      answer_count: enriched.length,
    };
  }

}
