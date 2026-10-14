import type { RemotePeer } from "./remotePeerClient";
import { describe, expect, it } from "vitest";
import {
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
});
