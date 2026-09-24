import type { SkillCandidate, SkillRecoToday } from "../../integrations/skills/skillsClient";
import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  loadSkillCatalog,
  recordSkillReco,
  skillRecoToday,
  suggestSkill,
} from "../../integrations/skills/skillsClient";
import { useBuildInfo } from "../../integrations/tauri/useBuildInfo";

/**
 * Skill recommendation for any user message (not just a conversation's first).
 *
 * The trigger rules live here — the client owns them, the agent only answers
 * `suggest_skill` (PRD v1.6 §3, §7):
 * - feature toggle on (`skillRecommend`)
 * - logged in with a sufficient Future balance
 * - the draft is long enough (`MIN_QUERY_BYTES`) and short enough (`MAX_QUERY_CHARS`)
 * - the user has not already picked a skill (`/name`) in the draft
 * - today's budget is not spent (`DAILY_RECOMMENDATION_LIMIT` recommendations)
 * - this skill has not already been recommended today
 * - this message has not already produced a recommendation today
 *
 * On submit the caller awaits `evaluate()`: it either returns a recommendation
 * to show (submit is held), or `null` (submit proceeds). A hard budget
 * (`RECOMMEND_TIMEOUT_MS`) bounds the round-trip — on timeout/error/no-match the
 * caller just sends.
 *
 * The card's text (`shownDescription`) follows the UI language; the candidates
 * sent to the recommender do not — see that function for why.
 */

/** Below this many UTF-8 bytes the draft is too short to mean anything (10 汉字). */
export const MIN_QUERY_BYTES = 30;
/** Don't recommend for very long drafts (also keeps the Jev prompt small). */
export const MAX_QUERY_CHARS = 2000;
/** Recommendations shown per user per local day in a formal release build. */
export const DAILY_RECOMMENDATION_LIMIT = 3;
/** Generous test-build budget, so manual/repeated verification is not throttled. */
export const TEST_DAILY_RECOMMENDATION_LIMIT = 1000;

/**
 * Formal releases are the production boundary. All non-release builds are test
 * builds and use the larger budget; until build identity arrives, fail closed
 * to the production limit so the UI never briefly over-recommends.
 */
export function dailyRecommendationLimit(isRelease: boolean | null | undefined): number {
  return isRelease === false ? TEST_DAILY_RECOMMENDATION_LIMIT : DAILY_RECOMMENDATION_LIMIT;
}
/**
 * Hard budget for the whole recommend round-trip.
 *
 * 3 s rather than the original 1.5 s: the recommender's measured latency is
 * p50 ≈ 0.5 s but p95 ≈ 1.4 s (`docs/internals/skill_reco/evaluation.md`), so a
 * 1.5 s deadline discarded one answer in twenty *and the client cannot tell a
 * slow answer from no answer* — the card just never appears. The submit is held
 * for this long, which is why the input is locked and the send button spins for
 * the wait (see `Composer`): a frozen-looking box would be worse than the wait.
 */
export const RECOMMEND_TIMEOUT_MS = 3000;

export interface SkillRecommendationState {
  /** The recommendation currently shown as a card, or null. */
  recommendation: SkillCandidate | null;
}

export interface SkillRecommendationControls {
  state: SkillRecommendationState;
  /**
   * Evaluate the draft for a recommendation. Resolves to the skill to show
   * (and stores it in state), or null to submit normally. Never rejects.
   */
  evaluate: (draft: string) => Promise<SkillCandidate | null>;
  /** Dismiss the current card without installing. */
  dismiss: () => void;
  /** The candidate set (catalogue − installed), for display/testing. */
  candidates: SkillCandidate[];
}

interface Options {
  /** Feature toggle (appSettings.skillRecommend). */
  enabled: boolean;
  /** Future session status; recommendation requires an authenticated session. */
  sessionStatus: string;
  /** Future balance in credits; recommendation requires a positive balance. */
  balance: number | null;
}

/** True when the draft already picks at least one skill (`/name` pill). */
function draftPicksSkill(draft: string): boolean {
  return /(?:^|\s)\/[a-z0-9][a-z0-9-]*/i.test(draft);
}

/**
 * FNV-1a (64-bit) of the draft's UTF-8 bytes, as hex. Used only to recognise a
 * message that already produced a recommendation today; not security-sensitive,
 * so a short non-cryptographic hash is enough (and works in every runtime
 * without a crypto API).
 */
export function messageHash(text: string): string {
  const prime = 0x100000001B3n;
  const mask = 0xFFFFFFFFFFFFFFFFn;
  let hash = 0xCBF29CE484222325n;
  for (const byte of new TextEncoder().encode(text))
    hash = ((hash ^ BigInt(byte)) * prime) & mask;
  return hash.toString(16).padStart(16, "0");
}

/**
 * The text the card shows for a recommendation.
 *
 * The candidate sent to the recommender is always the catalogue's English
 * description: that payload is exactly what the 100-question evaluation measured
 * (`docs/internals/skill_reco/evaluation.md`, `stage1-zh.mjs`), and a display
 * decision must not move the numbers it was tuned on. The card therefore
 * follows the UI language here, on the way out, falling back to the English
 * text the agent echoed whenever the catalogue has no Chinese line for that
 * skill.
 */
export function shownDescription(
  card: SkillCandidate,
  language: string,
  zhById: Map<string, string>,
): string {
  // Matches `SkillsView`: anything that is not explicitly English is Chinese.
  if (language === "en")
    return card.description;
  const zh = zhById.get(card.name);
  return zh && zh.trim().length > 0 ? zh : card.description;
}

/** The empty day state, used before the first store read resolves. */
const EMPTY_TODAY: SkillRecoToday = { count: 0, skillIds: [], messageHashes: [] };

/**
 * Read today's state, tolerating an unusable response.
 *
 * A command this build does not have (older backend, or a transport that
 * resolves an unknown command to `null`) must behave as "no data yet" rather
 * than throwing: the caller then evaluates normally, and the budget can only
 * ever be under-counted, which fails towards recommending rather than towards
 * silently disabling the feature.
 */
async function readToday(): Promise<SkillRecoToday> {
  const value = await skillRecoToday().catch(() => null);
  if (!value || typeof value !== "object")
    return EMPTY_TODAY;
  return {
    count: typeof value.count === "number" ? value.count : 0,
    skillIds: Array.isArray(value.skillIds) ? value.skillIds : [],
    messageHashes: Array.isArray(value.messageHashes) ? value.messageHashes : [],
  };
}

export function useSkillRecommendation({
  enabled,
  sessionStatus,
  balance,
}: Options): SkillRecommendationControls {
  const { i18n } = useTranslation();
  const build = useBuildInfo();
  const [recommendation, setRecommendation] = useState<SkillCandidate | null>(null);
  const [candidates, setCandidates] = useState<SkillCandidate[]>([]);
  // Live mirror so evaluate() reads the latest gate values regardless of render
  // timing; a stale closure would otherwise reuse the first render's
  // toggle/balance for the whole session.
  const gateRef = useRef({ enabled, sessionStatus, balance });
  gateRef.current = { enabled, sessionStatus, balance };
  const candidatesRef = useRef(candidates);
  candidatesRef.current = candidates;
  const inFlightRef = useRef(false);
  const dailyLimitRef = useRef(dailyRecommendationLimit(build.data?.isRelease));
  dailyLimitRef.current = dailyRecommendationLimit(build.data?.isRelease);
  // The catalogue's Chinese descriptions, keyed by skill id, and the current
  // language — both read through refs because `evaluate` is created once and
  // must see the latest values (same reason as `gateRef` above).
  const zhDescriptionsRef = useRef<Map<string, string>>(new Map());
  const languageRef = useRef(i18n.language);
  languageRef.current = i18n.language;

  const loggedIn = sessionStatus === "authenticated" || sessionStatus === "unavailable";
  const hasBalance = balance === null || balance > 0;
  const active = enabled && loggedIn && hasBalance;

  // Load the candidate set (catalogue − installed) once the feature is active.
  // Both lists come from the shared cache, so this costs nothing when the
  // composer on the same screen has already read them.
  useEffect(() => {
    if (!active)
      return;
    let cancelled = false;
    const { installed, catalogue } = loadSkillCatalog();
    Promise.all([catalogue.catch(() => []), installed.catch(() => [])])
      .then(([all, mine]) => {
        if (cancelled)
          return;
        const installedIds = new Set(mine.map(s => s.id));
        setCandidates(
          all
            .filter(entry => !installedIds.has(entry.id))
            .map(entry => ({ name: entry.id, description: entry.description })),
        );
        zhDescriptionsRef.current = new Map(
          all.map(entry => [entry.id, entry.descriptionZh]),
        );
      })
      .catch(() => {});
    return () => {
      cancelled = true;
    };
  }, [active]);

  const evaluate = useCallback(async (draft: string): Promise<SkillCandidate | null> => {
    const gate = gateRef.current;
    const loggedInNow = gate.sessionStatus === "authenticated" || gate.sessionStatus === "unavailable";
    const hasBalanceNow = gate.balance === null || gate.balance > 0;
    const trimmed = draft.trim();
    if (
      !gate.enabled
      || !loggedInNow
      || !hasBalanceNow
      || trimmed.length === 0
      || trimmed.length > MAX_QUERY_CHARS
      || new TextEncoder().encode(trimmed).length < MIN_QUERY_BYTES
      || draftPicksSkill(trimmed)
      || candidatesRef.current.length === 0
      || inFlightRef.current
    ) {
      return null;
    }

    // Read the day state at submit time rather than caching it at mount: it is a
    // local SQLite read, and a second window (or a previous submission) may have
    // spent part of the budget since.
    const today = await readToday();
    // A spent budget stops the calls entirely — no call, no card.
    if (today.count >= dailyLimitRef.current)
      return null;
    const hash = messageHash(trimmed);
    if (today.messageHashes.includes(hash))
      return null;

    inFlightRef.current = true;
    try {
      const result = await Promise.race([
        suggestSkill(trimmed, candidatesRef.current).catch(() => null),
        new Promise<null>(resolve => setTimeout(resolve, RECOMMEND_TIMEOUT_MS, null)),
      ]);
      if (!result)
        return null;
      // Same skill twice in one day: skip this recommendation rather than
      // showing a duplicate (never fall back to a second-best skill).
      //
      // This keeps today's already-recommended skills *in* the candidate list
      // on purpose. PRD v1.6 §5 lists them as a pool exclusion, but §4 requires
      // that when the best match was already recommended we show nothing rather
      // than promote the runner-up — and that is only detectable if the model
      // can still pick the already-recommended skill. Excluding it up front
      // would silently turn the rule into "recommend the next best", which §4
      // forbids. The pool exclusion is therefore enforced here, as "never
      // shown twice", instead of by filtering the candidates.
      if (today.skillIds.includes(result.name))
        return null;
      // Record before showing: the card counts towards the daily budget the
      // moment it is displayed, regardless of what the user does with it.
      await recordSkillReco(result.name, hash).catch(() => {});
      const shown = {
        ...result,
        description: shownDescription(
          result,
          languageRef.current,
          zhDescriptionsRef.current,
        ),
      };
      setRecommendation(shown);
      return shown;
    }
    finally {
      inFlightRef.current = false;
    }
  }, []);

  const dismiss = useCallback(() => setRecommendation(null), []);

  return { state: { recommendation }, evaluate, dismiss, candidates };
}
