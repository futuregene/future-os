/**
 * The zone list a scheduled task's picker offers, read the way the desktop reads
 * the same list.
 *
 * A task's trigger times are read on the desktop's clock, so the zones offered
 * here are the names that clock accepts — every IANA zone this runtime knows,
 * and a short list of common ones where the runtime cannot enumerate them
 * (`Intl.supportedValuesOf` is missing on some engines the phone may carry).
 * Whatever the task already carries is added either way, so opening an edit
 * never offers a zone the task is no longer able to keep.
 */

/** Zones offered when the runtime cannot enumerate its own list. */
const COMMON_ZONES = [
  "UTC",
  "Asia/Shanghai",
  "Asia/Tokyo",
  "Asia/Singapore",
  "Asia/Hong_Kong",
  "Asia/Taipei",
  "Asia/Seoul",
  "Asia/Kolkata",
  "Asia/Dubai",
  "Europe/London",
  "Europe/Paris",
  "Europe/Berlin",
  "Europe/Moscow",
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/Sao_Paulo",
  "Australia/Sydney",
];

let knownZones: string[] | null = null;

/** Every zone this runtime knows, read once: it does not change mid-session. */
export function allTimezones(): string[] {
  if (knownZones)
    return knownZones;
  let zones: string[] = [];
  try {
    const supported = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] })
      .supportedValuesOf?.("timeZone");
    if (Array.isArray(supported))
      zones = supported.slice();
  }
  catch {
    // Treated as "not supported" below.
  }
  knownZones = zones.length > 0 ? zones : COMMON_ZONES;
  return knownZones;
}

/** Every zone, with whatever the task carries added if the list lacks it. */
export function timezoneOptions(current: string): string[] {
  const zones = allTimezones();
  const value = current.trim();
  return value && !zones.includes(value) ? [value, ...zones] : zones;
}

/** The zones whose name contains `query`, case-insensitively, in list order. */
export function matchTimezones(query: string, current: string): string[] {
  const needle = query.trim().toLowerCase().replace(/\s+/g, "_");
  const zones = timezoneOptions(current);
  if (!needle)
    return zones;
  return zones.filter(zone => zone.toLowerCase().includes(needle));
}
