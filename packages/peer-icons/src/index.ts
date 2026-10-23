/**
 * The icons a user can assign to a paired machine — one list, both apps.
 *
 * Ids, order and labels live here and nowhere else. What is *not* here is the
 * drawing: each app maps an id to a glyph from the icon set it already draws
 * with (`lucide-react` on the desktop, `lucide-react-native` on the phone).
 *
 * That split is the point. The set used to be emoji, and an emoji is whatever
 * the platform's colour-emoji font says it is: Apple's 🖥️ on a Mac, Noto's on
 * Android, Segoe's on Windows, and a monochrome text glyph wherever the font
 * stack fell through to one. The same machine therefore looked like three
 * different machines depending on what you were holding, and the picker
 * promised a picture it could not keep. A line icon drawn from the same path
 * data on every platform ends that: stroke width, proportions and colour are
 * the app's, not the system's.
 *
 * The id (not the drawing) is what a label stores, so the set can be redrawn,
 * reordered or translated later without rewriting saved labels — and an id
 * this build does not know renders the default rather than arbitrary stored
 * text.
 */
export const PEER_ICONS = [
  // What kind of machine it is.
  { id: "desktop", labelKey: "desktop" },
  { id: "laptop", labelKey: "laptop" },
  { id: "phone", labelKey: "phone" },
  { id: "server", labelKey: "server" },
  { id: "cloud", labelKey: "cloud" },
  // Which OS it runs.
  { id: "windows", labelKey: "windows" },
  { id: "macos", labelKey: "macos" },
  { id: "linux", labelKey: "linux" },
  // Where it stands.
  { id: "home", labelKey: "home" },
  { id: "office", labelKey: "office" },
  { id: "cafe", labelKey: "cafe" },
  { id: "school", labelKey: "school" },
  // What it is for.
  { id: "rocket", labelKey: "rocket" },
  { id: "flask", labelKey: "flask" },
  { id: "dev", labelKey: "dev" },
  { id: "build", labelKey: "build" },
  { id: "data", labelKey: "data" },
  { id: "storage", labelKey: "storage" },
  { id: "media", labelKey: "media" },
  { id: "design", labelKey: "design" },
  { id: "ai", labelKey: "ai" },
  { id: "research", labelKey: "research" },
  { id: "secure", labelKey: "secure" },
  { id: "private", labelKey: "private" },
  { id: "fast", labelKey: "fast" },
  { id: "main", labelKey: "main" },
  { id: "studio", labelKey: "studio" },
  { id: "gaming", labelKey: "gaming" },
  // How it is reached.
  { id: "remote", labelKey: "remote" },
] as const;

/** One entry of the set. */
export type PeerIcon = (typeof PEER_ICONS)[number];

/** The id of every entry, as a string union. */
export type PeerIconId = PeerIcon["id"];

/**
 * What an unset or unknown label draws.
 *
 * The first entry, and a real glyph rather than a blank: this id is drawn in
 * place of a machine's name in a list, so a blank would read as a broken row.
 */
export const DEFAULT_PEER_ICON_ID: PeerIconId = "desktop";

/**
 * The id to draw for a stored label.
 *
 * `stored` comes from a label file some other build may have written, so it is
 * checked against the set rather than trusted — an unknown value must never
 * reach the screen as itself.
 */
export function peerIconId(stored: string | null | undefined): PeerIconId {
  if (!stored)
    return DEFAULT_PEER_ICON_ID;
  return PEER_ICONS.find(icon => icon.id === stored)?.id ?? DEFAULT_PEER_ICON_ID;
}
