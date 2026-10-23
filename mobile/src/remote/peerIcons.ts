/**
 * The peer-icon set, as this app reads it.
 *
 * The list itself — ids, order, labels — lives in `@future-os/peer-icons`,
 * which the desktop reads too; this module re-exports it so call sites keep one
 * import, and keeps nothing else. The drawing is `./PeerIconGlyph`, over the
 * table in `./peerGlyphs`: a module that exports components should export
 * nothing else, and a module of data should not pull the icon package in.
 */
export { DEFAULT_PEER_ICON_ID, PEER_ICONS, peerIconId } from "@future-os/peer-icons";
export type { PeerIcon, PeerIconId } from "@future-os/peer-icons";
