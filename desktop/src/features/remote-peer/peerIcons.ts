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
 *
 * Every glyph must be a **single RGI emoji** (`unicode.org/Public/emoji/…
 * /emoji-test.txt`), with `U+FE0F` where Unicode gives the code point a text
 * presentation by default. Both halves matter, and the two ways of getting it
 * wrong are silent:
 *
 *  - A code point that is not an emoji at all (`U+1F5A7`, the old `server`
 *    glyph, is the network-computers pictograph, not an emoji) has no glyph in
 *    any emoji font, so phones drew a tofu box.
 *  - A code point whose default presentation is *text* (`U+1F5A5`, `U+2601`)
 *    without `U+FE0F` falls back to the monochrome text glyph, so the same
 *    machine looked washed out here and coloured on mobile.
 *
 * The ids are stable and shared with mobile
 * (`mobile/src/remote/peerIcons.ts`); the two lists must stay identical, and
 * each side's test reads the other file to prove it.
 */

export interface PeerIcon {
  id: string;
  glyph: string;
  /** i18n key under `remotePeer.icons.*`, so the picker reads in the app's language. */
  labelKey: string;
}

export const PEER_ICONS: PeerIcon[] = [
  // What kind of machine it is.
  { id: "desktop", glyph: "🖥️", labelKey: "desktop" },
  { id: "laptop", glyph: "💻", labelKey: "laptop" },
  { id: "phone", glyph: "📱", labelKey: "phone" },
  { id: "server", glyph: "🗄️", labelKey: "server" },
  { id: "cloud", glyph: "☁️", labelKey: "cloud" },
  // Which OS it runs. Emoji stand-ins for the three logos: a real logo would
  // be an image in the picker *and* in the row, so it stays inside the one
  // glyph the rest of the set is drawn with — 🪟 window, 🍎 apple, 🐧 Tux.
  { id: "windows", glyph: "🪟", labelKey: "windows" },
  { id: "macos", glyph: "🍎", labelKey: "macos" },
  { id: "linux", glyph: "🐧", labelKey: "linux" },
  // Where it stands.
  { id: "home", glyph: "🏠", labelKey: "home" },
  { id: "office", glyph: "🏢", labelKey: "office" },
  { id: "cafe", glyph: "☕", labelKey: "cafe" },
  { id: "school", glyph: "🏫", labelKey: "school" },
  // What it is for.
  { id: "rocket", glyph: "🚀", labelKey: "rocket" },
  { id: "flask", glyph: "🧪", labelKey: "flask" },
  { id: "dev", glyph: "⚙️", labelKey: "dev" },
  { id: "build", glyph: "🤖", labelKey: "build" },
  { id: "data", glyph: "📊", labelKey: "data" },
  { id: "storage", glyph: "💾", labelKey: "storage" },
  { id: "media", glyph: "📺", labelKey: "media" },
  { id: "design", glyph: "🎨", labelKey: "design" },
  { id: "ai", glyph: "🧠", labelKey: "ai" },
  { id: "research", glyph: "🔬", labelKey: "research" },
  { id: "secure", glyph: "🛡️", labelKey: "secure" },
  { id: "private", glyph: "🔒", labelKey: "private" },
  { id: "fast", glyph: "⚡", labelKey: "fast" },
  { id: "main", glyph: "⭐", labelKey: "main" },
  { id: "studio", glyph: "🎬", labelKey: "studio" },
  { id: "gaming", glyph: "🎮", labelKey: "gaming" },
  // How it is reached.
  { id: "remote", glyph: "🌐", labelKey: "remote" },
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
