import type { MessageSegment } from "./agentThreadTypes";
import { useEffect, useMemo, useRef, useState } from "react";

/**
 * Smooth "catch-up typewriter" pacing for a streaming assistant bubble.
 *
 * The store still receives the full text in ~220ms polling batches (see
 * sendPipeline / useRunReattach); this hook only decides how much of that text
 * is *shown* on each animation frame, so the reply trickles out instead of
 * landing in visible chunks. It is purely presentational — the message state is
 * never mutated, so copy/settle/reattach all keep working off the full text.
 *
 * Pacing is a backlog-scaled drain, not a constant-speed typewriter: the more
 * text is buffered ahead of the screen, the faster it reveals, so it never lags
 * far behind. A large jump (stream settle, reattach hydration, a huge burst)
 * snaps to the full text immediately, capping worst-case lag around half a
 * second. Non-streaming (settled / historical) messages are passed through
 * untouched — no animation.
 */

// requestAnimationFrame runs ~60fps; these are tuned so a typical reply lags a
// few hundred ms behind the buffer (smooth) while a big backlog drains fast.
const MIN_CHARS_PER_FRAME = 1;
const DRAIN_DIVISOR = 10;
// Backlog (chars) beyond which we stop animating and show everything at once.
const SNAP_THRESHOLD = 400;

/** Total length of the reveal budget: the concatenated text of `text` segments. */
export function textBudget(segments: MessageSegment[]): number {
  let total = 0;
  for (const segment of segments) {
    if (segment.kind === "text")
      total += segment.text.length;
  }
  return total;
}

/**
 * Truncate the segment timeline to `revealed` characters of text budget. Text
 * segments consume the budget in order; a non-text segment (tool activity,
 * thinking, compaction) is a zero-width gate — it appears only once every text
 * segment before it is fully revealed, so the answer "finishes speaking" before
 * the next tool row pops in.
 */
export function sliceSegments(segments: MessageSegment[], revealed: number): MessageSegment[] {
  const out: MessageSegment[] = [];
  let consumed = 0;
  for (const segment of segments) {
    if (segment.kind !== "text") {
      out.push(segment);
      continue;
    }
    const remaining = revealed - consumed;
    if (remaining <= 0)
      break;
    if (remaining >= segment.text.length) {
      out.push(segment);
      consumed += segment.text.length;
      continue;
    }
    out.push({ ...segment, text: segment.text.slice(0, remaining) });
    break;
  }
  return out;
}

export interface PacedReveal {
  /** Paced view of the segment timeline (null when the message has no segments). */
  segments: MessageSegment[] | null;
  /** Paced view of the flat content (used when there are no segments). */
  content: string;
}

/**
 * @param segments  The message's full segment timeline, or null when it renders
 *                  from flat `content`.
 * @param content   The message's full flat content.
 * @param streaming Whether the reply is still streaming; pacing only runs while true.
 */
export function usePacedReveal(
  segments: MessageSegment[] | null,
  content: string,
  streaming: boolean,
): PacedReveal {
  const target = useMemo(
    () => (segments ? textBudget(segments) : content.length),
    [segments, content],
  );

  const [revealed, setRevealed] = useState(() => (streaming ? 0 : target));
  const revealedRef = useRef(revealed);
  revealedRef.current = revealed;

  useEffect(() => {
    // Settled / historical: no animation, show the whole thing.
    if (!streaming) {
      if (revealedRef.current !== target)
        setRevealed(target);
      return;
    }
    // Target shrank (defensive — projection is normally append-only) or a large
    // backlog (settle / hydration / burst): snap to full, don't animate.
    if (revealedRef.current > target || target - revealedRef.current > SNAP_THRESHOLD) {
      setRevealed(target);
      return;
    }
    if (revealedRef.current >= target)
      return; // caught up; wait for the next batch to grow `target`

    let frame = 0;
    const tick = () => {
      const current = revealedRef.current;
      const backlog = target - current;
      if (backlog <= 0)
        return;
      const step = Math.max(MIN_CHARS_PER_FRAME, Math.ceil(backlog / DRAIN_DIVISOR));
      const next = Math.min(target, current + step);
      setRevealed(next);
      if (next < target)
        frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [streaming, target]);

  return useMemo(() => {
    if (!streaming || revealed >= target)
      return { segments, content };
    if (segments)
      return { segments: sliceSegments(segments, revealed), content };
    return { segments, content: content.slice(0, revealed) };
  }, [segments, content, streaming, revealed, target]);
}
