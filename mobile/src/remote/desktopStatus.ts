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
 * Whether every listed machine is connected — the badge's green condition.
 *
 * `all` is what the dot needs: a phone paired to three desktops is "connected"
 * only when all three answer, and one of them being down is a state the reader
 * has to see rather than a detail hidden behind the machine that happens to be
 * the active one.
 */
export function machinesConnected(machines: readonly MachineSummary[]): {
  all: boolean;
  any: boolean;
  connected: number;
  total: number;
} {
  const connected = machines.filter(machine => machine.status === "connected").length;
  return {
    all: machines.length > 0 && connected === machines.length,
    any: connected > 0,
    connected,
    total: machines.length,
  };
}
