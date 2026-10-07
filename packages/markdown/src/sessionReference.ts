/**
 * The `#` session reference: one conversation pointing at another.
 *
 * A picked session serializes to a plain markdown link whose *destination*
 * carries the stable session id:
 *
 *     [Fix the flaky test](futureos://session/20261007-005402-418721fcc2...)
 *
 * The link form (rather than a bare `#id`) is what makes the reference usable
 * by every reader of the message: the composer shows the title as a pill, the
 * transcript renders it as a chip, and the agent receives the raw text — so the
 * session id travels into the conversation with no extra channel. Clients never
 * percent-encode the id: session ids are `[0-9a-f-]` only.
 *
 * Kept here (not in a client) because desktop, mobile and the TUI must agree on
 * one format — a reference written by one client has to be readable by another.
 */
export interface SessionReference {
  /** Stable agent session id; the payload the main conversation can act on. */
  sessionId: string;
  /** Display title at pick time. Cosmetic only — never trusted for lookup. */
  title: string;
}

export const SESSION_REFERENCE_URL_PREFIX = "futureos://session/";

/**
 * Label-safe title: `[`/`]` would close the markdown link early and truncate
 * the id, exactly as a `)` in a file path would. Parens read fine in prose and
 * round-trip through the parser unchanged.
 */
function sanitizeTitle(title: string): string {
  return title.replace(/[[\]]/g, "").replace(/\s+/g, " ").trim();
}

/** Build the markdown link for a picked session. */
export function buildSessionReference(reference: SessionReference): string {
  const title = sanitizeTitle(reference.title) || reference.sessionId;
  return `[${title}](${SESSION_REFERENCE_URL_PREFIX}${reference.sessionId})`;
}

/** The session id a reference href points at, or null for any other link. */
export function parseSessionReferenceHref(href: string): string | null {
  if (!href.startsWith(SESSION_REFERENCE_URL_PREFIX)) return null;
  const sessionId = href.slice(SESSION_REFERENCE_URL_PREFIX.length);
  if (!/^[A-Za-z0-9._-]+$/.test(sessionId)) return null;
  return sessionId;
}

export interface SessionReferenceMatch extends SessionReference {
  /** Offset of the whole `[title](href)` in the source string. */
  index: number;
  /** Length of the whole `[title](href)` in the source string. */
  length: number;
}

/** Matches a session reference link anywhere in a string. */
const SESSION_REFERENCE_LINK = /\[([^\]]*)\]\((futureos:\/\/session\/[^)\s]+)\)/g;

/**
 * Every session reference in `text`, in source order. Used by the transcript
 * renderer (chips) and the composer (restoring a draft's pills) so both read
 * the same pattern the builder writes.
 */
export function findSessionReferences(text: string): SessionReferenceMatch[] {
  const matches: SessionReferenceMatch[] = [];
  SESSION_REFERENCE_LINK.lastIndex = 0;
  for (
    let match = SESSION_REFERENCE_LINK.exec(text);
    match !== null;
    match = SESSION_REFERENCE_LINK.exec(text)
  ) {
    const sessionId = parseSessionReferenceHref(match[2] ?? "");
    if (!sessionId) continue;
    matches.push({
      index: match.index,
      length: match[0].length,
      sessionId,
      title: match[1] ?? "",
    });
  }
  return matches;
}
