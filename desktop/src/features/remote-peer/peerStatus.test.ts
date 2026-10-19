import type { RemotePeer } from "./remotePeerClient";
import { describe, expect, it } from "vitest";
import {
  peerDrivable,
  peerHeld,
  peerHolderName,
  peerLabelKey,
  peerLabelTone,
  peersConnected,
  peersDotState,
  peerStatus,
  peerStatusKey,
  peerStatusTone,
} from "./peerStatus";

/**
 * The vocabulary every surface that names a machine reads from.
 *
 * Asserted as a table because the point of the module is that these four
 * surfaces (a settings row, the device picker, the rail dot, the count above
 * the list) cannot disagree about the same machine.
 */

function peer(overrides: Partial<RemotePeer> = {}): RemotePeer {
  return {
    agentAvailable: true,
    bridgeInstanceId: "bridge_1",
    connected: true,
    desktopId: "desktop_a",
    error: null,
    features: [],
    icon: "laptop",
    name: "Studio iMac",
    pairId: "pair_1",
    stopped: false,
    ...overrides,
  };
}

describe("peerStatus", () => {
  it("reads the runtime's own facts", () => {
    const cases: Array<[string, Partial<RemotePeer>, string]> = [
      ["a live link with a live agent", {}, "connected"],
      ["a live link whose agent is not up", { agentAvailable: false }, "preparing"],
      ["a host that has never answered", { connected: false }, "connecting"],
      ["a host that refused or dropped", { connected: false, error: "boom" }, "offline"],
      ["a host the user turned off", { connected: false, stopped: true }, "stopped"],
    ];
    for (const [what, overrides, expected] of cases)
      expect(peerStatus(peer(overrides)), what).toBe(expected);
  });

  /**
   * A deliberate stop leaves the last failure behind in the runtime (the socket
   * really did go away). Reporting it as a fault would tell the user their own
   * decision went wrong.
   */
  it("calls a stopped host stopped even with a stored error", () => {
    expect(peerStatus(peer({ connected: false, error: "socket closed", stopped: true })))
      .toBe("stopped");
  });

  it("names each status in the section's own copy", () => {
    expect(peerStatusKey("connected")).toBe("statusConnected");
    expect(peerStatusKey("preparing")).toBe("statusAgentUnavailable");
    expect(peerStatusKey("connecting")).toBe("statusConnecting");
    expect(peerStatusKey("offline")).toBe("statusOffline");
    expect(peerStatusKey("stopped")).toBe("statusDisconnected");
  });

  it("gives each status the tone that says whether anything is wrong", () => {
    expect(peerStatusTone("connected")).toBe("accent");
    expect(peerStatusTone("preparing")).toBe("warning");
    expect(peerStatusTone("connecting")).toBe("warning");
    expect(peerStatusTone("offline")).toBe("danger");
    // Switched off is not broken.
    expect(peerStatusTone("stopped")).toBe("neutral");
  });
});

/**
 * Connected is not the same as usable.
 *
 * A machine another paired device drives answers every heartbeat and refuses
 * every read. Calling it "connected" — green badge, green dot — tells the user
 * everything works, one line above a list that is missing exactly its
 * conversations. These are the five states plus that one answer.
 */
describe("being held by another paired device", () => {
  const held = peer({ control: "standby", holder: { name: "Hui's phone" } });

  it("is connected, and still not ours to drive", () => {
    expect(peerStatus(held)).toBe("connected");
    expect(peerHeld(held)).toBe(true);
    expect(peerDrivable(held)).toBe(false);
  });

  it("is drivable when nobody else has it", () => {
    expect(peerDrivable(peer())).toBe(true);
    expect(peerHeld(peer())).toBe(false);
    // A host that predates the field says nothing; the machine is assumed ours.
    expect(peerDrivable(peer({ control: undefined }))).toBe(true);
    expect(peerDrivable(peer({ control: "active" }))).toBe(true);
  });

  it("gets its own word and an amber badge, not 'connected'", () => {
    expect(peerLabelKey(held)).toBe("statusTakenOver");
    expect(peerLabelTone(held)).toBe("warning");
    // Every other machine keeps the status vocabulary.
    expect(peerLabelKey(peer())).toBe("statusConnected");
    expect(peerLabelTone(peer())).toBe("accent");
    expect(peerLabelKey(peer({ connected: false, error: "gone" }))).toBe("statusOffline");
    expect(peerLabelTone(peer({ connected: false, error: "gone" }))).toBe("danger");
  });

  it("names the holder when it declared one, and says nothing when it did not", () => {
    expect(peerHolderName(held)).toBe("Hui's phone");
    expect(peerHolderName(peer({ control: "standby" }))).toBeNull();
    expect(peerHolderName(peer({ control: "standby", holder: { name: "   " } }))).toBeNull();
  });

  it("is not held while the link is down, whatever the last state said", () => {
    expect(peerHeld(peer({ control: "standby", connected: false, error: "gone" }))).toBe(false);
    expect(peerDrivable(peer({ control: "standby", connected: false, error: "gone" }))).toBe(false);
  });
});

describe("peersConnected", () => {
  it("counts only the machines that are really up", () => {
    const counts = peersConnected([
      peer(),
      peer({ desktopId: "desktop_b", agentAvailable: false }),
      peer({ desktopId: "desktop_c", connected: false, error: "gone" }),
      peer({ desktopId: "desktop_d", connected: false, stopped: true }),
    ]);
    expect(counts).toEqual({ all: false, any: true, connected: 1, total: 4 });
  });

  it("is all-connected only when every machine answers", () => {
    expect(peersConnected([peer(), peer({ desktopId: "desktop_b" })]).all).toBe(true);
  });

  /** Nothing paired is not "all connected": there is no set to judge. */
  it("treats no machines as neither all nor any", () => {
    expect(peersConnected([])).toEqual({ all: false, any: false, connected: 0, total: 0 });
  });
});

describe("peersDotState", () => {
  it("is green only when every machine is up", () => {
    expect(peersDotState([peer(), peer({ desktopId: "desktop_b" })])).toBe("up");
  });

  /**
   * The state the dot exists for: a merged list short of one machine, which no
   * single machine's own status could express.
   */
  it("is partial while some answer", () => {
    expect(peersDotState([peer(), peer({ desktopId: "desktop_b", connected: false })])).toBe("partial");
  });

  it("is down when none answer", () => {
    expect(peersDotState([peer({ connected: false })])).toBe("down");
  });

  /** A machine with nothing paired has no client-role state at all. */
  it("is nothing when nothing is paired", () => {
    expect(peersDotState([])).toBeNull();
  });

  /**
   * Every machine answers and none of them is ours: the merged list is empty of
   * their conversations, and green would say the opposite.
   */
  it("is partial when every machine answers but none is ours to drive", () => {
    expect(peersDotState([peer({ control: "standby" })])).toBe("partial");
    expect(peersDotState([
      peer({ control: "standby" }),
      peer({ desktopId: "desktop_b", control: "standby" }),
    ])).toBe("partial");
  });

  it("stays green while at least one machine is drivable", () => {
    expect(peersDotState([
      peer({ control: "standby" }),
      peer({ desktopId: "desktop_b" }),
    ])).toBe("up");
  });
});
