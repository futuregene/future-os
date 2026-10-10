import type { RemotePeer } from "./remotePeerClient";
import { ChevronLeft } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { TasksView } from "../tasks/TasksView";
import { iconGlyph, peerBadgeText } from "./peerIcons";
import { fetchRemoteSessions, listRemoteModels, remoteAgentModelOptions } from "./remotePeerClient";
import { remoteTaskBackend } from "./remoteTaskBackend";

/**
 * Another computer's tasks, edited with this app's own task editor.
 *
 * It replaces the whole column rather than sitting inside that computer's
 * settings page: the editor is a list-and-detail pair that needs the height, and
 * burying it in a scrolling settings pane would give it two scrollbars and no
 * room for either half.
 *
 * The editor is literally the local one (`TasksView`) pointed at the host
 * through a backend, so a fix to the form, the dependency editor or the
 * revision list lands on both — which is the point of pointing it rather than
 * writing a second one.
 */
export function RemoteTasksView({
  leftPanelExpanded,
  onBack,
  onOpenSession,
  onToggleLeftPanel,
  peer,
}: {
  leftPanelExpanded: boolean;
  onBack: () => void;
  /** Open one of that host's conversations, for a run's "open" action. */
  onOpenSession: (desktopId: string, sessionId: string) => void;
  onToggleLeftPanel: () => void;
  peer: RemotePeer;
}) {
  const { t } = useTranslation("remotePeer");
  const [models, setModels] = useState<ReturnType<typeof remoteAgentModelOptions>>([]);
  const [error, setError] = useState<string | null>(null);

  const backend = remoteTaskBackend(peer.desktopId);

  const loadModels = useCallback(async () => {
    setError(null);
    try {
      setModels(remoteAgentModelOptions(await listRemoteModels(peer.desktopId)));
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }, [peer.desktopId]);

  useEffect(() => {
    void loadModels();
  }, [loadModels]);

  /**
   * A run names the conversation by thread id; opening one needs the host's
   * session id.
   *
   * The two are different strings there, and guessing between them opens the
   * wrong conversation or none — so the host's own catalogue is asked which
   * session has that thread.
   */
  const openThread = useCallback(async (threadId: string) => {
    setError(null);
    try {
      const catalog = await fetchRemoteSessions(peer.desktopId);
      const session = catalog.sessions.find(row => row.threadId === threadId);
      if (!session) {
        // The run's conversation is gone from the catalogue (deleted, or the
        // catalogue is stale). Saying so beats opening an unrelated conversation.
        setError(t("tasksRunGone"));
        return;
      }
      onOpenSession(peer.desktopId, session.sessionId);
    }
    catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    }
  }, [onOpenSession, peer.desktopId, t]);

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header className="flex h-12 shrink-0 select-none items-center gap-3 border-b border-line-soft/40 px-4">
        <Button leftIcon={<ChevronLeft className="size-4" />} onClick={onBack} size="xs" variant="ghost">
          {t("backToDesktops")}
        </Button>
        <span aria-hidden className="text-base">{iconGlyph(peer.icon)}</span>
        <span className="min-w-0 flex-1 truncate text-sm font-semibold text-ink">
          {t("tasksOnHost", { name: peerBadgeText(peer, peer.desktopId) })}
        </span>
      </header>

      {error ? <p className="shrink-0 border-b border-line-soft px-4 py-2 text-xs text-danger">{error}</p> : null}

      {/* `onOpenThread` is async in this direction, which the editor does not
          await — it only needs the click to lead somewhere, and a failure is
          reported in the bar above rather than thrown into the editor. */}
      <div className="min-h-0 flex-1">
        <TasksView
          backend={backend}
          leftPanelExpanded={leftPanelExpanded}
          modelOptions={models}
          onOpenThread={threadId => void openThread(threadId)}
          onToggleLeftPanel={onToggleLeftPanel}
        />
      </div>
    </section>
  );
}
