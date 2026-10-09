import type { RemotePeer } from "./remotePeerClient";
import type { RemoteEntry } from "./useRemoteTimeline";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { MarkdownContent } from "../markdown/MarkdownContent";
import { iconGlyph, peerBadgeText } from "./peerIcons";

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
  composer,
  entries,
  error,
  hasMore,
  loading,
  loadingOlder,
  onLoadOlder,
  onRetry,
  peer,
  streaming,
  title,
}: {
  composer: React.ReactNode;
  entries: RemoteEntry[];
  error: string | null;
  hasMore: boolean;
  loading: boolean;
  loadingOlder: boolean;
  onLoadOlder: () => void;
  onRetry: () => void;
  peer: RemotePeer | undefined;
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
          {entries.map(entry => <EntryRow entry={entry} key={entry.id} />)}
        </div>
      </div>

      {composer}
    </section>
  );
}

function EntryRow({ entry }: { entry: RemoteEntry }) {
  const { t } = useTranslation("remotePeer");
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
    <div className={isUser ? "flex justify-end" : "flex justify-start"}>
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
    </div>
  );
}
