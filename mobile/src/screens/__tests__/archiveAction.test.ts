import type { RemoteSession } from "../../remote/types";
import { runningInWorkspace } from "../archiveAction";

const session = (id: string, extra: Partial<RemoteSession> = {}): RemoteSession => ({
  sessionId: id,
  threadId: `t-${id}`,
  title: id,
  streaming: false,
  ...extra,
});

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
