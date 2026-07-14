import type { ResolvedMarkdownReference } from "../../../integrations/storage/markdownReferences";
import type { FutureReference } from "../futureMarkdownTypes";
import { useTranslation } from "react-i18next";
import {
  isStoredApproval,
  isStoredArtifact,
  isStoredReview,
  isStoredRun,
} from "../../../integrations/storage/typeGuards";
import { ArtifactEmbed } from "./ArtifactEmbed";
import { renderFileReference } from "./fileReference";
import { MissingReference } from "./MissingReference";
import { ApprovalEmbed, ReviewEmbed } from "./ObjectEmbed";
import { PendingReference } from "./PendingReference";
import { RunEmbed } from "./RunEmbed";

export function FutureEmbed({
  reference,
  resolved,
}: {
  reference: FutureReference;
  resolved?: ResolvedMarkdownReference;
}) {
  const { t } = useTranslation("markdown");

  // `file` renders as a link (never a "missing" badge) — resolution is pure path
  // arithmetic, so a file reference always resolves. See resolve.rs::ResolvedFile.
  const fileLink = renderFileReference(reference, resolved);
  if (fileLink)
    return fileLink;

  // Resolve IPC still in flight — neutral placeholder, not the red badge.
  if (!resolved)
    return <PendingReference reference={reference} />;

  if (resolved.status !== "resolved") {
    return <MissingReference error={resolved.error} reference={reference} />;
  }

  if (reference.targetType === "artifact" && resolved.targetType === "artifact") {
    if (isStoredArtifact(resolved.data)) {
      return <ArtifactEmbed artifact={resolved.data} reference={reference} />;
    }
    return <MissingReference error={t("embed.artifactPayloadInvalid")} reference={reference} />;
  }

  if (reference.targetType === "run" && resolved.targetType === "run") {
    if (isStoredRun(resolved.data)) {
      return <RunEmbed reference={reference} run={resolved.data} />;
    }
    return <MissingReference error={t("embed.runPayloadInvalid")} reference={reference} />;
  }

  if (reference.targetType === "approval" && resolved.targetType === "approval") {
    if (isStoredApproval(resolved.data)) {
      return <ApprovalEmbed approval={resolved.data} reference={reference} />;
    }
    return <MissingReference error={t("embed.approvalPayloadInvalid")} reference={reference} />;
  }

  if (reference.targetType === "review" && resolved.targetType === "review") {
    if (isStoredReview(resolved.data)) {
      return <ReviewEmbed reference={reference} review={resolved.data} />;
    }
    return <MissingReference error={t("embed.reviewPayloadInvalid")} reference={reference} />;
  }

  return <MissingReference error={t("embed.typeMismatch")} reference={reference} />;
}
