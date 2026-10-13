/**
 * The icons a user can assign to a paired desktop.
 *
 * A fixed set, not free text, and stored by id: the glyph is rendered in place
 * of the desktop's name in the session list, where it has to be same-width and
 * distinguishable from the other desktops' glyphs at a glance. A user-supplied
 * string would fail both — and would be the one piece of free text that could
 * reach another machine if it were ever synced by mistake.
 *
 * Every glyph must be a **single RGI emoji** (`unicode.org/Public/emoji/…
 * /emoji-test.txt`), with `U+FE0F` where Unicode gives the code point a text
 * presentation by default. Both halves matter, and the two ways of getting it
 * wrong are silent:
 *
 *  - A code point that is not an emoji at all (`U+1F5A7`, the old `server`
 *    glyph, is the network-computers pictograph, not an emoji) has no glyph in
 *    any emoji font, so Android drew a tofu box.
 *  - A code point whose default presentation is *text* (`U+1F5A5`, `U+2601`)
 *    without `U+FE0F` falls back to the monochrome text glyph, so the same
 *    desktop looked washed out on the phone and coloured on the desktop.
 *
 * The ids are stable and shared with the desktop app
 * (`desktop/src/features/remote-peer/peerIcons.ts`); the two lists must stay
 * identical, and each side's test reads the other file to prove it.
 */
export interface PeerIcon {
  id: string;
  glyph: string;
  /** i18n key under `desktops.icons.*`. */
  labelKey: string;
}

export const PEER_ICONS: PeerIcon[] = [
  // What kind of machine it is.
  { id: "desktop", glyph: "🖥️", labelKey: "desktop" },
  { id: "laptop", glyph: "💻", labelKey: "laptop" },
  { id: "phone", glyph: "📱", labelKey: "phone" },
  { id: "server", glyph: "🗄️", labelKey: "server" },
  { id: "cloud", glyph: "☁️", labelKey: "cloud" },
  // Where it stands.
  { id: "home", glyph: "🏠", labelKey: "home" },
  { id: "office", glyph: "🏢", labelKey: "office" },
  { id: "cafe", glyph: "☕", labelKey: "cafe" },
  { id: "school", glyph: "🏫", labelKey: "school" },
  { id: "factory", glyph: "🏭", labelKey: "factory" },
  { id: "store", glyph: "🏪", labelKey: "store" },
  // What it is for.
  { id: "rocket", glyph: "🚀", labelKey: "rocket" },
  { id: "flask", glyph: "🧪", labelKey: "flask" },
  { id: "dev", glyph: "⚙️", labelKey: "dev" },
  { id: "build", glyph: "🤖", labelKey: "build" },
  { id: "sandbox", glyph: "📦", labelKey: "sandbox" },
  { id: "data", glyph: "📊", labelKey: "data" },
  { id: "research", glyph: "🔬", labelKey: "research" },
  { id: "secure", glyph: "🛡️", labelKey: "secure" },
  { id: "private", glyph: "🔒", labelKey: "private" },
  { id: "fast", glyph: "⚡", labelKey: "fast" },
  { id: "main", glyph: "⭐", labelKey: "main" },
  { id: "studio", glyph: "🎬", labelKey: "studio" },
  { id: "gaming", glyph: "🎮", labelKey: "gaming" },
  // How it is reached.
  { id: "remote", glyph: "🌐", labelKey: "remote" },
  { id: "edge", glyph: "🛰️", labelKey: "edge" },
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
