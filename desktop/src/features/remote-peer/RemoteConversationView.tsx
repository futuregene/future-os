import type { RemotePeer } from "./remotePeerClient";
import type { RemoteApproval } from "./useRemoteApprovals";
import type { RemoteEntry } from "./useRemoteTimeline";
import { asToolKind } from "@future-os/thread-projection";
import { Archive, FolderOpen, GitBranch } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { Button } from "../../components/ui/Button";
import { IconButton } from "../../components/ui/IconButton";
import { cn } from "../../lib/cn";
import { startWindowDrag } from "../../lib/windowDrag";
import { AgentActivityLine } from "../agent/AgentActivityList";
import { MessageFrame } from "../agent/messages/MessageFrame";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { peerBadgeText } from "./peerIcons";
import { precedingUserEntryId } from "./useRemoteTimeline";
import { useRemoteToolTarget } from "./useRemoteToolTarget";

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
  desktopId,
  composer,
  entries,
  error,
  hasMore,
  leftPanelExpanded,
  loading,
  loadingOlder,
  onCompact,
  onContinueRun,
  onDecideApproval,
  onFork,
  onLoadOlder,
  onOpenFiles,
  onRetry,
  onToggleLeftPanel,
  peer,
  persistedEntryIds,
  streaming,
  title,
  sessionId,
}: {
  approvals: RemoteApproval[];
  approvalErrors: Record<string, string>;
  approvalPending: string | null;
  /** The host this conversation lives on, for reads that address it. */
  desktopId: string;
  /** The host reports a compaction in flight for this conversation. */
  compacting: boolean;
  composer: React.ReactNode;
  entries: RemoteEntry[];
  error: string | null;
  hasMore: boolean;
  /** The panel toggle, as on this app's own conversation header. */
  leftPanelExpanded: boolean;
  loading: boolean;
  loadingOlder: boolean;
  /** Ask the host to compact this conversation's context. */
  onCompact: () => void;
  onDecideApproval: (approval: RemoteApproval, decision: "allow" | "deny") => void;
  /**
   * Resume a failed run, on the host that stopped it.
   *
   * Offered only where `run.error` is set, because that is the host's own
   * precondition: it refuses to continue anything that did not fail.
   */
  onContinueRun: (runId: string) => void;
  /** Open the browser for the files this conversation's host holds for it. */
  onOpenFiles: () => void;
  /**
   * Branch the conversation at the turn that produced an assistant reply, and
   * open the child. `forkable` is false when that turn is not persisted on the
   * host yet, which the caller must report rather than send.
   */
  onFork: (sourceEntryId: string, forkable: boolean) => void;
  onLoadOlder: () => void;
  onRetry: () => void;
  onToggleLeftPanel: () => void;
  peer: RemotePeer | undefined;
  /** The conversation's session id on that host. */
  sessionId: string;
  /** The entry ids the host will accept as a stored record (see the timeline). */
  persistedEntryIds: ReadonlySet<string>;
  streaming: boolean;
  title: string;
}) {
  const { t } = useTranslation("remotePeer");

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      {/* Same header row as this app's own conversation (`ThreadHeader`): the
          panel toggle and the title at the left, the conversation's own actions
          at the right, same height and padding — a remote conversation is a
          conversation, and its frame should not announce otherwise.
          `pr-14` reserves the collapsed context panel's corner, as there. */}
      <header
        className="flex h-12 shrink-0 select-none items-center gap-2 border-b border-line-soft/40 pl-4 pr-14"
        onMouseDown={startWindowDrag}
      >
        <div className="mr-3 flex min-w-0 flex-1 items-center" data-tauri-drag-region>
          <LeftPanelTitlebarToggle expanded={leftPanelExpanded} onToggle={onToggleLeftPanel} />
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold text-ink">{title}</div>
          </div>
        </div>
        {/* The conversation's own actions, as icon buttons: this header is the
            same row as this app's own (`ThreadHeader`), which carries a
            conversation's controls as 32px icon buttons with their labels in the
            tooltip. Text buttons here read as a toolbar bolted onto the title.
            The machine this conversation runs on is named by the composer's
            control row — beside the button that sends to it — so it does not
            take a second line under the title, which no local header has. */}
        <IconButton
          icon={<FolderOpen className="size-4" />}
          label={t("filesTitle")}
          onClick={onOpenFiles}
        />
        {/* Compact is a conversation-level action on the host, next to the run
            state it is mutually exclusive with: a host answering a prompt does
            not also compact, and a host compacting refuses a prompt. */}
        {compacting
          ? <span className="shrink-0 px-1 text-xs text-ink-muted">{t("compacting")}</span>
          : (
              <IconButton
                disabled={streaming || loading || error !== null || entries.length === 0}
                icon={<Archive className="size-4" />}
                label={t("compact")}
                onClick={onCompact}
              />
            )}
        {streaming
          ? <span className="shrink-0 px-1 text-xs text-accent">{t("statusRunning")}</span>
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
                desktopId={desktopId}
                entry={entry}
                forkable={forkSource !== null && persistedEntryIds.has(forkSource)}
                forkSource={forkSource}
                key={entry.id}
                onContinueRun={onContinueRun}
                onFork={onFork}
                sessionId={sessionId}
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
/**
 * What one tool call did, as an activity line.
 *
 * Rendered through the same `AgentActivityLine` this app's own transcript uses,
 * with an `AgentActivityItem` built from the tool call: the icon, the label
 * ("Ran a command", "Read a file"), the target and the expandable detail are the
 * local row's, not a second rendering of the same idea. What the reader compares
 * when they switch between a local and a remote conversation is whether the tool
 * rows look like tool rows — so the local ones are the ones used.
 *
 * One component per call rather than one per turn, because the arguments are
 * fetched per call and a hook cannot be called in a loop: a turn that ran three
 * tools has three targets to fetch, and rendering only the first hid the rest of
 * what the machine did.
 *
 * The fetch is skipped when the call cannot be addressed (no call id, no run) —
 * a request the host would have to refuse.
 */
function RemoteToolRow({
  desktopId,
  name,
  runId,
  sessionId,
  toolCallId,
}: {
  desktopId: string;
  name: string;
  runId: string;
  sessionId: string;
  toolCallId: string;
}) {
  // A tool row keeps its identity and can lose its arguments: a lean history
  // page strips a shell call's command, keeping the call id and the run to fetch
  // it back. Asked for on mount and cached in the module, because a transcript
  // re-renders on every streaming push.
  const target = useRemoteToolTarget({
    desktopId,
    enabled: toolCallId !== "" && runId !== "",
    runId,
    sessionId,
    toolCallId,
  });
  const kind = asToolKind(name);
  return (
    <AgentActivityLine
      item={{
        id: toolCallId !== "" ? toolCallId : `${name}-${runId}`,
        kind,
        status: "completed",
        // What it acted on: the command for a shell call, else the path. The
        // lean feed strips a shell call's arguments, which is why they are
        // fetched rather than read off the block.
        target: target ?? undefined,
      }}
      // No `runId`: the local inspect action addresses a run in *this* app's
      // store, and this run lives on the host.
    />
  );
}

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
  desktopId,
  entry,
  forkable,
  forkSource,
  onContinueRun,
  onFork,
  sessionId,
  streaming,
}: {
  desktopId: string;
  entry: RemoteEntry;
  /** False when the turn this reply rests on is not persisted on the host yet. */
  forkable: boolean;
  forkSource: string | null;
  onContinueRun: (runId: string) => void;
  onFork: (sourceEntryId: string, forkable: boolean) => void;
  sessionId: string;
  streaming: boolean;
}) {
  const { t } = useTranslation("remotePeer");
  const [hovered, setHovered] = useState(false);
  const isUser = entry.role === "user";
  // Every tool call of the turn, not the first one. A turn routinely runs
  // several (`read` then `shell`), and rendering only the first hid the rest of
  // what the machine actually did — found by rendering a real transcript, where
  // one assistant turn makes two calls.
  //
  // `MessageBlock` (the projection package's own shape) names the discriminant
  // `kind`, so a tool block is `kind: "tool_call"` with a `name`.
  const tools = entry.blocks.filter(block => block.kind === "tool_call" || block.kind === "tool");
  const text = entry.blocks
    .filter(block => block.kind === "text" && typeof block.text === "string")
    .map(block => block.text as string)
    .join("")
    .trim();
  const runError = entry.run?.error ?? null;

  return (
    /* The same frame this app's own transcript draws a message in
       (`MessageFrame`): an author line above the text, ruled off on the
       assistant's side, the user's own message in a right-aligned bubble. The
       two views had different frames — an accent-coloured bubble for the user
       and a bordered card for the assistant, neither of them anything this app
       draws — which is what made switching between them read as two products
       rather than one conversation on two machines. */
    <MessageFrame
      author={t(`author.${isUser ? "you" : "researchCopilot"}`, { ns: "agent" })}
      createdAt={new Date(entry.createdAtMs).toISOString()}
      dataMessageId={entry.id}
      onEnter={() => setHovered(true)}
      onLeave={() => setHovered(false)}
      role={isUser ? "user" : "assistant"}
    >
      {tools.map(tool => (
        <RemoteToolRow
          desktopId={desktopId}
          key={toolKey(tool, tools)}
          name={tool.name ?? ""}
          runId={typeof entry.runId === "string" ? entry.runId : ""}
          sessionId={sessionId}
          toolCallId={typeof tool.toolCallId === "string" ? tool.toolCallId : ""}
        />
      ))}
      {text
        ? (isUser
            ? <div className="whitespace-pre-wrap wrap-break-word">{text}</div>
            : <MarkdownContent content={text} />)
        : null}
      {runError
        ? (
            <div className="mt-2 flex items-center gap-2 text-xs text-danger">
              <span className="min-w-0 wrap-break-word">{runError}</span>
              {/* Only when the host named the run: a continuation is addressed
                  by it, and one it cannot address would be refused. */}
              {entry.runId && !streaming
                ? (
                    <button
                      className="shrink-0 rounded px-1.5 py-0.5 font-medium text-accent hover:bg-accent-soft"
                      onClick={() => onContinueRun(entry.runId!)}
                      type="button"
                    >
                      {t("continueRun")}
                    </button>
                  )
                : null}
            </div>
          )
        : null}
      {/* The fork control sits in the message's own footer, as it does in this
          app's transcript: beside the message it belongs to, inside the reading
          column. Placing it *next to* the frame instead took 26px off the width
          of every message that had one — measured, and the reason the last
          message on this page was narrower than the same message locally.
          Hidden while a run is in flight, like the local one: the turn being
          forked is not settled yet, so its persisted identity may not exist. */}
      {forkSource && !streaming
        ? (
            <div className="mt-3 flex justify-start">
              <button
                aria-label={t("fork")}
                className={cn(
                  "rounded p-1 text-ink-muted transition-opacity duration-200 hover:text-ink focus-visible:opacity-100",
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
            </div>
          )
        : null}
    </MessageFrame>
  );
}

/**
 * A stable key for one tool call in a turn.
 *
 * The host's call id when it gave one; otherwise the tool's own name plus its
 * place in the turn, which is stable because a turn's blocks do not reorder.
 */
function toolKey(
  tool: RemoteEntry["blocks"][number],
  all: RemoteEntry["blocks"],
): string {
  return typeof tool.toolCallId === "string" && tool.toolCallId !== ""
    ? tool.toolCallId
    : `${tool.name ?? "tool"}-${all.indexOf(tool)}`;
}
