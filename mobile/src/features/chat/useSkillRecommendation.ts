// The shape mirrors the desktop and TUI clients', so a message recognised on one
// client reads the same in the others' records.
import { useCallback, useRef, useState } from "react";
import { useRemote } from "../../remote/RemoteContext";
import type { AvailableSkill } from "../../remote/types";
import {
  alreadyEvaluated,
  alreadyRecommended,
  exhausted,
  loadDay,
  messageHash,
  recordShown,
} from "./skillRecoBudget";

/** One candidate offered to the recommender, and the recommendation shape. */
export interface SkillCandidate {
  name: string;
  description: string;
}

/**
 * Skill recommendation for the phone's composer (PRD v1.6).
 *
 * The phone cannot reach the agent, so the recommendation travels through the
 * paired desktop; the phone owns the trigger rules and its own daily budget.
 * Same rules as the desktop and TUI clients:
 * - the draft must be a real message (not a slash command), inside the length
 *   window, and not already name a skill;
 * - the day's budget must be unspent and this message not already asked about;
 * - a recommendation spends the budget when it is *shown*, whatever happens next.
 *
 * Best-effort: the desktop reports a declined or unavailable recommendation as
 * null, and every failure here sends the message instead.
 */

/** Shortest draft worth asking about: 30 UTF-8 bytes (10 汉字, ~30 ASCII). */
const MIN_QUERY_BYTES = 30;
/** Longest draft worth asking about; longer ones are sent immediately. */
const MAX_QUERY_CHARS = 2000;
/** Hard budget for the whole round-trip. 3 s — see the desktop hook's constant
 * for why (the recommender's p95 is ≈1.4 s, and the phone adds a relay hop), and
 * for the input lock that keeps the wait honest. */
export const RECOMMEND_TIMEOUT_MS = 3000;
/** A Choice accepts at most 255 options and "none of these" takes one. */
const MAX_CANDIDATES = 254;

export interface PendingSuggestion {
  /** The recommended skill, waiting for the user to decide. */
  skill: SkillCandidate;
  /** The draft it was recommended for, held so the message is not sent yet. */
  draft: string;
}

/** True when the draft already names a skill (`/name`), i.e. the user chose one. */
export function draftPicksSkill(draft: string): boolean {
  return draft.split(/\s+/).some(token => token.length > 1 && token.startsWith("/"));
}

/**
 * The text the card shows for a recommendation.
 *
 * The candidate sent to the recommender is always the catalogue's English
 * description — the payload the 100-question evaluation was tuned on
 * (`docs/internals/skill_reco/evaluation.md`) — so the card follows the UI
 * language here instead, falling back to the English text the desktop echoed
 * whenever the catalogue carries no Chinese line for that skill.
 */
export function shownDescription(
  card: SkillCandidate,
  language: string,
  zhById: Map<string, string>,
): string {
  // Matches `SkillPicker`: Mobile's language tags are `zh`/`en`.
  if (!language.startsWith("zh"))
    return card.description;
  const zh = zhById.get(card.name);
  return zh && zh.trim().length > 0 ? zh : card.description;
}

/** UTF-8 length without `TextEncoder` (absent in the RN runtime). */
export function utf8Length(text: string): number {
  let length = 0;
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    length += code < 0x80 ? 1 : code < 0x800 ? 2 : code < 0x10000 ? 3 : 4;
  }
  return length;
}

/** Resolve null when `promise` has not settled within the budget. */
function withTimeout(promise: Promise<SkillCandidate | null>): Promise<SkillCandidate | null> {
  return Promise.race([
    promise,
    new Promise<null>(resolve => setTimeout(() => resolve(null), RECOMMEND_TIMEOUT_MS)),
  ]);
}

export interface SkillRecommendationApi {
  /** The suggestion waiting for a decision, or null. */
  suggestion: PendingSuggestion | null;
  /**
   * True while the recommender is being asked about a draft.
   *
   * The caller locks the composer for this window (see `ComposerDock`): the
   * message that will be sent must be the one that was evaluated, and the
   * send button turns into a spinner so the wait is visible rather than the
   * composer looking dead.
   */
  evaluating: boolean;
  /**
   * Evaluate `draft`. Resolves true when a suggestion is shown (the caller must
   * NOT send yet), false when the message should be sent normally.
   */
  evaluate: (draft: string) => Promise<boolean>;
  /** Install the suggested skill, returning the draft with `/skill` appended. */
  installAndUse: () => Promise<string | null>;
  /** Dismiss the suggestion; the caller then sends the original draft. */
  dismiss: () => void;
}

export function useSkillRecommendation(
  enabled: boolean,
  desktopOnline: boolean,
  language: string,
): SkillRecommendationApi {
  const remote = useRemote();
  const [suggestion, setSuggestion] = useState<PendingSuggestion | null>(null);
  const [evaluating, setEvaluating] = useState(false);
  // One evaluation cannot overlap another. The toggle is a dependency of
  // `evaluate` below rather than a ref, so each evaluation sees the current
  // value without writing a ref during render.
  const inFlightRef = useRef(false);
  // The catalogue's Chinese descriptions, keyed by skill id, for the card's
  // text (see `shownDescription`) — filled by the candidate read below.
  const zhDescriptionsRef = useRef<Map<string, string>>(new Map());

  /** The uninstalled skills: the desktop's catalogue minus its installed set. */
  const candidateList = useCallback(async (): Promise<SkillCandidate[]> => {
    const [catalogue, installed] = await Promise.all([
      remote.listAvailableSkills().catch(() => [] as AvailableSkill[]),
      remote.listInstalledSkills().catch(() => []),
    ]);
    const installedIds = new Set(installed.map(skill => skill.id));
    zhDescriptionsRef.current = new Map(
      catalogue.map(entry => [entry.id, entry.descriptionZh ?? ""]),
    );
    return catalogue
      .filter(entry => !installedIds.has(entry.id))
      .map(entry => ({ name: entry.id, description: entry.description }))
      .slice(0, MAX_CANDIDATES);
  }, [remote]);

  const evaluate = useCallback(async (draft: string): Promise<boolean> => {
    const trimmed = draft.trim();
    if (
      !enabled
      || !desktopOnline
      || trimmed.length === 0
      || trimmed.length > MAX_QUERY_CHARS
      || utf8Length(trimmed) < MIN_QUERY_BYTES
      || draftPicksSkill(trimmed)
      || inFlightRef.current
    ) {
      return false;
    }

    inFlightRef.current = true;
    try {
      // Read the day per submission: another message (or an earlier attempt)
      // may have spent part of the budget since the last read.
      const today = await loadDay();
      if (exhausted(today)) return false;
      const hash = messageHash(trimmed);
      if (alreadyEvaluated(today, hash)) return false;

      const candidates = await candidateList();
      if (candidates.length === 0) return false;

      // Lock the composer for exactly the wait that can take the full budget —
      // the local budget/candidate checks above are fast and must not flash the
      // input disabled.
      setEvaluating(true);
      try {
        const answer = await withTimeout(remote.suggestSkill(trimmed, candidates));
        if (!answer) return false;
        // Already shown today: skip rather than offer a different skill (§4).
        if (alreadyRecommended(today, answer.name)) return false;
        // Recorded before it is shown: the card spends the budget the moment it
        // is displayed, whatever the user then does.
        await recordShown(answer.name, hash);
        setSuggestion({
          skill: {
            name: answer.name,
            description: shownDescription(answer, language, zhDescriptionsRef.current),
          },
          draft: trimmed,
        });
        return true;
      }
      finally {
        setEvaluating(false);
      }
    } catch {
      // Every failure means "send the message": a recommendation is never worth
      // failing a send over.
      return false;
    } finally {
      inFlightRef.current = false;
    }
  }, [candidateList, desktopOnline, enabled, language, remote]);

  const installAndUse = useCallback(async (): Promise<string | null> => {
    const pending = suggestion;
    if (!pending) return null;
    try {
      const catalogue = await remote.listAvailableSkills();
      const version = catalogue.find(entry => entry.id === pending.skill.name)?.latestVersion;
      if (!version) return null;
      await remote.installSkill(pending.skill.name, version);
      const separator = pending.draft.endsWith(" ") ? "" : " ";
      setSuggestion(null);
      return `${pending.draft}${separator}/${pending.skill.name}`;
    } catch {
      // Leave the card up so the user can retry or send without it.
      return null;
    }
  }, [remote, suggestion]);

  const dismiss = useCallback(() => setSuggestion(null), []);

  return { suggestion, evaluating, evaluate, installAndUse, dismiss };
}
