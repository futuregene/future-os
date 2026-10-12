// The shape mirrors the desktop and TUI clients', so a message recognised on one
// client reads the same in the others' records.
import { useCallback, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useRemote } from "../../remote/RemoteContext";
import { useCommittedRef } from "../../lib/useCommittedRef";
import { useConversationOperation } from "./useConversationOperation";
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
   * NOT send yet), or the operation was superseded. False permits normal send.
   */
  evaluate: (draft: string) => Promise<boolean>;
  /** Install the suggested skill, appending `/skill` to the current composer draft. */
  installAndUse: (currentDraft?: string) => Promise<string | null>;
  /** Dismiss the suggestion; the caller then sends the original draft. */
  dismiss: () => void;
}

export function useSkillRecommendation(
  enabled: boolean,
  desktopOnline: boolean,
  language: string,
): SkillRecommendationApi {
  const remote = useRemote();
  const { listAvailableSkills, listInstalledSkills, suggestSkill, installSkill } = remote;
  const { scope, captureOperation } = useConversationOperation(remote);
  const [card, setCard] = useState<{ scope: object; pending: PendingSuggestion; } | null>(null);
  const [evaluationScope, setEvaluationScope] = useState<object | null>(null);
  const suggestion = card?.scope === scope ? card.pending : null;
  const evaluating = evaluationScope === scope;
  const suggestionRef = useCommittedRef(suggestion);
  const languageRef = useCommittedRef(language);
  const inFlightRef = useRef<{ cancel: () => void; } | null>(null);
  useLayoutEffect(() => () => {
    inFlightRef.current?.cancel();
    inFlightRef.current = null;
  }, [scope]);

  const evaluate = useCallback(async (draft: string): Promise<boolean> => {
    if (inFlightRef.current) return true;
    const trimmed = draft.trim();
    if (!enabled || !desktopOnline || !trimmed
      || trimmed.length > MAX_QUERY_CHARS || utf8Length(trimmed) < MIN_QUERY_BYTES
      || draftPicksSkill(trimmed)) return false;
    const lifetimeIsCurrent = captureOperation();
    let active = true;
    const isCurrent = () => active && lifetimeIsCurrent();
    let cancel!: () => void;
    const cancelled = new Promise<boolean>(resolve => {
      cancel = () => { active = false; resolve(true); };
    });
    const attempt = { cancel };
    inFlightRef.current = attempt;
    setEvaluationScope(scope);
    let timeout: ReturnType<typeof setTimeout> | undefined;
    const work = async () => {
      const today = await loadDay();
      if (!isCurrent()) return true;
      if (exhausted(today)) return false;
      const hash = messageHash(trimmed);
      if (alreadyEvaluated(today, hash)) return false;
      const [catalogue, installed] = await Promise.all([
        listAvailableSkills().catch(() => [] as AvailableSkill[]),
        listInstalledSkills().catch(() => []),
      ]);
      if (!isCurrent()) return true;
      const installedIds = new Set(installed.map(skill => skill.id));
      const candidates = catalogue.filter(entry => !installedIds.has(entry.id))
        .map(entry => ({ name: entry.id, description: entry.description })).slice(0, MAX_CANDIDATES);
      if (!candidates.length) return false;
      const answer = await suggestSkill(trimmed, candidates);
      if (!isCurrent()) return true;
      if (!answer || alreadyRecommended(today, answer.name)) return false;
      await recordShown(answer.name, hash, new Date(), isCurrent);
      if (!isCurrent()) return true;
      setCard({
        scope, pending: {
          skill: {
            name: answer.name, description: shownDescription(answer, languageRef.current,
              new Map(catalogue.map(entry => [entry.id, entry.descriptionZh ?? ""])))
          },
          draft: trimmed,
        }
      });
      return true;
    };
    try {
      return await Promise.race([
        work(), cancelled,
        new Promise<boolean>(resolve => {
          timeout = setTimeout(() => {
            active = false;
            resolve(false);
          }, RECOMMEND_TIMEOUT_MS);
        }),
      ]);
    } catch {
      return !lifetimeIsCurrent();
    } finally {
      active = false;
      clearTimeout(timeout);
      if (inFlightRef.current === attempt) {
        inFlightRef.current = null;
        setEvaluationScope(null);
      }
    }
  }, [captureOperation, desktopOnline, enabled, languageRef, listAvailableSkills, listInstalledSkills, scope, suggestSkill]);

  const installAndUse = useCallback(async (currentDraft?: string): Promise<string | null> => {
    const pending = suggestionRef.current;
    if (!pending) return null;
    const draft = currentDraft ?? pending.draft;
    const isCurrent = captureOperation();
    try {
      const catalogue = await listAvailableSkills();
      if (!isCurrent()) return null;
      const version = catalogue.find(entry => entry.id === pending.skill.name)?.latestVersion;
      if (!version) return null;
      await installSkill(pending.skill.name, version);
      if (!isCurrent()) return null;
      setCard(null);
      return `${draft}${draft.endsWith(" ") ? "" : " "}/${pending.skill.name}`;
    } catch {
      return null;
    }
  }, [captureOperation, installSkill, listAvailableSkills, suggestionRef]);
  const dismiss = useCallback(() => setCard(null), []);
  return useMemo(() => ({ suggestion, evaluating, evaluate, installAndUse, dismiss }),
    [suggestion, evaluating, evaluate, installAndUse, dismiss]);
}
