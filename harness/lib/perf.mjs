/**
 * Turning a measured number into a pass/fail judgement.
 *
 * An absolute ceiling ("boot under 8s") answers one question — "is this
 * catastrophically broken?" — and nothing else. Measured on this machine, the
 * boot time sits at 2.0–2.2s, so that ceiling tolerates a 4× regression; the
 * CLI's 800ms ceiling for a 9ms command tolerates 60×. A threshold nobody can
 * plausibly cross is not a test, it is decoration.
 *
 * So a metric can now be judged against a recorded baseline:
 *
 *   limit = max(baseline * (1 + tolerance), baseline + floor, floor)
 *
 * The `floor` matters for small or negative numbers: a 0.2ms p95 has no useful
 * multiplicative meaning, and heap growth is often negative. `max` stays as the
 * catastrophe guard, so both have to hold.
 *
 * Nothing here is per-surface: the desktop, CLI and TUI drivers all resolve a
 * limit through `resolveLimit`, so a threshold means the same thing everywhere.
 */

/** Nearest-rank percentile; null for no data rather than 0. */
export function percentile(values, p) {
  if (!values.length)
    return null;
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[Math.max(0, index)];
}

export function mean(values) {
  if (!values.length)
    return null;
  return values.reduce((sum, value) => sum + value, 0) / values.length;
}

/**
 * Least-squares slope of `values` over their index, in units per step.
 *
 * This is what makes "one switch costs 300ms" different from "each switch costs
 * 6ms more than the last": the second is a leak or an accidental O(n) inside a
 * repeated path, and only the series shows it.
 */
export function slope(values) {
  if (values.length < 2)
    return null;
  const n = values.length;
  const meanX = (n - 1) / 2;
  const meanY = mean(values);
  let num = 0;
  let den = 0;
  for (let i = 0; i < n; i++) {
    num += (i - meanX) * (values[i] - meanY);
    den += (i - meanX) ** 2;
  }
  return den === 0 ? 0 : num / den;
}

/**
 * How much slower the last third of a repeated interaction is than the first.
 *
 * Absolute latency differs per machine; drift does not. A ratio near 1 means
 * repetition is free, above ~1.5 means each pass leaves something behind.
 */
export function drift(values) {
  if (values.length < 6)
    return null;
  const third = Math.max(1, Math.floor(values.length / 3));
  const first = mean(values.slice(0, third));
  const last = mean(values.slice(-third));
  if (!first)
    return null;
  return Math.round((last / first) * 100) / 100;
}

/**
 * Resolve an `assertPerf` spec into the number it must stay under.
 *
 * Returns `{ limit, reason }`; `limit` is null when the spec carries no bound at
 * all, which the caller treats as "no evidence to judge".
 */
export function resolveLimit(spec, baseline) {
  const tolerance = spec.tolerance ?? 0.25;
  const base = baseline?.median ?? baseline?.value ?? null;
  const reasons = [];
  let limit = null;
  if (base !== null && base !== undefined) {
    // Signed metrics (heap/RSS growth, which is often negative) have no useful
    // multiplicative bound; `toleranceAbs` gives them one in their own unit.
    let candidate = spec.toleranceAbs
      ? base + Math.abs(spec.toleranceAbs)
      : base * (1 + tolerance);
    candidate = Math.max(candidate, spec.floor ?? -Infinity);
    limit = candidate;
    reasons.push(`baseline ${base}${spec.toleranceAbs ? ` + ${Math.abs(spec.toleranceAbs)}` : ` × (1 + ${tolerance})`}`);
  }
  if (typeof spec.max === "number") {
    limit = limit === null ? spec.max : Math.min(limit, spec.max);
    reasons.push(`ceiling ${spec.max}`);
  }
  return { limit, reason: reasons.join(" & ") || "no bound", baseline: base ?? null };
}

/**
 * Read the recorded baselines, tolerating their absence.
 *
 * A missing baseline must not fail a run: a fresh clone has no `perf/` history,
 * and turning "we never measured this" into a red build teaches people to edit
 * the file until it is green. Unknown keys simply fall back to the ceiling.
 */
export function readBaseline(file, { existsSync, readFileSync } = {}) {
  try {
    if (existsSync && !existsSync(file))
      return { metrics: {} };
    const parsed = JSON.parse(readFileSync(file, "utf8"));
    return { ...parsed, metrics: parsed.metrics ?? {} };
  }
  catch {
    return { metrics: {} };
  }
}

/** The key a case's `assertPerf` step looks up, so cases and data agree on one spelling. */
export function baselineKey(caseId, spec) {
  const parts = [caseId, spec.metric];
  if (spec.label)
    parts.push(spec.label);
  if (spec.stat)
    parts.push(spec.stat);
  return parts.join(":");
}

/** Render a value for a report line: integers stay integers, ratios keep 2 decimals. */
export function formatPerf(value, unit = "") {
  if (value === null || value === undefined)
    return "—";
  const rounded = Math.abs(value) >= 100 ? Math.round(value) : Math.round(value * 100) / 100;
  return unit ? `${rounded}${unit}` : String(rounded);
}
