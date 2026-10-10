import type { SkillRecoSource } from "../agent/useSkillRecommendation";
import { requestRemotePeer } from "./remotePeerClient";

/** One candidate offered to the recommender. */
interface SkillCandidate {
  name: string;
  description: string;
}

/** Today's recommendation state, as the host keeps it. */
interface RemoteSkillRecoToday {
  count: number;
  skillIds: string[];
  messageHashes: string[];
}

const EMPTY_TODAY: RemoteSkillRecoToday = { count: 0, skillIds: [], messageHashes: [] };

/**
 * The skill recommender of a paired computer.
 *
 * A source for the same hook this app's own composer uses, so the trigger rules
 * — draft length, the `/name` rule, one evaluation at a time, the timeout, the
 * daily budget, never showing one skill twice in a day — are the rules that were
 * evaluated and tuned, rather than a second set written here.
 *
 * `accountGated: false`: the account that matters is that machine's, this one
 * cannot read its sign-in state, and the host already answers with "no
 * recommendation" when its own account cannot produce one (`suggest_skill` is
 * best-effort by contract).
 */
export function remoteSkillRecoSource(desktopId: string): SkillRecoSource {
  return {
    accountGated: false,

    /**
     * The host's own day state.
     *
     * A read that fails is an empty day rather than an error: the budget can
     * then only be *under*-counted, which fails towards recommending rather than
     * towards silently disabling the feature.
     */
    async today() {
      const raw = await requestRemotePeer<Record<string, unknown>>(
        desktopId,
        { type: "skill_reco_today" },
        "list",
      ).catch(() => null);
      const today = raw?.today;
      if (!today || typeof today !== "object")
        return EMPTY_TODAY;
      const state = today as Record<string, unknown>;
      return {
        count: typeof state.count === "number" ? state.count : 0,
        skillIds: Array.isArray(state.skillIds) ? state.skillIds.filter(id => typeof id === "string") as string[] : [],
        messageHashes: Array.isArray(state.messageHashes)
          ? state.messageHashes.filter(hash => typeof hash === "string") as string[]
          : [],
      };
    },

    /**
     * One recommendation from that host, or null.
     *
     * The host answers `skill: null` — with or without an error — for every way
     * it can decline: not signed in there, a timeout in its own recommender, no
     * match. So a refusal and a failure are the same answer here, which is what
     * a best-effort recommendation needs.
     */
    async suggest(query, candidates: SkillCandidate[]) {
      const raw = await requestRemotePeer<Record<string, unknown>>(
        desktopId,
        { type: "suggest_skill", query, candidates },
        "list",
      );
      const skill = raw?.skill;
      if (!skill || typeof skill !== "object")
        return null;
      const row = skill as Record<string, unknown>;
      if (typeof row.name !== "string" || !row.name)
        return null;
      return {
        name: row.name,
        description: typeof row.description === "string" ? row.description : "",
      };
    },

    /**
     * Record a card that was shown, against that host's daily budget.
     *
     * Only a shown card consumes the budget, which is why this is a separate
     * call the hook makes after deciding to show one.
     */
    async record(skillId, messageHash) {
      await requestRemotePeer(
        desktopId,
        { type: "record_skill_reco", skillId, messageHash },
        "list",
      );
    },
  };
}
