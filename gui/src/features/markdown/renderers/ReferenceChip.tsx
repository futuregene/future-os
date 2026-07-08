import type { ResolvedMarkdownReference } from "../../../integrations/storage/markdownReferences";
import type { FutureReference } from "../futureMarkdownTypes";
import { AlertTriangle, Beaker, Box, FileDiff, Microscope, PlayCircle } from "lucide-react";
import { MissingReference } from "./MissingReference";
import { PendingReference } from "./PendingReference";

export function ReferenceChip({
  reference,
  resolved,
}: {
  reference: FutureReference;
  resolved?: ResolvedMarkdownReference;
}) {
  // Resolve IPC still in flight — neutral placeholder, not the red badge.
  if (!resolved)
    return <PendingReference reference={reference} />;

  if (resolved.status !== "resolved") {
    return <MissingReference error={resolved.error} reference={reference} />;
  }

  const label = reference.label ?? reference.targetId;
  return (
    <span
      className="inline-flex max-w-full items-center gap-1 rounded-md border border-line-soft bg-surface-subtle px-1.5 py-0.5 text-[0.92em] font-medium text-ink-soft"
      title={`${reference.targetType}:${reference.targetId}`}
    >
      {renderReferenceIcon(reference.targetType)}
      <span className="truncate">{label}</span>
    </span>
  );
}

function renderReferenceIcon(targetType: FutureReference["targetType"]) {
  const className = "size-3 shrink-0";
  switch (targetType) {
    case "approval":
      return <AlertTriangle className={className} />;
    case "research":
      return <Microscope className={className} />;
    case "review":
      return <FileDiff className={className} />;
    case "run":
      return <PlayCircle className={className} />;
    case "tool":
      return <Beaker className={className} />;
    default:
      return <Box className={className} />;
  }
}
