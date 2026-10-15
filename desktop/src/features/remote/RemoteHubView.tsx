import type { ReactNode } from "react";
import type { RemotePeer } from "../remote-peer/remotePeerClient";
import type { RemoteStatus } from "../remote/remoteClient";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { errorMessage } from "../../lib/errors";
import { emitFutureEvent } from "../../lib/futureEvents";
import { startWindowDrag } from "../../lib/windowDrag";
import { setRemotePeerLabel } from "../remote-peer/remotePeerClient";
import { RemotePeerSettings } from "../remote-peer/RemotePeerSettings";
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
 *
 * A machine's tasks and its settings are **pages of this window** — they replace
 * this page rather than opening a window of their own, because the user is still
 * working in this one and leaving it (its size, position and place) is a bigger
 * move than the errand deserves. Each carries a back control at the top left,
 * which is where a reader looks for one.
 */
export function RemoteHubView({
  autoConnect,
  leftPanelExpanded,
  onOpenRemoteSession,
  onReconnectRemoteAll,
  onStartConversation,
  onToggleAutoConnect,
  onToggleLeftPanel,
  onRefreshRemote,
  remoteStatus,
}: {
  /** This machine's auto-connect preference for its paired devices. */
  autoConnect: boolean;
  leftPanelExpanded: boolean;
  /** Open one of a paired computer's conversations, by its session id. */
  onOpenRemoteSession: (desktopId: string, sessionId: string) => void;
  /**
   * Bring every paired computer up, the ones the user stopped included.
   *
   * The liveness pass is the shell's — the same call the shell makes at launch,
   * for the machines it is supposed to keep live — so this page only asks for it.
   */
  onReconnectRemoteAll: () => Promise<void>;
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
  /** The machine whose settings page is open, if any. */
  const [settingsFor, setSettingsFor] = useState<RemotePeer | null>(null);
  /** The machine whose tasks are open, if any. They need the whole column. */
  const [tasksFor, setTasksFor] = useState<RemotePeer | null>(null);
  /**
   * Bumped whenever something outside the machine list changed it — a rename on
   * a machine's settings page — so the list re-reads when the user comes back
   * instead of showing what it read when the page was first opened.
   */
  const [refreshNonce, setRefreshNonce] = useState(0);

  /** A rename that could not be stored locally: said out loud, not swallowed. */
  function reportLabelFailure(error: unknown) {
    emitFutureEvent("toast", { message: errorMessage(error), tone: "error" });
  }

  if (tasksFor) {
    return (
      <RemoteTasksView
        leftPanelExpanded={leftPanelExpanded}
        onBack={() => setTasksFor(null)}
        onOpenSession={onOpenRemoteSession}
        onToggleLeftPanel={onToggleLeftPanel}
        peer={tasksFor}
      />
    );
  }

  if (settingsFor) {
    return (
      <RemotePeerSettings
        onBack={() => setSettingsFor(null)}
        onChanged={() => setRefreshNonce(nonce => nonce + 1)}
        onOpenTasks={() => setTasksFor(settingsFor)}
        onUpdateLabel={(patch) => {
          void setRemotePeerLabel(settingsFor.desktopId, patch)
            // The page describes the machine from the peer it was opened with;
            // the name and icon live in the local credential book, so the page
            // has to be told what it just became.
            .then(() => setSettingsFor(current => (current ? { ...current, ...patch } : current)))
            .then(() => setRefreshNonce(nonce => nonce + 1))
            .catch(reportLabelFailure);
        }}
        peer={settingsFor}
      />
    );
  }

  return (
    <Shell
      leftPanelExpanded={leftPanelExpanded}
      onToggleLeftPanel={onToggleLeftPanel}
      title={t("title")}
    >
      <div className="space-y-4">
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
          <RemoteClientSection
            onOpenSettings={setSettingsFor}
            onReconnectAll={onReconnectRemoteAll}
            onStartConversation={onStartConversation}
            refreshNonce={refreshNonce}
          />
        </Direction>
      </div>
    </Shell>
  );
}

/**
 * The hub's page shell: title bar, and a scrolling column for the content.
 *
 * Only the hub itself uses it — a machine's settings and its tasks are pages
 * that bring their own header (with their own back control), which is what makes
 * them read as pages rather than as sections of this one.
 */
function Shell({
  children,
  leftPanelExpanded,
  onToggleLeftPanel,
  title,
}: {
  children: ReactNode;
  leftPanelExpanded: boolean;
  onToggleLeftPanel: () => void;
  title: string;
}) {
  return (
    <section className="flex h-full min-h-0 flex-col bg-surface">
      <header
        className="flex h-12 shrink-0 select-none items-center justify-between border-b border-line-soft/40 px-4"
        onMouseDown={startWindowDrag}
      >
        <div className="flex min-w-0 flex-1 items-center" data-tauri-drag-region>
          <LeftPanelTitlebarToggle expanded={leftPanelExpanded} onToggle={onToggleLeftPanel} />
          <span className="truncate text-sm font-semibold text-ink">{title}</span>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto p-8">
        <div className="mx-auto w-full max-w-3xl">{children}</div>
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
