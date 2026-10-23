import { peerIconId } from "@future-os/peer-icons";
import { View } from "react-native";
import { useTheme } from "../theme/ThemeContext";
import { PEER_GLYPHS } from "./peerGlyphs";

/**
 * A machine's mark, drawn from the shared set.
 *
 * A line icon rather than an emoji, because an emoji is drawn by whichever
 * colour-emoji font the device ships: the same machine came out as Noto's
 * picture on Android, Apple's on iOS, and a monochrome fallback wherever the
 * stack found neither — three machines for one label. Stroke, proportions and
 * colour are the app's now, and the desktop draws the same path data, so a
 * machine looks the same in every client that shows it.
 *
 * `size` is in points and `color` defaults to the theme's body ink: the glyph
 * is a mark, not a status, so it takes the palette around it rather than
 * carrying a colour of its own.
 */
export function PeerIconGlyph({
  color,
  id,
  size = 20,
  strokeWidth = 2,
}: {
  color?: string;
  id: string | null | undefined;
  size?: number;
  strokeWidth?: number;
}) {
  const { colors } = useTheme();
  const Glyph = PEER_GLYPHS[peerIconId(id)];
  // Unreachable in the app: the table is typed as a complete record and the id
  // is resolved into that union first. It exists because a test that replaces
  // the icon module with a handful of named stubs leaves the rest of the table
  // undefined, and such a test is about the screen, not the icon set — an empty
  // slot of the right size keeps it from crashing on a glyph it never meant to
  // draw.
  if (!Glyph)
    return <View style={{ width: size, height: size }} />;
  return <Glyph color={color ?? colors.ink} size={size} strokeWidth={strokeWidth} />;
}
