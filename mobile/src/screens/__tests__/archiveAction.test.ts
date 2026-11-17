import type { RemoteSession } from "../../remote/types";
import type { AppAlertButton } from "../../components/appAlerts";
import { archiveSessionWithPrompt, runningInWorkspace } from "../archiveAction";

const session = (id: string, extra: Partial<RemoteSession> = {}): RemoteSession => ({
  sessionId: id,
  threadId: `t-${id}`,
  title: id,
  streaming: false,
  ...extra,
});

/** Resolve once `fn` has been called: the archive path crosses several awaits. */
async function waitForCall(fn: { mock: { calls: unknown[] } }, attempts = 50): Promise<void> {
  for (let index = 0; index < attempts && fn.mock.calls.length === 0; index += 1)
    await new Promise(resolve => setTimeout(resolve, 0));
}

describe("the archive prompt's running set", () => {
  it("names one workspace's running conversations on one machine", () => {
    const rows = [
      { desktopId: "a", session: session("run", { workspaceId: "w", streaming: true }) },
      { desktopId: "a", session: session("idle", { workspaceId: "w" }) },
      // Another machine's identical workspace id is another workspace.
      { desktopId: "b", session: session("peer", { workspaceId: "w", streaming: true }) },
      { desktopId: "a", session: session("elsewhere", { workspaceId: "other", streaming: true }) },
    ];
    expect(runningInWorkspace(rows, "w", "a").map(item => item.sessionId)).toEqual(["run"]);
  });

  it("answers nothing for a workspace with no running conversation", () => {
    const rows = [{ desktopId: null, session: session("idle", { workspaceId: "w" }) }];
    expect(runningInWorkspace(rows, "w", null)).toEqual([]);
  });
});

describe("the archive prompt's run-log notice", () => {
  type AlertCall = { title: string; message: string; buttons: AppAlertButton[] };

  function harness(overrides: { streaming?: boolean } = {}) {
    const alerts: AlertCall[] = [];
    const archiveSession = jest.fn(async () => {});
    const abortSession = jest.fn(async () => {});
    const onNoticeSeen = jest.fn();
    const onFailed = jest.fn();
    const t = (key: string, options?: Record<string, unknown>) => {
      const text = options?.name ? `${key}:${String(options.name)}` : key;
      return text;
    };
    return {
      alerts,
      archiveSession,
      abortSession,
      onNoticeSeen,
      onFailed,
      t,
      commands: { archiveSession, abortSession } as never,
      input: {
        alert: (title: string, message?: string, buttons?: AppAlertButton[]) => {
          alerts.push({ title, message: message ?? "", buttons: buttons ?? [] });
        },
        onFailed,
        onNoticeSeen,
        session: session("s1", { streaming: overrides.streaming ?? false }),
        t,
      },
    };
  }

  it("archives straight away when the notice was already shown", () => {
    const h = harness();
    archiveSessionWithPrompt({ ...h.input, cleanup: { enabled: true, noticeSeen: true }, commands: h.commands });

    expect(h.alerts).toHaveLength(0);
    expect(h.archiveSession).toHaveBeenCalledTimes(1);
    expect(h.onNoticeSeen).not.toHaveBeenCalled();
  });

  it("asks once before clearing logs, even for a conversation that is idle", async () => {
    const h = harness();
    archiveSessionWithPrompt({ ...h.input, cleanup: { enabled: true, noticeSeen: false }, commands: h.commands });

    // The menu item alone cannot say what the clearing takes with it.
    expect(h.alerts).toHaveLength(1);
    expect(h.alerts[0]!.message).toContain("sessions.archiveCleanupNotice");
    expect(h.alerts[0]!.buttons[1]!.text).toBe("sessions.archiveAndClear");
    expect(h.archiveSession).not.toHaveBeenCalled();

    h.alerts[0]!.buttons[1]!.onPress!();
    await Promise.resolve();
    await Promise.resolve();
    expect(h.archiveSession).toHaveBeenCalledTimes(1);
    expect(h.onNoticeSeen).toHaveBeenCalledTimes(1);
  });

  it("archives without asking when cleanup is turned off", () => {
    const h = harness();
    archiveSessionWithPrompt({ ...h.input, cleanup: { enabled: false, noticeSeen: false }, commands: h.commands });

    expect(h.alerts).toHaveLength(0);
    expect(h.archiveSession).toHaveBeenCalledTimes(1);
  });

  it("combines the running question with the notice into one dialog", async () => {
    const h = harness({ streaming: true });
    archiveSessionWithPrompt({ ...h.input, cleanup: { enabled: true, noticeSeen: false }, commands: h.commands });

    expect(h.alerts).toHaveLength(1);
    expect(h.alerts[0]!.title).toBe("sessions.archiveRunningTitle");
    expect(h.alerts[0]!.message).toContain("sessions.archiveRunningConfirm");
    expect(h.alerts[0]!.message).toContain("sessions.archiveCleanupNotice");
    expect(h.alerts[0]!.buttons[1]!.text).toBe("sessions.archiveStopAndClear");

    h.alerts[0]!.buttons[1]!.onPress!();
    // The running path awaits the abort, then the retrying archive loop.
    await waitForCall(h.onNoticeSeen);
    expect(h.abortSession).toHaveBeenCalledTimes(1);
    expect(h.archiveSession).toHaveBeenCalledTimes(1);
    expect(h.onNoticeSeen).toHaveBeenCalledTimes(1);
  });

  it("does not record the notice when the reader cancels", () => {
    const h = harness();
    archiveSessionWithPrompt({ ...h.input, cleanup: { enabled: true, noticeSeen: false }, commands: h.commands });

    // Cancel has no handler: it closes the dialog and nothing else.
    expect(h.alerts[0]!.buttons[0]!.text).toBe("chat.cancel");
    expect(h.alerts[0]!.buttons[0]!.onPress).toBeUndefined();
    expect(h.onNoticeSeen).not.toHaveBeenCalled();
    expect(h.archiveSession).not.toHaveBeenCalled();
  });
});
