import { peerIconTint } from "@future-os/peer-icons";
import { View, type StyleProp, type ViewStyle } from "react-native";
import { radius } from "../theme/tokens";
import { PeerIconGlyph } from "./PeerIconGlyph";

/**
 * A paired machine's icon, on a coloured chip.
 *
 * The chip is the mark's own hue at low alpha, with the glyph in that hue on
 * top — the phone's version of the desktop's `PeerIconChip`, and the same
 * numbers so a machine is drawn the same on both: 36px, 12px radius, a 20px
 * glyph. A flat grey box said "some machine"; the colour is what the emoji the
 * set replaced used to give for free, and it is how twenty rows stay
 * distinguishable at a glance.
 *
 * `size` is the box, `glyph` the mark inside it: the list row is 36×44 (a card
 * beside two lines of text), a page header 36×36.
 */
export function PeerIconChip({
  glyph = 20,
  id,
  size,
  style,
}: {
  /** Glyph size in points. */
  glyph?: number;
  id: string | null | undefined;
  /** Box size; a pair `{ width, height }` for the row's taller card. */
  size: number | { height: number; width: number };
  style?: StyleProp<ViewStyle>;
}) {
  const box = typeof size === "number" ? { height: size, width: size } : size;
  return (
    <View
      style={[
        { ...box, alignItems: "center", justifyContent: "center" },
        { borderRadius: radius.md, backgroundColor: peerIconTint(id).soft },
        style,
      ]}
    >
      <PeerIconGlyph id={id} size={glyph} />
    </View>
  );
}
