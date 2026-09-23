import type { SkillCandidate } from "../../integrations/skills/skillsClient";
import { useCallback, useEffect, useRef, useState } from "react";
import { listAvailableSkills, listInstalledSkills, suggestSkill } from "../../integrations/skills/skillsClient";

/**
 * First-turn skill recommendation for the new-conversation composer.
 *
 * The trigger rules live here (the client owns them; the agent only answers
 * `suggest_skill`):
 * - feature toggle on (`skillRecommend`)
 * - logged in with a sufficient Future balance
 * - the user has not already picked a skill (`/name`) in the draft
 * - the draft is short enough (`MAX_QUERY_CHARS`)
 *
 * On submit the caller awaits `evaluate()`: it either returns a recommendation
 * to show (submit is held), or `null` (submit proceeds). A hard 1 s budget
 * bounds the whole thing — on timeout/error/no-match the caller just sends.
 */

/** Don't recommend for very long drafts (also keeps the Jev prompt small). */
export const MAX_QUERY_CHARS = 2000;
/** Hard budget for the whole recommend round-trip, matching the product spec. */
const RECOMMEND_TIMEOUT_MS = 1000;

export interface SkillRecommendationState {
  /** The recommendation currently shown as a card, or null. */
  recommendation: SkillCandidate | null;
  /** True while a recommend round-trip is in flight (submit is held). */
  pending: boolean;
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

export function useSkillRecommendation({
  enabled,
  sessionStatus,
  balance,
}: Options): SkillRecommendationControls {
  const [recommendation, setRecommendation] = useState<SkillCandidate | null>(null);
  const [pending, setPending] = useState(false);
  const [candidates, setCandidates] = useState<SkillCandidate[]>([]);
  // Live mirror so evaluate() reads the latest gate values regardless of
  // render timing; a stale closure would otherwise reuse the first render's
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
      || draftPicksSkill(trimmed)
      || candidatesRef.current.length === 0
      || inFlightRef.current
    ) {
      return null;
    }

    inFlightRef.current = true;
    setPending(true);
    try {
      const result = await Promise.race([
        suggestSkill(trimmed, candidatesRef.current).catch(() => null),
        new Promise<null>(resolve => setTimeout(resolve, RECOMMEND_TIMEOUT_MS, null)),
      ]);
      if (result) {
        setRecommendation(result);
        return result;
      }
      return null;
    }
    finally {
      inFlightRef.current = false;
      setPending(false);
    }
  }, []);

  const dismiss = useCallback(() => setRecommendation(null), []);

  return { state: { recommendation, pending }, evaluate, dismiss, candidates };
}
