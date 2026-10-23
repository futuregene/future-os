import { peerIconId, peerIconTint } from "@future-os/peer-icons";
import { cn } from "../../lib/cn";
import { PEER_GLYPHS } from "./peerGlyphs";

/**
 * A machine's mark, drawn from the shared set.
 *
 * A line icon rather than an emoji, because an emoji is drawn by whichever
 * colour-emoji font the platform ships: the same machine came out as Apple's
 * picture on a Mac, Noto's on Android, and a monochrome fallback wherever the
 * stack found neither — three machines for one label. Stroke, proportions and
 * colour are the app's now, and the phone draws the same path data in the same
 * hue, so a machine looks the same in every client that shows it.
 *
 * `tone` is the exception, and it is deliberately narrow: a chip or a row that
 * has a reason to override the paint (the picker's selected tile wants its own
 * emphasis) can, and everything else takes the icon's own colour from the
 * shared palette. It is not a place to invent a colour per screen.
 *
 * `className` carries the size (`size-4`, `size-5`), and it is the only size
 * input: one scale keeps the set proportionate across a 12px list row and a
 * 20px picker tile.
 */
export function PeerIconGlyph({
  className,
  id,
  strokeWidth = 2,
  tone,
}: {
  className?: string;
  id: string | null | undefined;
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
  // span keeps it from crashing on a glyph it never meant to draw.
  if (!Glyph)
    return <span aria-hidden className={cn("shrink-0", className)} style={{ color }} />;
  return <Glyph aria-hidden className={cn("shrink-0", className)} color={color} strokeWidth={strokeWidth} />;
}
