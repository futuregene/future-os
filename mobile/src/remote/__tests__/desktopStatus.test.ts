import {
  activeMachineStatus,
  machineConfigurable,
  machineStatusKey,
  machinesConnected,
  machinesPartlyConnected,
  pairedMachines,
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
      .toEqual({ all: false, any: true, connected: 1, total: 2, drivable: 1 });
    expect(machinesConnected([machine("a", "offline"), machine("b", "connecting")]).all)
      .toBe(false);
  });

  test("counts a machine held by another device as connected but undrivable", () => {
    // The dot's green needs both: answering *and* ours to command. A standby
    // machine passes the first and fails the second, which is why it is yellow.
    const held = { ...machine("a", "connected"), control: "standby" as const };
    expect(machinesConnected([held]).all).toBe(true);
    expect(machinesConnected([held]).drivable).toBe(0);
    expect(machinesConnected([held, machine("b", "connected")]).drivable).toBe(1);
  });

  test("says nothing about machines it does not have", () => {
    // No pairing at all: the dot falls back to the active connection's own
    // presentation rather than reporting "0 of 0 connected".
    expect(machinesConnected([]))
      .toEqual({ all: false, any: false, connected: 0, total: 0, drivable: 0 });
  });
});

describe("whose words the status block uses", () => {  const counts = (...statuses: MachineSummary["status"][]) =>
    machinesConnected(statuses.map((status, index) => machine(`m${index}`, status)));
  const down = connectionPresentation({
    phase: "failed",
    desktopOnline: false,
    error: "whatever the transport said",
  });
  const ready = connectionPresentation({ phase: "ready", desktopOnline: true });
  const revoked = connectionPresentation({ phase: "revoked", desktopOnline: false });

  test("the set's, while some machines answer and some do not", () => {
    // The case that read as "no connection" under a list saying two of three
    // desktops were connected, whichever of them the open machine happened to be.
    expect(machinesPartlyConnected(counts("connected", "connected", "offline"), ready)).toBe(true);
    expect(machinesPartlyConnected(counts("connected", "connecting", "offline"), down)).toBe(true);
    expect(machinesPartlyConnected(counts("preparing", "connected"), down)).toBe(true);
  });

  test("the open machine's, when every machine answers", () => {
    expect(machinesPartlyConnected(counts("connected", "connected"), ready)).toBe(false);
  });

  test("the open machine's, when nothing answers", () => {
    // There is no set to describe: its cause, its support code and the action
    // that fits it are what the reader needs.
    expect(machinesPartlyConnected(counts("offline", "connecting"), down)).toBe(false);
    expect(machinesPartlyConnected(counts("offline", "offline"), ready)).toBe(false);
  });

  test("a single machine has no set to speak for", () => {
    expect(machinesPartlyConnected(counts("offline"), down)).toBe(false);
    expect(machinesPartlyConnected(counts(), ready)).toBe(false);
  });

  test("a pairing this machine lost keeps its own words and its own action", () => {
    // "Some desktops are not connected" cannot explain an unpair prompt: the
    // expired pairing is the thing to fix, on the machine whose conversation it
    // was.
    expect(machinesPartlyConnected(counts("connected", "offline"), revoked)).toBe(false);
  });
});

describe("one list of the paired machines", () => {
  const desktops = [
    { desktopId: "open", pairId: "pair-1", name: "Studio", icon: "rocket" },
    { desktopId: "other", pairId: "pair-2" },
  ];
  const ready = connectionPresentation({ phase: "ready", desktopOnline: true });

  test("judges the open machine from the connection the app renders from", () => {
    const [open] = pairedMachines({
      desktops,
      activeDesktopId: "open",
      presence: { control: "active" },
      presentation: ready,
      others: [],
    });
    expect(open).toEqual({
      desktopId: "open",
      label: "Studio",
      icon: "rocket",
      active: true,
      status: "connected",
      control: "active",
    });
  });

  test("falls back to the id for a machine the user never named", () => {
    const [open] = pairedMachines({
      desktops,
      activeDesktopId: "other",
      presence: null,
      presentation: ready,
      others: [],
    });
    expect(open!.label).toBe("other");
    expect(open!.icon).toBeUndefined();
  });

  test("keeps the other machines' own report, and never lists one twice", () => {
    const peer: MachineSummary = {
      desktopId: "other",
      label: "Other",
      active: false,
      status: "offline",
    };
    const machines = pairedMachines({
      desktops,
      activeDesktopId: "open",
      presence: { control: "active" },
      presentation: ready,
      // The catalogue is supposed to exclude the open machine; a stale entry
      // must not draw it twice under two states.
      others: [peer, { ...peer, desktopId: "open", status: "connected" }],
    });
    expect(machines.map(machine => machine.desktopId)).toEqual(["open", "other"]);
    expect(machines[1]).toEqual(peer);
  });

  test("with no open machine it is just the others", () => {
    const peer: MachineSummary = {
      desktopId: "other",
      label: "Other",
      active: false,
      status: "connecting",
    };
    expect(
      pairedMachines({
        desktops,
        activeDesktopId: null,
        presence: null,
        presentation: ready,
        others: [peer],
      }),
    ).toEqual([peer]);
  });
});

describe("whether a machine's settings are reachable", () => {
  test("only while it answers and is this phone's to drive", () => {
    expect(machineConfigurable({ desktopId: "a", label: "a", active: false, status: "connected" }))
      .toBe(true);
    expect(
      machineConfigurable({ desktopId: "a", label: "a", active: false, status: "connected", control: "active" }),
    ).toBe(true);
  });

  test("not while it is off, reconnecting or still starting", () => {
    for (const status of ["offline", "connecting", "preparing"] as const) {
      expect(machineConfigurable({ desktopId: "a", label: "a", active: false, status })).toBe(false);
    }
  });

  test("not while another device holds it, whatever its status says", () => {
    // It answers every heartbeat and refuses every read: the page would open on
    // a banner and no rows.
    expect(
      machineConfigurable({ desktopId: "a", label: "a", active: false, status: "connected", control: "standby" }),
    ).toBe(false);
  });

  test("missing data is not a fault", () => {
    // No runtime has reported on this machine yet; the row keeps its action
    // rather than greying out on an absence of evidence.
    expect(machineConfigurable(undefined)).toBe(true);
  });
});
