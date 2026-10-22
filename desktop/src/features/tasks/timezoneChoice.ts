/**
 * The choices a scheduled task's time-zone picker offers, in the reader's own
 * language and with the offset spelled out.
 *
 * A zone identifier (`America/Los_Angeles`) is the value a task stores, but it is
 * a poor thing to read: it is English, it says nothing about how far it is from
 * the reader, and two side-by-side entries look interchangeable. So each row is
 * built from two facts the runtime already knows — the zone's own name in the
 * current locale (`Intl.DisplayNames`) and its offset from UTC right now — and the
 * identifier stays only as the value that gets saved (and as a search key, since
 * that is what a support note or another app will quote).
 *
 * Offsets are computed by reading the zone's wall clock and comparing it with the
 * same instant in UTC, rather than by parsing a formatted name. A string like
 * `GMT+5:30` would have to be parsed back into a number for sorting anyway, and
 * the parse differs by engine; the arithmetic does not.
 */

/**
 * The zones worth offering before the full list: the reader is rarely in the
 * rest.
 */
const COMMON_ZONES: readonly string[] = [
  "Asia/Shanghai",
  "Asia/Hong_Kong",
  "Asia/Taipei",
  "Asia/Tokyo",
  "Asia/Seoul",
  "Asia/Singapore",
  "Asia/Bangkok",
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
  "Pacific/Auckland",
  "UTC",
];

/**
 * The zones a runtime without `Intl.supportedValuesOf` still gets.
 *
 * An older webview answers nothing, and a picker with one row is worse than a
 * short list: these are the same common zones, so the form stays usable and the
 * reader can still name the offset they need.
 */
const FALLBACK_ZONES: readonly string[] = COMMON_ZONES.filter(zone => zone !== "UTC");

/**
 * Other ways a reader names one of the common zones.
 *
 * CLDR's display names are the *standard's* names — `Asia/Shanghai` is
 * “中国标准时间”, `Asia/Tokyo` is “日本标准时间” — so searching for the city a
 * reader has in mind finds nothing. These are extra search terms only; the row
 * still reads the display name, and the value still stored is the identifier.
 * The list stops at the zones that lead the picker: translating a city for all
 * four hundred would be a dictionary to maintain, and nobody scrolls that far.
 */
const ZONE_ALIASES: Readonly<Record<string, string>> = {
  "Asia/Shanghai": "上海 shanghai",
  "Asia/Hong_Kong": "香港 hong kong",
  "Asia/Taipei": "台北 taipei",
  "Asia/Tokyo": "东京 tokyo japan",
  "Asia/Seoul": "首尔 seoul korea",
  "Asia/Singapore": "新加坡 singapore",
  "Asia/Bangkok": "曼谷 bangkok thailand",
  "Asia/Kolkata": "印度 india mumbai delhi kolkata",
  "Asia/Dubai": "迪拜 dubai uae",
  "Europe/London": "伦敦 london uk britain",
  "Europe/Paris": "巴黎 paris france",
  "Europe/Berlin": "柏林 berlin germany",
  "Europe/Moscow": "莫斯科 moscow russia",
  "America/New_York": "纽约 new york usa east",
  "America/Chicago": "芝加哥 chicago usa central",
  "America/Denver": "丹佛 denver usa mountain",
  "America/Los_Angeles": "洛杉矶 los angeles usa west",
  "America/Sao_Paulo": "圣保罗 sao paulo brazil",
  "Australia/Sydney": "悉尼 sydney australia",
  "Pacific/Auckland": "奥克兰 auckland new zealand",
  "UTC": "utc 协调世界时 gmt 零时区",
};

export interface TimezoneChoice {
  /** The IANA name saved on the task; `""` is this machine's own zone. */
  zone: string;
  /** What the row reads, in the UI's language, without the offset. */
  label: string;
  /** The offset at the instant the list was built, e.g. `UTC+8`, `UTC-3:30`. */
  offset: string;
}

/** Every zone this webview knows, read once: the database does not change. */
let allZones: string[] | null = null;

function zoneList(): string[] {
  if (allZones)
    return allZones;
  let zones: string[] = [];
  try {
    const supported = (Intl as unknown as { supportedValuesOf?: (key: string) => string[] })
      .supportedValuesOf?.("timeZone");
    if (Array.isArray(supported))
      zones = supported;
  }
  catch {
    // Treated as "not supported" below.
  }
  allZones = zones.length > 0 ? zones : [...FALLBACK_ZONES];
  return allZones;
}

/**
 * The zone's name in `locale`, falling back to the identifier.
 *
 * The name comes from a formatter, not `Intl.DisplayNames`: the display-name API
 * has no `timeZone` type (ECMA-402 stops at calendar and date-time field), so
 * `{ timeZoneName: "shortGeneric" }` is the only standard way to ask what a zone
 * is called — it answers `中国时间` / `China Time`, and it answers it in every
 * engine this app runs on.
 *
 * `shortGeneric` rather than `long`: the long form names the standard in force
 * (`China Standard Time`), which changes with the season for the zones that
 * observe daylight saving, and a picker whose rows are renamed twice a year is
 * not a picker anyone learns. A name that is only an offset (`GMT+00:00`, or the
 * bare `GMT` that CLDR has for much of Africa) is not a name either — the
 * identifier names the place, so it is used instead.
 */
export function timezoneName(zone: string, locale: string): string {
  try {
    const parts = nameFormatter(zone, locale).formatToParts(new Date());
    const name = parts.find(part => part.type === "timeZoneName")?.value;
    if (!name)
      return zone;
    return /^(?:GMT|UTC)/i.test(name) ? zone : name;
  }
  catch {
    return zone;
  }
}

/**
 * The name formatter, per zone and language.
 *
 * A formatter fixes its zone when it is built, so naming four hundred zones means
 * four hundred of them — worth keeping for the life of the session, since the
 * picker is rebuilt every time the form opens.
 */
const nameFormatters = new Map<string, Intl.DateTimeFormat>();

function nameFormatter(zone: string, locale: string): Intl.DateTimeFormat {
  const key = `${locale}\u0000${zone}`;
  let formatter = nameFormatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat(locale, { timeZone: zone, timeZoneName: "shortGeneric" });
    nameFormatters.set(key, formatter);
  }
  return formatter;
}

/**
 * The zone's offset from UTC at `at`, in minutes east.
 *
 * The zone's wall clock is read as if it were UTC and compared with the instant
 * itself, which is the definition of the offset. `hour12: false` can answer `24`
 * for midnight in some engines, hence the modulo before the difference is taken.
 */
export function timezoneOffsetMinutes(zone: string, at: Date): number {
  if (zone === "UTC" || zone === "Etc/UTC")
    return 0;
  let parts: Intl.DateTimeFormatPart[];
  try {
    parts = new Intl.DateTimeFormat("en-US", {
      day: "2-digit",
      hour: "2-digit",
      hour12: false,
      minute: "2-digit",
      month: "2-digit",
      second: "2-digit",
      timeZone: zone,
      year: "numeric",
    }).formatToParts(at);
  }
  catch {
    // A zone this runtime does not know — a task written elsewhere, or a host
    // whose database has more zones than this webview's. It still belongs in the
    // list (dropping it would silently rewrite the task), and an offset of zero
    // is the honest answer to "how far is it": nobody knows.
    return 0;
  }
  const read = (type: string) => Number(parts.find(part => part.type === type)?.value);
  const wallClock = Date.UTC(
    read("year"),
    read("month") - 1,
    read("day"),
    read("hour") % 24,
    read("minute"),
    read("second"),
  );
  return Math.round((wallClock - at.getTime()) / 60_000);
}

/**
 * An offset as a reader says it: `UTC+8`, `UTC+5:30`, `UTC`, `UTC-3`.
 *
 * The reader asked for the number because the region alone does not say how far
 * away a zone is, so the sign is always explicit and half-hour zones keep their
 * minutes.
 */
export function formatOffset(minutes: number): string {
  if (minutes === 0)
    return "UTC";
  const sign = minutes > 0 ? "+" : "-";
  const total = Math.abs(minutes);
  const hours = Math.floor(total / 60);
  const rest = total % 60;
  return rest === 0 ? `UTC${sign}${hours}` : `UTC${sign}${hours}:${String(rest).padStart(2, "0")}`;
}

/** One choice, with its offset read at `at`. */
export function timezoneChoice(zone: string, locale: string, at: Date, systemLabel: string): TimezoneChoice {
  if (!zone)
    return { zone: "", label: systemLabel, offset: "" };
  return {
    zone,
    label: timezoneName(zone, locale),
    offset: formatOffset(timezoneOffsetMinutes(zone, at)),
  };
}

/** Every choice, in the order the picker offers it. */
export function timezoneChoices(
  locale: string,
  at: Date,
  systemLabel: string,
  current = "",
): TimezoneChoice[] {
  const zones = zoneList();
  // Whatever the task already stores is offered even when this runtime's list
  // lacks it, so an edit never silently rewrites a zone it cannot show.
  const all = current && current !== "UTC" && !zones.includes(current) ? [current, ...zones] : zones;
  const common = COMMON_ZONES.filter(zone => all.includes(zone));
  const rest = all
    .filter(zone => !common.includes(zone))
    .map(zone => ({ zone, minutes: timezoneOffsetMinutes(zone, at) }))
    // West to east (UTC-11 … UTC+14), then by identifier so the order is stable
    // and a reader scanning for "somewhere near me" finds the band in one pass.
    .sort((a, b) => a.minutes - b.minutes || compareIds(a.zone, b.zone));
  const row = (zone: string, minutes: number): TimezoneChoice => ({
    zone,
    label: timezoneName(zone, locale),
    offset: formatOffset(minutes),
  });
  return [
    // The unset choice leads: for most readers it is the right answer, and it is
    // what a new task starts on.
    { zone: "", label: systemLabel, offset: "" },
    ...common.map(zone => row(zone, timezoneOffsetMinutes(zone, at))),
    ...rest.map(({ zone, minutes }) => row(zone, minutes)),
  ];
}

/** Identifier order, by code unit: the same order on every machine. */
function compareIds(left: string, right: string): number {
  return left < right ? -1 : left > right ? 1 : 0;
}

/**
 * The choices a query keeps, by each thing a reader might type.
 *
 * The identifier, the localized name and the offset all match, because all three
 * are how people name a zone: "Shanghai", "上海", and "+8" should all find the
 * same row, and so should the stored value somebody copied out of a task.
 */
export function filterTimezoneChoices(choices: readonly TimezoneChoice[], query: string): TimezoneChoice[] {
  const needle = query.trim().toLowerCase();
  if (!needle)
    return [...choices];
  // `+8` / `-3` are how an offset is spoken, and `UTC+8` is how it is written;
  // both reduce to the same digits, so the sign is optional in the match.
  const bare = needle.replace(/^utc|^gmt/, "").replace(/\s+/g, "");
  return choices.filter((choice) => {
    if (choice.zone.toLowerCase().includes(needle))
      return true;
    if (choice.label.toLowerCase().includes(needle))
      return true;
    const alias = ZONE_ALIASES[choice.zone];
    if (alias?.includes(needle))
      return true;
    const offset = choice.offset.toLowerCase();
    return offset.includes(needle) || (bare.length > 0 && offset.replace(/^utc/, "").includes(bare));
  });
}

/** One line naming a zone: `上海时间 · UTC+8`, or the machine's own zone. */
export function timezoneLabel(zone: string, locale: string, at: Date, systemLabel: string): string {
  const choice = timezoneChoice(zone, locale, at, systemLabel);
  return choice.offset ? `${choice.label} · ${choice.offset}` : choice.label;
}
