import {
  activeMachineStatus,
  machineStatusKey,
  machinesConnected,
  peerMachineStatus,
  type MachineSummary,
} from "../desktopStatus";
import { connectionPresentation } from "../connectionPresentation";

const machine = (
  desktopId: string,
  status: MachineSummary["status"],
): MachineSummary => ({ desktopId, label: desktopId, active: false, status });

describe("a machine's status", () => {
  test("a healthy socket with a live heartbeat is connected", () => {
    expect(
      peerMachineStatus({ failed: false, ready: true, online: true, agentAvailable: true }),
    ).toBe("connected");
  });

  test("an agent that is restarting is preparing, not connected", () => {
    // The socket answers but nothing can run: the same state the badge calls
    // "desktop is preparing" for the active machine.
    expect(
      peerMachineStatus({ failed: false, ready: true, online: true, agentAvailable: false }),
    ).toBe("preparing");
  });

  test("a machine whose heartbeat stopped is offline", () => {
    expect(
      peerMachineStatus({ failed: false, ready: true, online: false, agentAvailable: true }),
    ).toBe("offline");
  });

  test("a connection that gave up is offline whatever it last heard", () => {
    expect(
      peerMachineStatus({ failed: true, ready: true, online: true, agentAvailable: true }),
    ).toBe("offline");
    // Still handshaking: not a failure, and not a claim that it is there.
    expect(
      peerMachineStatus({ failed: false, ready: false, online: true, agentAvailable: true }),
    ).toBe("connecting");
  });

  test("the active machine's status is read from the connection the app renders", () => {
    expect(activeMachineStatus(connectionPresentation({ phase: "ready", desktopOnline: true })))
      .toBe("connected");
    expect(activeMachineStatus(connectionPresentation({ phase: "ready", desktopOnline: true, agentAvailable: false })))
      .toBe("preparing");
    expect(activeMachineStatus(connectionPresentation({ phase: "ready", desktopOnline: false })))
      .toBe("connecting");
    expect(activeMachineStatus(connectionPresentation({ phase: "stopped", desktopOnline: false })))
      .toBe("offline");
  });

  test("every status has wording of its own", () => {
    expect(
      (["connected", "preparing", "connecting", "offline"] as const).map(machineStatusKey),
    ).toEqual([
      "connection.machine.connected",
      "connection.machine.preparing",
      "connection.machine.connecting",
      "connection.machine.offline",
    ]);
  });
});

describe("the dot's own answer", () => {
  test("is green only when every machine is connected", () => {
    expect(machinesConnected([machine("a", "connected"), machine("b", "connected")]).all)
      .toBe(true);
    expect(machinesConnected([machine("a", "connected"), machine("b", "offline")]))
      .toEqual({ all: false, any: true, connected: 1, total: 2 });
    expect(machinesConnected([machine("a", "offline"), machine("b", "connecting")]).all)
      .toBe(false);
  });

  test("says nothing about machines it does not have", () => {
    // No pairing at all: the dot falls back to the active connection's own
    // presentation rather than reporting "0 of 0 connected".
    expect(machinesConnected([])).toEqual({ all: false, any: false, connected: 0, total: 0 });
  });
});
