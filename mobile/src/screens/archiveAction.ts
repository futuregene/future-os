import type { AppAlertButton } from "../components/appAlerts";
import type { DesktopCommands } from "../remote/desktopCommands";
import type { RemoteSession } from "../remote/types";

/**
 * The app's own alert, as the screens hold it (`useAppDialog`), so these flows
 * share the app's dialog styling instead of reaching for the native one.
 */
type AlertFn = (title: string, message?: string, buttons?: AppAlertButton[]) => void;

/**
 * How long an archive keeps retrying while a run it just stopped settles.
 *
 * The host settles its own run row when the Agent acknowledges the abort, but
 * the streaming snapshot it also checks is updated by the Agent a moment later.
 * An archive issued in that window is refused as "still running"; retrying the
 * (idempotent, transactional) archive is what turns the reader's click into the
 * outcome they asked for, without guessing a fixed delay up front.
 */
const ARCHIVE_RETRY_DELAYS = [300, 400, 500, 600, 800];

export function sleep(ms: number): Promise<void> {
  return new Promise(resolve => setTimeout(resolve, ms));
}

export async function archiveWhenStopped(run: () => Promise<void>): Promise<void> {
  let lastError: unknown;
  for (let attempt = 0; attempt <= ARCHIVE_RETRY_DELAYS.length; attempt += 1) {
    try {
      await run();
      return;
    }
    catch (error) {
      lastError = error;
      const delay = ARCHIVE_RETRY_DELAYS[attempt];
      if (delay === undefined) break;
      await sleep(delay);
    }
  }
  throw lastError;
}

/**
 * The conversations in a workspace that are still running, as the catalogue
 * reports them. The archive confirmation names them, so it can say how many the
 * reader is about to stop.
 */
export function runningInWorkspace(
  rows: readonly { desktopId: string | null; session: RemoteSession }[],
  workspaceId: string,
  desktopId: string | null,
): RemoteSession[] {
  return rows
    .filter(row => row.session.streaming === true
      && (row.session.workspaceId ?? "") === workspaceId
      && row.desktopId === desktopId)
    .map(row => row.session);
}

/**
 * Archive one conversation, asking first when it is still running.
 *
 * Same two choices the desktop dialog offers: cancel, or stop it and archive.
 * A conversation that was not running skips the question — the menu item is the
 * decision there.
 */
export function archiveSessionWithPrompt(input: {
  alert: AlertFn;
  commands: DesktopCommands;
  onFailed: () => void;
  session: RemoteSession;
  t: (key: string, options?: Record<string, unknown>) => string;
}): void {
  const { alert, commands, onFailed, session, t } = input;
  const name = session.title || t("sessions.unnamed");
  const archive = () => archiveWhenStopped(() =>
    commands.archiveSession(session.sessionId, session.threadId));
  if (!session.streaming) {
    void commands.archiveSession(session.sessionId, session.threadId).catch(onFailed);
    return;
  }
  alert(
    t("sessions.archiveRunningTitle"),
    t("sessions.archiveRunningConfirm", { name }),
    [
      { text: t("chat.cancel"), style: "cancel" },
      {
        text: t("sessions.archiveStopAndArchive"),
        onPress: () => {
          void commands.abortSession(session.sessionId)
            .then(archive)
            .catch(onFailed);
        },
      },
    ],
  );
}

/**
 * Archive a workspace, asking first when any conversation in it is running.
 *
 * The prompt counts them, because a workspace archive stops all of them at once
 * and "3 conversations are running" is the fact the reader needs before saying
 * yes.
 */
export function archiveWorkspaceWithPrompt(input: {
  alert: AlertFn;
  commands: DesktopCommands;
  name: string;
  onFailed: () => void;
  running: readonly RemoteSession[];
  t: (key: string, options?: Record<string, unknown>) => string;
  workspaceId: string;
}): void {
  const { alert, commands, name, onFailed, running, t, workspaceId } = input;
  const archive = () => commands.archiveWorkspace(workspaceId);
  if (running.length === 0) {
    void archive().catch(onFailed);
    return;
  }
  alert(
    t("sessions.archiveWorkspaceRunningTitle"),
    t("sessions.archiveWorkspaceRunningConfirm", { count: running.length, name }),
    [
      { text: t("chat.cancel"), style: "cancel" },
      {
        text: t("sessions.archiveStopAndArchive"),
        onPress: () => {
          void Promise.all(running.map(session =>
            commands.abortSession(session.sessionId).catch(() => undefined)))
            .then(() => archiveWhenStopped(archive))
            .catch(onFailed);
        },
      },
    ],
  );
}
