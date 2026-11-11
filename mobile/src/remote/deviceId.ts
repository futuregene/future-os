/**
 * The display form of the id a phone is paired under.
 *
 * The id itself is minted on the first pairing and stored in the system
 * keychain (`storage.ts`); this is only how it is written on screen. Two
 * surfaces use it — this phone's own settings row, and the device list of a
 * machine it is paired with — and they have to agree, or the user comparing the
 * two screens to find out which phone is connected is comparing two different
 * strings.
 *
 * Short, not truncated for no reason: `dev_` plus the first eight characters is
 * what a person reads off one screen and looks for on the other. The full value
 * is a lookup key, not a secret, but it is long enough that nobody compares it
 * by eye.
 */
export function shortDeviceId(deviceId: string): string {
  const trimmed = deviceId.trim();
  const body = trimmed.startsWith("dev_") ? trimmed.slice(4) : trimmed;
  return `dev_${body.slice(0, 8)}`;
}
