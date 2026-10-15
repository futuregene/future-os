import type { ReactNode } from "react";

/**
 * A `#` reference to another conversation, wherever markdown carries one.
 *
 * The composer writes a picked conversation as `[Title](futureos://session/<id>)`
 * — a plain markdown link, so the model receives the id as ordinary text (see
 * `@future-os/markdown`) — which means every transcript that renders the prompt
 * back sees a link node, not a special one. Handed to `SafeLink` it would print
 * the bare title in inert grey: the `#` marker that tells it apart from a file
 * mention at a glance would be gone, and with it the signal that this names a
 * conversation rather than a document.
 *
 * The chip restores the marker the composer shows and keeps the reference in the
 * accent colour, so a prompt reads in the transcript the way it was written.
 */
export function SessionReferenceChip({ children, href }: { children: ReactNode; href: string }) {
  return (
    <span className="font-medium text-accent" title={href}>
      #
      {children}
    </span>
  );
}
