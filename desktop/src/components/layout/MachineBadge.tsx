import type { ReactNode } from "react";

/**
 * Which machine a conversation is on, drawn in the conversation's header.
 *
 * The rail already says this per row — the machine's glyph rides with the title,
 * and the device selector above the list names the whole list's machine — but the
 * header said nothing at all, so a reader who opened a conversation from a merged
 * list had no way to tell an iMac's conversation from this computer's once the
 * list was out of sight. The header is where the rest of a conversation's
 * identity lives (its title, its spend), so it is where this goes.
 *
 * A qualifier, not a title: muted and small, and after the title it qualifies —
 * the same treatment the row gives it, so the two places read as one fact rather
 * than two.
 *
 * The glyph is supplied by the caller rather than looked up from an id: this
 * machine is drawn with an icon and a host with its own emoji, and which one it
 * is, is the caller's knowledge.
 */
export function MachineBadge({
  glyph,
  label,
  title,
}: {
  /** The machine's mark: the shared icon set for a host, and for this one. */
  glyph: ReactNode;
  /** The machine's name, or the shortest thing that identifies it. */
  label: string;
  /** The full name, for the tooltip — the label truncates to keep the title room. */
  title?: string;
}) {
  return (
    <span
      className="ml-2 inline-flex min-w-0 shrink-0 items-center gap-1 text-xs font-normal text-ink-muted"
      data-testid="thread-machine"
      title={title ?? label}
    >
      <span aria-hidden className="shrink-0">{glyph}</span>
      {/* Capped rather than free: a long machine name must not squeeze the
          conversation's own title, which is what the header is for. */}
      <span className="max-w-40 truncate">{label}</span>
    </span>
  );
}
