import type { RemoteCatalog } from "./mergeConversations";
import type { RemoteIdentityMemory } from "./remoteConversationIdentity";
import { describe, expect, it } from "vitest";
import { rememberRemoteIdentities, remoteConversationIdentity, remoteIdentityKey } from "./remoteConversationIdentity";

/**
 * What an open conversation is called once its host stops reporting it.
 *
 * The case this exists for: another paired device takes the machine over, so the
 * host refuses this client's reads and sends it no more snapshots, every row of
 * that host leaves the catalogue — and the title went with them, turning a
 * conversation the reader was reading into its session id.
 */
function memory(): RemoteIdentityMemory {
  return new Map();
}

function catalog(sessions: Array<Partial<RemoteCatalog["sessions"][number]> & { sessionId: string }>): RemoteCatalog {
  return {
    desktopId: "desk_a",
    sessions: sessions.map(row => ({ title: row.sessionId, ...row })),
  };
}

const target = { desktopId: "desk_a", sessionId: "sess_1" };

describe("remoteConversationIdentity", () => {
  it("reads the title from the catalogue while the host still reports the row", () => {
    expect(remoteConversationIdentity([catalog([{ sessionId: "sess_1", title: "Rename on the host" }])], target, memory()))
      .toEqual({ title: "Rename on the host", workspaceName: null });
  });

  it("keeps the last title after the catalogue drops the row", () => {
    const identities = memory();
    rememberRemoteIdentities([catalog([{ sessionId: "sess_1", title: "Deadlock in the revive pass" }])], identities);
    // The host stopped answering (another device holds it), so its rows are gone.
    expect(remoteConversationIdentity([], target, identities))
      .toEqual({ title: "Deadlock in the revive pass", workspaceName: null });
  });

  it("keeps the workspace name with the title", () => {
    const identities = memory();
    rememberRemoteIdentities(
      [catalog([{ sessionId: "sess_1", title: "Release notes", mode: "workspace", workspaceName: "future-os" }])],
      identities,
    );
    expect(remoteConversationIdentity([], target, identities))
      .toEqual({ title: "Release notes", workspaceName: "future-os" });
  });

  it("leaves the workspace off a chat conversation, remembered or not", () => {
    const identities = memory();
    rememberRemoteIdentities(
      // A chat row that carries a name anyway: the mode decides, not the payload.
      [catalog([{ sessionId: "sess_1", title: "Quick question", mode: "chat", workspaceName: "future-os" }])],
      identities,
    );
    expect(remoteConversationIdentity([], target, identities).workspaceName).toBeNull();
  });

  it("falls back to the session id for a conversation it has never seen", () => {
    expect(remoteConversationIdentity([], target, memory()))
      .toEqual({ title: "sess_1", workspaceName: null });
  });

  it("prefers a catalogue row to what was remembered", () => {
    const identities = memory();
    rememberRemoteIdentities([catalog([{ sessionId: "sess_1", title: "Old name" }])], identities);
    expect(remoteConversationIdentity([catalog([{ sessionId: "sess_1", title: "Renamed" }])], target, identities).title)
      .toBe("Renamed");
  });

  it("keys by host as well as session: one session id on two machines is two conversations", () => {
    const identities = memory();
    rememberRemoteIdentities([catalog([{ sessionId: "sess_1", title: "On A" }])], identities);
    expect(remoteConversationIdentity([], { desktopId: "desk_b", sessionId: "sess_1" }, identities).title)
      .toBe("sess_1");
    expect(remoteIdentityKey("desk_a", "sess_1")).not.toBe(remoteIdentityKey("desk_b", "sess_1"));
  });

  it("keeps only the newest title seen for a row", () => {
    const identities = memory();
    rememberRemoteIdentities([catalog([{ sessionId: "sess_1", title: "First" }])], identities);
    rememberRemoteIdentities([catalog([{ sessionId: "sess_1", title: "Second" }])], identities);
    expect(remoteConversationIdentity([], target, identities).title).toBe("Second");
  });
});
