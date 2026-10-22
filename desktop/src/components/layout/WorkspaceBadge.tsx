import { Folder } from "lucide-react";

/**
 * Which workspace a conversation works in, drawn in the conversation's header.
 *
 * The rail says it per heading — a folder and the workspace's own name — but a
 * conversation read from the flat recent list, or from the transcript once the
 * list has scrolled out of sight, had nothing left to say which folder it was
 * in. A conversation on another machine carried the badge and this computer's
 * own did not, so the same conversation read as a workspace member on the host
 * and as nothing at all here. Both headers now draw this one badge, in the
 * rail's vocabulary, so the two read as one fact rather than two.
 *
 * A qualifier, not a title: muted and small, and after the title it qualifies —
 * the treatment [`MachineBadge`](./MachineBadge.tsx) gives the machine beside
 * it.
 */
export function WorkspaceBadge({
  name,
}: {
  /** The workspace's own name, as the rail's heading shows it. */
  name: string;
}) {
  return (
    <span
      className="ml-2 inline-flex min-w-0 shrink-0 items-center gap-1 text-xs font-normal text-ink-muted"
      data-testid="thread-workspace"
      title={name}
    >
      <Folder aria-hidden className="size-3.5 shrink-0" />
      {/* Capped like the machine badge: a long workspace name must not squeeze
          the conversation's own title, which is what the header is for. */}
      <span className="max-w-40 truncate">{name}</span>
    </span>
  );
}
