// Local stand-in ranker, used only when the Jev API is unavailable (no key, 401, network).
// It is a lexical BM25 over the roster — no model, no semantics. It exists so the demo UI
// stays clickable before the API key works; every response it produces is labelled `local`.
//
// Chinese has no spaces, so a query run is segmented by greedy longest match against a
// vocabulary harvested from the roster itself. Tokens that appear in no skill are dropped
// instead of becoming noise, which is what keeps "解释一下什么是 monad" from matching skills.
const CJK = /[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]/;
const K1 = 1.2;
const B = 0.75;
const MAX_TERM = 4;
const clamp01 = (value) => Math.max(0, Math.min(1, Number(value.toFixed(4))));

const STOPWORDS = new Set([
  "a", "an", "the", "my", "our", "your", "this", "that", "these", "those", "it", "and", "or", "for", "to", "of",
  "in", "on", "at", "is", "are", "be", "with", "can", "you", "me", "i", "we", "help", "please", "want", "need",
  "using", "use", "get", "make", "do", "does", "how", "what", "from", "into",
  "我", "我们", "你", "请", "帮我", "帮忙", "一下", "这个", "这些", "那", "的", "是", "在", "把", "和", "与",
  "给", "从", "到", "想", "要", "能", "可以", "怎么", "什么", "如何", "一份", "一些", "一个", "需要", "帮我做",
]);

const cjkRuns = (text) => (text || "").toLowerCase().match(/[\u3040-\u30ff\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]{2,}/g) || [];

/** Vocabulary of CJK terms (2..4 chars) that actually occur in the roster. */
export function buildVocab(roster) {
  const vocab = new Set();
  for (const skill of roster.skills) {
    for (const text of [skill.name, skill.description, skill.descriptionZh, skill.category]) {
      for (const run of cjkRuns(text)) {
        for (let i = 0; i < run.length; i += 1) {
          for (let len = 2; len <= MAX_TERM && i + len <= run.length; len += 1) {
            vocab.add(run.slice(i, i + len));
          }
        }
      }
    }
  }
  return vocab;
}

export function tokenize(text, vocab) {
  const tokens = [];
  for (const chunk of (text || "").toLowerCase().split(/[^\p{L}\p{N}+.#_-]+/u)) {
    if (!chunk) continue;
    if (!CJK.test(chunk)) {
      const word = chunk.replace(/^[._-]+|[._-]+$/g, "");
      if (word.length > 1 && !STOPWORDS.has(word)) tokens.push(word);
      continue;
    }
    let i = 0;
    while (i < chunk.length) {
      let matched = 0;
      for (let len = MAX_TERM; len >= 2 && !matched; len -= 1) {
        if (i + len <= chunk.length && vocab.has(chunk.slice(i, i + len))) matched = len;
      }
      if (matched && !STOPWORDS.has(chunk.slice(i, i + matched))) tokens.push(chunk.slice(i, i + matched));
      i += matched || 1;
    }
  }
  return [...new Set(tokens)];
}

function fieldCounts(text, vocab) {
  const counts = new Map();
  for (const token of tokenize(text, vocab)) counts.set(token, (counts.get(token) || 0) + 1);
  return counts;
}

/** `excerptWeight` lets stage 2 lean on the SKILL.md body instead of the one-liner. */
export function buildLocalIndex(roster, { vocab, excerptWeight = 1 } = {}) {
  const weights = { name: 4, description: 2, descriptionZh: 2, excerpt: excerptWeight };
  const docs = [];
  const df = new Map();
  for (const skill of roster.skills) {
    const fields = {
      name: fieldCounts(skill.name.replace(/[-_]/g, " "), vocab),
      description: fieldCounts(skill.description, vocab),
      descriptionZh: fieldCounts(skill.descriptionZh, vocab),
      excerpt: fieldCounts(skill.excerpt.slice(0, 1200), vocab),
    };
    const tf = new Map();
    let length = 0;
    for (const [field, weight] of Object.entries(weights)) {
      for (const [token, count] of fields[field]) {
        const weighted = count * weight;
        tf.set(token, (tf.get(token) || 0) + weighted);
        length += weighted;
      }
    }
    for (const token of tf.keys()) df.set(token, (df.get(token) || 0) + 1);
    docs.push({ skill, tf, length });
  }
  const avgdl = docs.reduce((sum, doc) => sum + doc.length, 0) / Math.max(1, docs.length);
  return { docs, df, avgdl, size: docs.length, vocab };
}

const idfOf = (index, token) => {
  const df = index.df.get(token) || 0;
  if (!df) return 0;
  return Math.log(1 + (index.size - df + 0.5) / (df + 0.5));
};

/** A term no skill mentions still counts as query information that nothing covers. */
const unseenIdf = (index) => Math.log(1 + (index.size + 0.5) / 1.5);

/**
 * Partial match, both directions: the query's "报告" reaches the roster's "报告生成", and
 * "slides" reaches "slide". Discounted, because it is weaker evidence than an exact term.
 */
function partialMatch(doc, token) {
  if (token.length < 2) return null;
  for (const [term, count] of doc.tf) {
    if (term.length > token.length && term.startsWith(token)) return { tf: count, factor: 0.85 };
  }
  for (const [term, count] of doc.tf) {
    if (token.length > term.length && token.startsWith(term)) return { tf: count, factor: 0.7 };
  }
  return null;
}

/**
 * BM25 over `docs`, plus `coverage`: the share of the query's information (idf-weighted)
 * that this document actually contains. `coverage` drives the gate and the fit noul,
 * `raw` drives the ranking.
 */
function scoreDocs(index, tokens, docs) {
  // A term no skill mentions is worth little: it neither lifts a document nor, on its own,
  // makes the query look matched.
  const info = tokens.map((token) => {
    const known = index.df.has(token);
    const idf = known ? idfOf(index, token) : unseenIdf(index);
    return { token, idf, weight: known ? 1 : 0.35 };
  });
  const denom = info.reduce((sum, entry) => sum + entry.idf * entry.weight, 0) || 1;
  const scored = [];
  for (const doc of docs) {
    let raw = 0;
    let matchedIdf = 0;
    for (const entry of info) {
      let tf = doc.tf.get(entry.token);
      let factor = 1;
      if (tf === undefined) {
        const partial = partialMatch(doc, entry.token);
        if (!partial) continue;
        tf = partial.tf;
        factor = partial.factor;
      }
      const idf = entry.idf * entry.weight * factor;
      matchedIdf += idf;
      raw += idf * ((tf * (K1 + 1)) / (tf + K1 * (1 - B + (B * doc.length) / index.avgdl)));
    }
    if (raw > 0) scored.push({ name: doc.skill.name, raw, coverage: matchedIdf / denom, matchedIdf });
  }
  return scored.sort((a, b) => b.raw - a.raw);
}

function withProbabilities(scored) {
  if (!scored.length) return [];
  const peak = scored[0].raw;
  const temperature = Math.max(0.5, peak / 5);
  const weights = scored.map((item) => Math.exp(item.raw / temperature));
  const total = weights.reduce((sum, value) => sum + value, 0);
  return scored.map((item, i) => ({ ...item, p: Number((weights[i] / total).toFixed(4)) }));
}

export function localRank(index, text) {
  const tokens = tokenize(text, index.vocab);
  const ranked = withProbabilities(scoreDocs(index, tokens, index.docs));
  const coverage = ranked[0]?.coverage ?? 0;
  const needs = clamp01((coverage - 0.22) * 1.9);
  return {
    ranked,
    gate: {
      needs_skill: needs,
      operates_on_user_assets: clamp01((coverage - 0.3) * 1.9),
      follows_written_procedure: clamp01((coverage - 0.3) * 1.9),
      mean: needs,
    },
  };
}

export function localVerify(deepIndex, text, candidates) {
  const wanted = new Set(candidates.map((candidate) => candidate.name));
  const docs = deepIndex.docs.filter((doc) => wanted.has(doc.skill.name));
  const tokens = tokenize(text, deepIndex.vocab);
  const scored = new Map(scoreDocs(deepIndex, tokens, docs).map((item) => [item.name, item]));
  return candidates.map((candidate) => {
    const hit = scored.get(candidate.name);
    const coverage = hit?.coverage ?? 0;
    return { ...candidate, raw: hit?.raw ?? 0, coverage: Number(coverage.toFixed(4)), fit: clamp01((coverage - 0.25) * 2.6) };
  });
}
