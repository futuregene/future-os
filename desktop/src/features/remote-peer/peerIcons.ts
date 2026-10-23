/**
 * What a paired machine is called, and where its icons come from.
 *
 * The icon *set* — ids, order, labels, and the id-to-picture tables on both
 * clients — lives in `@future-os/peer-icons` and `./PeerIconGlyph`; this module
 * keeps only the display helpers that read a label. A component module cannot
 * also export these (fast refresh draws the line there), which is why the two
 * are apart.
 */
export { DEFAULT_PEER_ICON_ID, PEER_ICONS, peerIconId, peerIconTint } from "@future-os/peer-icons";
export type { PeerIcon, PeerIconId, PeerIconTint } from "@future-os/peer-icons";

/**
 * What to show for a host in a list: the name the user gave it, else a short
 * form of its identity. Short because the row is shared with a title and a
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
