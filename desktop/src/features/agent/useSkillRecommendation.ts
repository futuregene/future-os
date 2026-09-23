import type { SkillCandidate, SkillRecoToday } from "../../integrations/skills/skillsClient";
import { useCallback, useEffect, useRef, useState } from "react";
import {
  listAvailableSkills,
  listInstalledSkills,
  recordSkillReco,
  skillRecoToday,
  suggestSkill,
} from "../../integrations/skills/skillsClient";

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
 */

/** Below this many UTF-8 bytes the draft is too short to mean anything (10 汉字). */
export const MIN_QUERY_BYTES = 30;
/** Don't recommend for very long drafts (also keeps the Jev prompt small). */
export const MAX_QUERY_CHARS = 2000;
/** Recommendations shown per user per local day; a spent budget stops the calls. */
export const DAILY_RECOMMENDATION_LIMIT = 3;
/** Hard budget for the whole recommend round-trip, matching the product spec. */
const RECOMMEND_TIMEOUT_MS = 1500;

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

  const loggedIn = sessionStatus === "authenticated" || sessionStatus === "unavailable";
  const hasBalance = balance === null || balance > 0;
  const active = enabled && loggedIn && hasBalance;

  // Load the candidate set (catalogue − installed) once the feature is active.
  useEffect(() => {
    if (!active)
      return;
    let cancelled = false;
    Promise.all([
      listAvailableSkills().catch(() => [] as Awaited<ReturnType<typeof listAvailableSkills>>),
      listInstalledSkills().catch(() => [] as Awaited<ReturnType<typeof listInstalledSkills>>),
    ])
      .then(([catalogue, installed]) => {
        if (cancelled)
          return;
        const installedIds = new Set(installed.map(s => s.id));
        setCandidates(
          catalogue
            .filter(entry => !installedIds.has(entry.id))
            .map(entry => ({ name: entry.id, description: entry.description })),
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
    if (today.count >= DAILY_RECOMMENDATION_LIMIT)
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
      setRecommendation(result);
      return result;
    }
    finally {
      inFlightRef.current = false;
    }
  }, []);

  const dismiss = useCallback(() => setRecommendation(null), []);

  return { state: { recommendation }, evaluate, dismiss, candidates };
}
