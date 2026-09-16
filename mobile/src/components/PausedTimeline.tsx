import { createContext, memo, type PropsWithChildren } from "react";

/** Presentation only: a file surface can cover the transcript while remote
 * state continues advancing. Consumers use this to stop display-only timers. */
export const TimelineVisibleContext = createContext(true);

type Props = PropsWithChildren<{ paused: boolean }>;
const RetainedChildren = memo(function RetainedChildren({ children }: Props) {
  return <>{children}</>;
}, (_previous, next) => next.paused);

/** Retain native scroll position, expanded rows and composer state; do not
 * unmount the conversation just to stop rendering a covered transcript.
 * Context still propagates through memo, so its timers sleep on the first
 * covered frame. Uncovering always installs the latest children/callbacks. */
export function PausedTimeline({ paused, children }: Props) {
  return <TimelineVisibleContext value={!paused}>
    <RetainedChildren paused={paused}>{children}</RetainedChildren>
  </TimelineVisibleContext>;
}
