import type { ReactNode } from "react";
import type { RemotePeer } from "../remote-peer/remotePeerClient";
import type { RemoteStatus } from "../remote/remoteClient";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { startWindowDrag } from "../../lib/windowDrag";
import { RemoteClientSection } from "../remote-peer/RemotePeersView";
import { RemoteTasksView } from "../remote-peer/RemoteTasksView";
import { RemoteHostSection } from "../remote/RemoteView";

/**
 * The Remote page: one entry in the rail, both directions of remote on it.
 *
 * They used to be two rail entries a word apart — "Phone Control" and "Remote
 * Desktops" — which asked the user to work out from the label which side of the
 * connection they were configuring. Nothing in either label said "who connects
 * to whom", and the names themselves were not even accurate any more: a
 * computer* can connect to this machine, and this machine can connect to a
 * computer, so neither direction is about phones.
 *
 * So the page leads with that distinction in words, then splits by **direction**
 * — "let other devices connect here" and "connect to another computer" — which
 * is a difference a reader gets right on the first pass.
 *
 * The two directions really are independent: one needs a FutureOS sign-in and
 * the other does not, one is a server and the other a client, and either can be
 * used without the other. That independence is why each is a section that
 * carries its own state, rather than one merged control surface.
 */
export function RemoteHubView({
  autoConnect,
  leftPanelExpanded,
  onOpenRemoteSession,
  onStartConversation,
  onToggleAutoConnect,
  onToggleLeftPanel,
  onRefreshRemote,
  remoteStatus,
}: {
  /** This machine's auto-connect preference for its paired devices. */
  autoConnect: boolean;
  leftPanelExpanded: boolean;
  /** Open one of a paired computer's conversations by its session id. */
  onOpenRemoteSession: (desktopId: string, sessionId: string) => void;
  /** Open a new conversation on a paired computer; owned by the shell. */
  onStartConversation: (desktopId: string) => void;
  onToggleAutoConnect: (value: boolean) => void;
  onToggleLeftPanel: () => void;
  onRefreshRemote: () => Promise<void>;
  remoteStatus: RemoteStatus | null;
}) {
  const { t } = useTranslation("remoteHub");
  const { t: tRemote } = useTranslation("remote");
  const { t: tPeer } = useTranslation("remotePeer");
  /**
   * The host whose tasks are open, if any.
   *
   * Held here rather than inside the client section because the task editor
   * needs the whole column — it is a list and a detail side by side, and the
   * section lives in a narrow scrolling pane.
   */
  const [tasksPeer, setTasksPeer] = useState<RemotePeer | null>(null);

  if (tasksPeer) {
    return (
      <RemoteTasksView
        leftPanelExpanded={leftPanelExpanded}
        onBack={() => setTasksPeer(null)}
        onOpenSession={onOpenRemoteSession}
        onToggleLeftPanel={onToggleLeftPanel}
        peer={tasksPeer}
      />
    );
  }

  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between border-b border-line-soft/40 px-4"
        onMouseDown={startWindowDrag}
      >
        <div className="flex min-w-0 flex-1 items-center" data-tauri-drag-region>
          <LeftPanelTitlebarToggle expanded={leftPanelExpanded} onToggle={onToggleLeftPanel} />
          <span className="truncate text-sm font-semibold text-ink">{t("title")}</span>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-8">
        <div className="mx-auto w-full max-w-3xl space-y-6">
          <p className="text-sm text-ink-muted" data-testid="remote-hub-lead">{t("lead")}</p>

          <Direction
            subtitle={tRemote("description")}
            testId="remote-hub-host"
            title={t("hostTitle")}
          >
            <RemoteHostSection
              autoConnect={autoConnect}
              onRefreshRemote={onRefreshRemote}
              onToggleAutoConnect={onToggleAutoConnect}
              remoteStatus={remoteStatus}
            />
          </Direction>

          <Direction
            subtitle={tPeer("description")}
            testId="remote-hub-client"
            title={t("clientTitle")}
          >
            <RemoteClientSection onOpenTasks={setTasksPeer} onStartConversation={onStartConversation} />
          </Direction>
        </div>
      </div>
    </section>
  );
}

/**
 * One direction, with its heading and its explanation.
 *
 * The explanation is required rather than optional: the whole reason these two
 * live on one page is that the user has to be able to tell them apart, and a
 * bare heading relies on them already knowing which side is which.
 */
function Direction({
  children,
  subtitle,
  testId,
  title,
}: {
  children: ReactNode;
  subtitle: string;
  testId: string;
  title: string;
}) {
  return (
    <section
      className="rounded-lg border border-line-soft bg-surface-subtle p-4"
      data-testid={testId}
    >
      <h2 className="text-sm font-semibold text-ink">{title}</h2>
      <p className="mt-1 text-xs leading-5 text-ink-muted">{subtitle}</p>
      <div className="mt-4">{children}</div>
    </section>
  );
}
