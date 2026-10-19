import type { ConnectionPresentation } from "./connectionPresentation";

/**
 * How one paired machine stands, as the desktop picker and the connection
 * popover show it.
 *
 * Four states, because they are the four questions a reader asks of a machine
 * that is listed but not talking: is it answering at all (`connected`), is its
 * agent up (`preparing`), is it still being reached (`connecting`), or is it
 * simply not there (`offline`).
 */
export type MachineStatus = "connected" | "preparing" | "connecting" | "offline";

export interface MachineSummary {
  desktopId: string;
  /** What the user called the machine; the id when they have not named it. */
  label: string;
  /** The glyph they picked for it, if any. */
  icon?: string;
  /** The machine the phone is talking to for the open conversation. */
  active: boolean;
  status: MachineStatus;
  /**
   * Whether this phone may drive that machine.
   *
   * A machine can answer every heartbeat and still not be ours to command:
   * another paired device is in front of it. That is a different fact from
   * `status` — "connected, but not driving" — and the two must not be collapsed,
   * or a phone that was just displaced reads as fully working.
   */
  control?: "active" | "standby";
}

/** i18n key for a status, under the popover's rows. */
export function machineStatusKey(status: MachineStatus): string {
  return `connection.machine.${status}`;
}

/**
 * A peer's status from the transport's own facts. `ready` is the handshake's
 * own answer, `online` the judged presence heartbeat, `agentAvailable` the
 * desktop's report that its Agent is up — a machine whose socket is healthy but
 * whose agent is restarting is `preparing`, not `connected`.
 */
export function peerMachineStatus({
  failed,
  ready,
  online,
  agentAvailable,
}: {
  failed: boolean;
  ready: boolean;
  online: boolean;
  agentAvailable: boolean;
}): MachineStatus {
  if (failed) return "offline";
  if (!ready) return "connecting";
  if (!online) return "offline";
  return agentAvailable ? "connected" : "preparing";
}

/**
 * The active machine's status, taken from the connection presentation the badge
 * already shows: the same facts, read through the state the whole app renders
 * from, so the dot and the row can never disagree.
 */
export function activeMachineStatus(
  presentation: ConnectionPresentation,
): MachineStatus {
  if (presentation.level === "connected") return "connected";
  if (presentation.level === "connecting")
    return presentation.customerState === "devicePreparing" ? "preparing" : "connecting";
  return "offline";
}

/**
 * Whether every listed machine is connected — the badge's green condition —
 * and whether any of them may actually be driven.
 *
 * `all` is what the dot needs: a phone paired to three desktops is "connected"
 * only when all three answer, and one of them being down is a state the reader
 * has to see rather than a detail hidden behind the machine that happens to be
 * the active one.
 *
 * `drivable` is the second axis. A machine that answers but is held by another
 * device is connected and *not usable*: the phone can read nothing from it and
 * send nothing to it. One such machine is already not a green set: the dot
 * answers for every paired machine, so a held machine turns it yellow exactly as
 * an offline one does. That the others are still usable is the rows' answer, not
 * the dot's — "somewhere in the list works" is not what a green dot promises.
 */
export interface MachinesConnected {
  all: boolean;
  any: boolean;
  connected: number;
  total: number;
  /** Machines this phone may drive: connected and not standing by. */
  drivable: number;
}

export function machinesConnected(machines: readonly MachineSummary[]): MachinesConnected {
  const answered = machines.filter(machine => machine.status === "connected");
  const connected = answered.length;
  return {
    all: machines.length > 0 && connected === machines.length,
    any: connected > 0,
    connected,
    total: machines.length,
    drivable: answered.filter(machine => machine.control !== "standby").length,
  };
}

/**
 * Whether the status block's own words are the set's rather than one machine's.
 *
 * The dot, the rows and the button below them all answer for every paired
 * machine; a block that reported the machine whose conversation happens to be
 * open beneath them is how a phone with two of three desktops up came to read
 * as "no connection", directly under a list saying two of them were connected.
 * So when some machines answer and some do not, the block says that instead.
 *
 * Two cases keep the ordinary copy, because there the open machine's own state
 * *is* the phone's: nothing answering at all (it names the cause, its support
 * code and the action for it), and a pairing this phone's machine has lost
 * (only that connection's own words can explain an "unpair" prompt).
 */
export function machinesPartlyConnected(
  counts: MachinesConnected,
  active: ConnectionPresentation,
): boolean {
  return counts.total > 1 && counts.any && !counts.all && active.action !== "pairAgain";
}
