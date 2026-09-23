import AsyncStorage from "@react-native-async-storage/async-storage";

/**
 * The phone's own skill-recommendation budget.
 *
 * Each client keeps its own (the desktop has its own SQLite table, the TUI its
 * own file), so one user's daily total across clients is deliberately not
 * capped at three. Storage is AsyncStorage because the phone has no SQLite.
 *
 * Semantics (PRD v1.6 §7), the same as the other clients:
 * - the limit counts **recommendations shown**, not calls — a call that
 *   recommends nothing leaves no record, so the budget can outlive three calls;
 * - one skill is never shown twice in a day;
 * - one message is never asked about twice.
 */

/** Recommendations shown per local day. */
export const DAILY_LIMIT = 3;

const STORAGE_KEY = "futureos.mobile.skill-reco.v1";

export interface SkillRecoDay {
  /** Local calendar day (`YYYY-MM-DD`) this record belongs to. */
  day: string;
  /** Skill ids already recommended today. */
  skills: string[];
  /** Hashes of the messages that already produced a recommendation today. */
  messages: string[];
}

/** Local calendar day. `toISOString` is UTC, so it is not used here. */
export function today(now = new Date()): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

/** A day with nothing recorded yet. */
export function emptyDay(): SkillRecoDay {
  return { day: today(), skills: [], messages: [] };
}

/**
 * Stable, non-security hash of a draft, as hex. Used only to recognise a
 * message that already produced a recommendation today; the text itself is
 * never stored. FNV-1a over the UTF-8 bytes, matching the desktop and TUI.
 */
export function messageHash(text: string): string {
  let hash = 0xcbf29ce484222325n;
  const prime = 0x100000001b3n;
  const mask = 0xffffffffffffffffn;
  for (const byte of utf8Bytes(text.trim())) {
    hash = ((hash ^ BigInt(byte)) * prime) & mask;
  }
  return hash.toString(16).padStart(16, "0");
}

/** UTF-8 bytes without relying on `TextEncoder` (absent in the RN runtime). */
function utf8Bytes(text: string): number[] {
  const bytes: number[] = [];
  for (const char of text) {
    const code = char.codePointAt(0) ?? 0;
    if (code < 0x80) {
      bytes.push(code);
    } else if (code < 0x800) {
      bytes.push(0xc0 | (code >> 6), 0x80 | (code & 0x3f));
    } else if (code < 0x10000) {
      bytes.push(0xe0 | (code >> 12), 0x80 | ((code >> 6) & 0x3f), 0x80 | (code & 0x3f));
    } else {
      bytes.push(
        0xf0 | (code >> 18),
        0x80 | ((code >> 12) & 0x3f),
        0x80 | ((code >> 6) & 0x3f),
        0x80 | (code & 0x3f),
      );
    }
  }
  return bytes;
}

/**
 * Parse a stored record. Anything corrupt, from another day, or the wrong
 * shape reads as "nothing spent yet": a corrupt budget must never disable the
 * feature, and the worst case (one extra recommendation) is bounded because the
 * next record rewrites it.
 */
export function parseDay(raw: string | null, now = new Date()): SkillRecoDay {
  const fresh = { day: today(now), skills: [] as string[], messages: [] as string[] };
  if (raw === null) return fresh;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return fresh;
  }
  if (typeof parsed !== "object" || parsed === null) return fresh;
  const record = parsed as Partial<SkillRecoDay>;
  if (record.day !== fresh.day) return fresh;
  const strings = (value: unknown) =>
    Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
  const skills = strings(record.skills);
  // A hand-edited record cannot buy more than the limit.
  return { day: fresh.day, skills: skills.slice(0, DAILY_LIMIT), messages: strings(record.messages) };
}

export function count(day: SkillRecoDay): number {
  return day.skills.length;
}

export function exhausted(day: SkillRecoDay): boolean {
  return count(day) >= DAILY_LIMIT;
}

export function alreadyRecommended(day: SkillRecoDay, skill: string): boolean {
  return day.skills.includes(skill);
}

export function alreadyEvaluated(day: SkillRecoDay, hash: string): boolean {
  return day.messages.includes(hash);
}

/** Read today's record, treating any storage failure as "nothing yet". */
export async function loadDay(now = new Date()): Promise<SkillRecoDay> {
  try {
    return parseDay(await AsyncStorage.getItem(STORAGE_KEY), now);
  } catch {
    return { day: today(now), skills: [], messages: [] };
  }
}

/**
 * Record one shown recommendation. Best effort: a failed write costs at most one
 * extra recommendation today, which is not worth failing the send over.
 */
export async function recordShown(
  skill: string,
  hash: string,
  now = new Date(),
): Promise<SkillRecoDay> {
  const day = await loadDay(now);
  const next: SkillRecoDay = {
    day: day.day,
    skills: [...day.skills, skill],
    messages: [...day.messages, hash],
  };
  try {
    await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // ignored: see above
  }
  return next;
}

/** Test/DI seam: forget the in-process copy (not used in production paths). */
export async function clearDay(): Promise<void> {
  try {
    await AsyncStorage.removeItem(STORAGE_KEY);
  } catch {
    // ignored
  }
}
