/**
 * U+FFFD is a byte the wire lost; a control character is never part of a label
 * a person typed and reads as a broken glyph in most renderings.
 */
const ILLEGIBLE = /[\uFFFD\u0000-\u001F\u007F-\u009F]/;

/**
 * Whether a device name is worth showing at all.
 *
 * A name reaches this phone from two places that are not ours to trust: another
 * device's own declaration, and this phone's stored copy of one taken when the
 * machine was paired. Both can carry U+FFFD. A Chinese computer name that went
 * through a launcher decoding it with a local code page arrives as
 * `FutureOS Desktop (␦␦␦␦␦␦ļ␦)` — one replacement per lost byte — and on screen
 * that is a row of replacement boxes where the name should be. It cannot be
 * read, searched for, or matched to the machine it claims to be, so it is not a
 * name.
 *
 * The rule is the one the host and the relay already apply where a name enters
 * the system (`host/src/remote_peer/runtime.rs::usable_name`,
 * `packages/remote-server/src/commands.rs::declared_name`): drop it, and let the
 * surface fall back to its own "another device" wording. Applying it on the
 * client as well is what keeps a stale peer — one still running a build that
 * sends the leftovers — from putting them on this screen, where the wording for
 * an unnamed device exists and a mangled one has none.
 *
 * Deliberately not about non-ASCII: a machine named in Chinese is a name, and
 * dropping it would hide every device whose owner named it in their own
 * language.
 */
export function readableDeviceName(value: string | null | undefined): string | undefined {
  const name = value?.trim();
  if (!name) return undefined;
  return ILLEGIBLE.test(name) ? undefined : name;
}
