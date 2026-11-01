import type { MachineSummary } from "../remote/desktopStatus";

/** One machine the share sheet can offer, and whether it will take the share. */
export interface ShareDevice {
  desktopId: string;
  /** What the user called the machine; its id when they have not named it. */
  label: string;
  /** The glyph they picked for it, if any. */
  icon?: string;
  /** The machine answers, and this phone is the device driving it. */
  canTake: boolean;
}

/**
 * The machines a share can land on, in the order the phone holds them (the one
 * it is reading first).
 *
 * A share arrives on the phone and belongs to no machine, so this is the whole
 * list the sheet has to answer with — and a machine that cannot take one stays
 * in it, disabled: a reader looking for a paired desktop is owed the row, not
 * an absence they have to explain to themselves.
 *
 * "Can take" is the two facts a host needs at once: it is `connected` — the one
 * state in which it answers at all — and this phone is the device driving it,
 * because a machine another paired device holds refuses every command this one
 * sends even while its heartbeat is fresh.
 */
export function shareDevices(machines: readonly MachineSummary[]): ShareDevice[] {
  return machines.map(machine => ({
    desktopId: machine.desktopId,
    label: machine.label,
    ...(machine.icon ? { icon: machine.icon } : {}),
    canTake: machine.status === "connected" && machine.control !== "standby",
  }));
}

/**
 * Whether the sheet has to ask *which* machine before asking where on it.
 *
 * With one machine that can take the share there is nothing to ask, and the
 * sheet opens on its destinations exactly as it always has. The exception is
 * that machine not being the one the phone is reading: those destinations are
 * the open machine's, and it cannot take the share at all, so the choice of
 * machine is a real one even though only one row in it works.
 */
export function needsDeviceStep(
  devices: readonly ShareDevice[],
  activeDesktopId: string | null,
): boolean {
  const takers = devices.filter(device => device.canTake);
  if (takers.length > 1) return true;
  const only = takers[0];
  return only !== undefined && only.desktopId !== activeDesktopId;
}
