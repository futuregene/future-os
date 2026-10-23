import { peerIconId, peerIconTint } from "@future-os/peer-icons";
import { View } from "react-native";
import { PEER_GLYPHS } from "./peerGlyphs";

/**
 * A machine's mark, drawn from the shared set.
 *
 * A line icon rather than an emoji, because an emoji is drawn by whichever
 * colour-emoji font the device ships: the same machine came out as Noto's
 * picture on Android, Apple's on iOS, and a monochrome fallback wherever the
 * stack found neither — three machines for one label. Stroke, proportions and
 * colour are the app's now, and the desktop draws the same path data in the same
 * hue, so a machine looks the same in every client that shows it.
 *
 * `size` is in points. The colour comes from the shared palette rather than the
 * theme: the point of it is that this machine's mark is the same colour on both
 * clients, and a theme-dependent hue would break that the moment one of them
 * was in dark mode. `tone` overrides it for the rare surface that has its own
 * reason (the picker's selected tile), not as a per-screen choice.
 */
export function PeerIconGlyph({
  id,
  size = 20,
  strokeWidth = 2,
  tone,
}: {
  id: string | null | undefined;
  size?: number;
  strokeWidth?: number;
  /** Override the icon's own colour; omit for the shared palette's. */
  tone?: string;
}) {
  const Glyph = PEER_GLYPHS[peerIconId(id)];
  const color = tone ?? peerIconTint(id).color;
  // Unreachable in the app: the table is typed as a complete record and the id
  // is resolved into that union first. It exists because a test that replaces
  // the icon module with a handful of named stubs leaves the rest of the table
  // undefined, and such a test is about the screen, not the icon set — an empty
  // slot of the right size keeps it from crashing on a glyph it never meant to
  // draw.
  if (!Glyph)
    return <View style={{ width: size, height: size }} />;
  return <Glyph color={color} size={size} strokeWidth={strokeWidth} />;
}
