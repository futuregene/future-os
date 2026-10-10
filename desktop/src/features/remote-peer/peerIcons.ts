/**
 * The icons a user can assign to a paired remote desktop.
 *
 * A fixed list, not free text: the glyph is rendered in place of the machine's
 * name in the conversation list, where it has to be (a) distinguishable at a
 * glance from the other machines' glyphs and (b) the same width, so rows do not
 * jump. A user-supplied emoji would fail both — and would also be the one
 * string that reaches another machine if it were ever synced by accident.
 *
 * The id (not the glyph) is what is stored, so the set can be re-rendered or
 * translated later without rewriting stored labels.
 */

export interface PeerIcon {
  id: string;
  glyph: string;
  /** i18n key under `remotePeer.icons.*`, so the picker reads in the app's language. */
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
 * Falls back to the **first** icon rather than returning the raw stored value:
 * an id this build does not know (a label written by a newer build, or a hand
 * edit) must not put arbitrary text into the list. The user's own machine is
 * drawn separately by the rail, so this fallback is only ever for a remote host.
 */
export function iconGlyph(id: string | null | undefined): string {
  if (!id)
    return PEER_ICONS[0]!.glyph;
  return PEER_ICONS.find(icon => icon.id === id)?.glyph ?? PEER_ICONS[0]!.glyph;
}

/**
 * What to show for a host in a list: the user's icon when set, else a short
 * form of the identity. Short because the row is shared with a title and a
 * timestamp — the full name is available in the host's own settings page.
 */
export function peerBadgeText(peer: { icon: string | null; name: string | null; desktopId: string }, fallback: string): string {
  if (peer.name)
    return peer.name;
  // `desktop_ab12cd34…` is opaque; the tail is the part that differs between
  // machines of the same account, so it is the part worth showing.
  const tail = peer.desktopId.replace(/^desktop_/, "").slice(0, 6);
  return tail || fallback;
}
