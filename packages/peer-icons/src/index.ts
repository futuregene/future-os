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
 * the app's, not the system's — the colour too, which is why the palette below
 * is here and not in either client.
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

/**
 * The colour each entry is drawn in, and the wash behind it.
 *
 * Here rather than in either app, for the same reason the ids are: a machine's
 * mark is the app's picture, so its colour has to be the same picture on a Mac
 * and on a phone. A hue chosen per client would put the rocket orange in one
 * list and red in the other — the mismatch the set was unified to remove,
 * arriving through the palette instead of through the emoji font.
 *
 * Mid-tone hues, chosen to stay legible on both the light `#ffffff` surface and
 * the dark `#1d232c` one, since neither client swaps this palette per theme.
 * Reading *which* icon it is never depends on the colour alone — the shape
 * carries that — so this is decoration with a job: making twenty marks in a
 * grid as quick to tell apart as the emoji they replaced.
 */
const PEER_ICON_COLORS: Record<PeerIconId, string> = {
  desktop: "#2563eb",
  laptop: "#0891b2",
  phone: "#16a34a",
  server: "#64748b",
  cloud: "#0ea5e9",
  windows: "#0284c7",
  macos: "#64748b",
  linux: "#d97706",
  home: "#059669",
  office: "#7c3aed",
  cafe: "#a16207",
  school: "#4f46e5",
  rocket: "#ea580c",
  flask: "#0d9488",
  dev: "#64748b",
  build: "#6366f1",
  data: "#2563eb",
  storage: "#7c3aed",
  media: "#db2777",
  design: "#e11d48",
  ai: "#8b5cf6",
  research: "#0d9488",
  secure: "#059669",
  private: "#b45309",
  fast: "#ca8a04",
  main: "#d97706",
  studio: "#9333ea",
  gaming: "#dc2626",
  remote: "#0284c7",
};

/** How a mark is painted: the ink of the glyph, and the wash behind it. */
export interface PeerIconTint {
  /** The glyph's own stroke colour, as `#rrggbb`. */
  color: string;
  /**
   * The same hue at low alpha, for the chip a mark sits on — `rgba(…)`, which
   * both a CSS background and a React Native style accept, and which therefore
   * tints whatever surface it lands on in either theme.
   */
  soft: string;
}

/** The chip wash's opacity: visible at 36px without competing with the glyph. */
const SOFT_ALPHA = 0.14;

function tintOf(hex: string): PeerIconTint {
  const value = Number.parseInt(hex.slice(1), 16);
  const red = (value >> 16) & 0xff;
  const green = (value >> 8) & 0xff;
  const blue = value & 0xff;
  return { color: hex, soft: `rgba(${red}, ${green}, ${blue}, ${SOFT_ALPHA})` };
}

const PEER_ICON_TINTS = Object.fromEntries(
  PEER_ICONS.map(icon => [icon.id, tintOf(PEER_ICON_COLORS[icon.id])]),
) as Record<PeerIconId, PeerIconTint>;

/**
 * The paint for a stored label, resolved the same way its glyph is.
 *
 * An unknown or empty id gets the default icon's tint, so a hand-edited label
 * draws the same default picture in the same colour rather than a bare glyph on
 * an untinted chip.
 */
export function peerIconTint(stored: string | null | undefined): PeerIconTint {
  return PEER_ICON_TINTS[peerIconId(stored)];
}
