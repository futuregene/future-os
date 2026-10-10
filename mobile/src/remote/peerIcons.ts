/**
 * The icons a user can assign to a paired desktop.
 *
 * A fixed set, not free text, and stored by id: the glyph is rendered in place
 * of the desktop's name in the session list, where it has to be same-width and
 * distinguishable from the other desktops' glyphs at a glance. A user-supplied
 * string would fail both — and would be the one piece of free text that could
 * reach another machine if it were ever synced by mistake.
 *
 * Kept in sync with the desktop app's own set by name: the two platforms let a
 * user label the same machine, and a glyph that exists on one and not the other
 * would make the same desktop look like two different ones.
 */
export interface PeerIcon {
  id: string;
  glyph: string;
  /** i18n key under `desktops.icons.*`. */
  labelKey: string;
}

export const PEER_ICONS: PeerIcon[] = [
  { id: "desktop", glyph: "🖥", labelKey: "desktop" },
  { id: "laptop", glyph: "💻", labelKey: "laptop" },
  { id: "server", glyph: "🖧", labelKey: "server" },
  { id: "cloud", glyph: "☁", labelKey: "cloud" },
  { id: "home", glyph: "🏠", labelKey: "home" },
  { id: "office", glyph: "🏢", labelKey: "office" },
  { id: "rocket", glyph: "🚀", labelKey: "rocket" },
  { id: "flask", glyph: "🧪", labelKey: "flask" },
];

/**
 * The glyph for a stored id.
 *
 * An id this build does not know — a label written by a newer build, or a
 * hand-edited store — renders the default rather than the raw value: this text
 * ends up on screen next to a session title, so it must never be arbitrary.
 */
export function iconGlyph(id: string | null | undefined): string {
  if (!id) return PEER_ICONS[0]!.glyph;
  return PEER_ICONS.find(icon => icon.id === id)?.glyph ?? PEER_ICONS[0]!.glyph;
}
