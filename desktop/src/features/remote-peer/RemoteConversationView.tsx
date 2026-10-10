import type { RemotePeer } from "./remotePeerClient";
import type { RemoteApproval } from "./useRemoteApprovals";
import type { RemoteEntry } from "./useRemoteTimeline";
import { GitBranch } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { cn } from "../../lib/cn";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import { precedingUserEntryId } from "./useRemoteTimeline";

/**
 * A remote conversation: the session's transcript plus a composer that sends to
 * the host.
 *
 * Rendered with the app's own Markdown component rather than the local message
 * list. The local list is built around a *local* message model — run records,
 * approvals, artifacts, review snapshots — none of which exist for a remote
 * session on this machine. Reusing it would mean fabricating local records for
 * them, which is exactly the second-copy-of-truth this feature must avoid.
 */
export function RemoteConversationView({
  approvals,
  approvalErrors,
  approvalPending,
  compacting,
  composer,
  entries,
  error,
  hasMore,
  loading,
  loadingOlder,
  onCompact,
  onDecideApproval,
  onFork,
  onLoadOlder,
  onOpenFiles,
  onRetry,
  peer,
  persistedEntryIds,
  settings,
  streaming,
  title,
}: {
  approvals: RemoteApproval[];
  approvalErrors: Record<string, string>;
  approvalPending: string | null;
  /** The host reports a compaction in flight for this conversation. */
  compacting: boolean;
  composer: React.ReactNode;
  entries: RemoteEntry[];
  error: string | null;
  hasMore: boolean;
  loading: boolean;
  loadingOlder: boolean;
  /** Ask the host to compact this conversation's context. */
  onCompact: () => void;
  onDecideApproval: (approval: RemoteApproval, decision: "allow" | "deny") => void;
  /** Open the browser for the files this conversation's host holds for it. */
  onOpenFiles: () => void;
  /** The conversation's host-owned model and thinking level, if the caller has them. */
  settings?: React.ReactNode;
  /**
   * Branch the conversation at the turn that produced an assistant reply, and
   * open the child. `forkable` is false when that turn is not persisted on the
   * host yet, which the caller must report rather than send.
   */
  onFork: (sourceEntryId: string, forkable: boolean) => void;
  onLoadOlder: () => void;
  onRetry: () => void;
  peer: RemotePeer | undefined;
  /** The entry ids the host will accept as a stored record (see the timeline). */
  persistedEntryIds: ReadonlySet<string>;
  streaming: boolean;
  title: string;
}) {
  const { t } = useTranslation("remotePeer");

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header className="flex h-14 shrink-0 items-center gap-3 border-b border-line-soft px-4">
        <span aria-hidden className="text-base">{iconGlyph(peer?.icon)}</span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-ink">{title}</div>
          <div className="truncate text-xs text-ink-muted">
            {t("runsOn", { name: peer ? peerBadgeText(peer, peer.desktopId) : "" })}
          </div>
        </div>
        {/* The conversation's own settings, beside the controls for it: these act
            on the host and the picker says so through their labels. */}
        {settings}
        <Button onClick={onOpenFiles} size="sm" variant="ghost">
          {t("filesTitle")}
        </Button>
        {/* Compact is a conversation-level action on the host, next to the run
            state it is mutually exclusive with: a host answering a prompt does
            not also compact, and a host compacting refuses a prompt. */}
        {compacting
          ? <span className="shrink-0 text-xs text-ink-muted">{t("compacting")}</span>
          : (
              <Button
                disabled={streaming || loading || error !== null || entries.length === 0}
                onClick={onCompact}
                size="sm"
                variant="ghost"
              >
                {t("compact")}
              </Button>
            )}
        {streaming
          ? <span className="shrink-0 text-xs text-accent">{t("statusRunning")}</span>
          : null}
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto px-4 py-4">
        {hasMore
          ? (
              <div className="mb-4 flex justify-center">
                <Button disabled={loadingOlder} onClick={onLoadOlder} size="sm" variant="ghost">
                  {loadingOlder ? t("loadingOlder") : t("loadOlder")}
                </Button>
              </div>
            )
          : null}

        {loading
          ? <p className="text-center text-xs text-ink-muted">{t("loadingHistory")}</p>
          : null}

        {error
          ? (
              <div className="mx-auto max-w-md rounded-md border border-danger-line bg-danger-soft px-3 py-2 text-sm text-danger">
                <div>{error}</div>
                <Button className="mt-2" onClick={onRetry} size="sm" variant="ghost">{t("retry")}</Button>
              </div>
            )
          : null}

        {!loading && !error && entries.length === 0
          ? <p className="text-center text-xs text-ink-muted">{t("emptyConversation")}</p>
          : null}

        <div className="mx-auto w-full max-w-3xl space-y-4">
          {entries.map((entry) => {
            // Computed here, where the whole list is in scope: the fork point is
            // a property of the entry's *position* in the conversation.
            const forkSource = entry.role === "user" ? null : precedingUserEntryId(entries, entry.id);
            return (
              <EntryRow
                entry={entry}
                forkable={forkSource !== null && persistedEntryIds.has(forkSource)}
                forkSource={forkSource}
                key={entry.id}
                onFork={onFork}
                streaming={streaming}
              />
            );
          })}
        </div>
      </div>

      {/* Approvals sit above the composer, not inside the transcript: they are a
          decision the user has to make now, and a card that scrolls away with
          history is a card that gets missed while the host waits. */}
      {approvals.length > 0
        ? (
            <div className="shrink-0 space-y-2 border-t border-line-soft bg-surface-subtle px-4 py-3">
              {approvals.map(approval => (
                <ApprovalCard
                  approval={approval}
                  error={approvalErrors[approval.id] ?? null}
                  key={approval.id}
                  onDecide={onDecideApproval}
                  pending={approvalPending === approval.id}
                  peerName={peer ? peerBadgeText(peer, peer.desktopId) : ""}
                />
              ))}
            </div>
          )
        : null}

      {composer}
    </section>
  );
}

/**
 * A pending approval on the host.
 *
 * The card states where the tool will run, because the decision is easy to make
 * wrongly: "allow" on a card that does not name the machine reads as a local
 * action, and the whole point of this feature is that it is not.
 */
function ApprovalCard({
  approval,
  error,
  onDecide,
  pending,
  peerName,
}: {
  approval: RemoteApproval;
  error: string | null;
  onDecide: (approval: RemoteApproval, decision: "allow" | "deny") => void;
  pending: boolean;
  peerName: string;
}) {
  const { t } = useTranslation("remotePeer");
  return (
    <div className="mx-auto w-full max-w-3xl rounded-lg border border-line-soft bg-surface p-3">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-ink">
            {approval.title || approval.toolName || t("approvalFallbackTitle")}
          </div>
          {approval.summary
            ? <div className="mt-0.5 wrap-break-word text-xs text-ink-muted">{approval.summary}</div>
            : null}
          <div className="mt-1 text-xs text-ink-muted">
            {t("runsOn", { name: peerName })}
            {approval.riskLevel ? ` · ${approval.riskLevel}` : ""}
          </div>
          {error ? <div className="mt-1 text-xs text-danger">{error}</div> : null}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <Button disabled={pending} onClick={() => onDecide(approval, "deny")} size="sm" variant="ghost">
            {t("deny")}
          </Button>
          <Button disabled={pending} onClick={() => onDecide(approval, "allow")} size="sm" variant="primary">
            {t("allow")}
          </Button>
        </div>
      </div>
    </div>
  );
}

function EntryRow({
  entry,
  forkable,
  forkSource,
  onFork,
  streaming,
}: {
  entry: RemoteEntry;
  /** False when the turn this reply rests on is not persisted on the host yet. */
  forkable: boolean;
  forkSource: string | null;
  onFork: (sourceEntryId: string, forkable: boolean) => void;
  streaming: boolean;
}) {
  const { t } = useTranslation("remotePeer");
  const [hovered, setHovered] = useState(false);
  const isUser = entry.role === "user";
  // `MessageBlock` (the projection package's own shape) names the discriminant
  // `kind`, so a tool block is `kind: "tool_call"` with a `name`.
  const tool = entry.blocks.find(block => block.kind === "tool_call" || block.kind === "tool");
  const text = entry.blocks
    .filter(block => block.kind === "text" && typeof block.text === "string")
    .map(block => block.text as string)
    .join("")
    .trim();
  const runError = entry.run?.error ?? null;

  return (
    <div
      className={isUser ? "flex justify-end" : "flex justify-start"}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div
        className={
          isUser
            ? "max-w-[85%] rounded-lg bg-accent px-3 py-2 text-sm text-white"
            : "max-w-[85%] rounded-lg border border-line-soft bg-surface-subtle px-3 py-2"
        }
      >
        {tool
          ? (
              <div className="mb-1 text-xs text-ink-muted">
                {t("toolRunning", { name: tool.name ?? "" })}
              </div>
            )
          : null}
        {text
          ? (isUser
              ? <div className="whitespace-pre-wrap wrap-break-word">{text}</div>
              : <MarkdownContent content={text} />)
          : null}
        {runError
          ? <div className="mt-1 text-xs text-danger">{runError}</div>
          : null}
        <div className={isUser ? "mt-1 text-[10px] text-white/70" : "mt-1 text-[10px] text-ink-muted"}>
          {new Date(entry.createdAtMs).toLocaleString()}
        </div>
      </div>
      {/* Hidden while a run is in flight, like the local Fork button: the turn
          being forked is not settled yet, so its persisted identity may not
          exist. */}
      {forkSource && !streaming
        ? (
            <button
              aria-label={t("fork")}
              className={cn(
                "ml-1 self-start rounded p-1 text-ink-muted transition-opacity duration-200 hover:text-ink focus-visible:opacity-100",
                // Still reachable by keyboard: a control that only exists on
                // hover is unreachable without a pointer.
                hovered ? "opacity-100" : "pointer-events-none opacity-0",
              )}
              onClick={() => onFork(forkSource, forkable)}
              title={t("fork")}
              type="button"
            >
              <GitBranch className="size-3.5" />
            </button>
          )
        : null}
    </div>
  );
}
