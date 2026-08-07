import type { FutureReference } from "../futureMarkdownTypes";

/**
 * Neutral placeholder for a reference whose resolve IPC is still in flight
 * (`resolved === undefined`). Distinct from `MissingReference`'s red badge,
 * which is reserved for a genuinely missing / failed / type-mismatched target —
 * otherwise every non-file chip would flash red on first paint. Mirrors the
 * pending treatment file references get (see `renderFileReference`).
 */
export function PendingReference({ reference }: { reference: FutureReference }) {
  return <span className="text-ink-soft">{reference.label ?? reference.targetId}</span>;
}
