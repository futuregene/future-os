import AsyncStorage from "@react-native-async-storage/async-storage";
import {
  DAILY_LIMIT,
  alreadyEvaluated,
  alreadyRecommended,
  clearDay,
  count,
  emptyDay,
  exhausted,
  loadDay,
  messageHash,
  parseDay,
  recordShown,
  today,
} from "../skillRecoBudget";

jest.mock("@react-native-async-storage/async-storage", () => {
  // A stateful fake: the budget is only meaningful if a record reads back after
  // being written, so the mock stores what it is given.
  const store = new Map<string, string>();
  return {
    __esModule: true,
    default: {
      getItem: jest.fn(async (key: string) => store.get(key) ?? null),
      setItem: jest.fn(async (key: string, value: string) => {
        store.set(key, value);
      }),
      removeItem: jest.fn(async (key: string) => {
        store.delete(key);
      }),
      __store: store,
    },
  };
});

const storage = AsyncStorage as jest.Mocked<typeof AsyncStorage>;
const storageStore = (AsyncStorage as unknown as { __store: Map<string, string> }).__store;
const STORAGE_KEY = "futureos.mobile.skill-reco.v1";

beforeEach(() => {
  jest.clearAllMocks();
  storageStore.clear();
});

describe("the daily budget", () => {
  it("starts empty and accumulates until the limit", async () => {
    expect(exhausted(await loadDay())).toBe(false);
    await recordShown("future-web", messageHash("first"));
    await recordShown("future-paper", messageHash("second"));
    const day = await loadDay();
    expect(count(day)).toBe(2);
    expect(exhausted(day)).toBe(false);
    await recordShown("future-slides", messageHash("third"));
    expect(exhausted(await loadDay())).toBe(true);
    expect(count(await loadDay())).toBe(DAILY_LIMIT);
  });

  it("remembers which skills and messages it has already handled", async () => {
    await recordShown("future-web", messageHash("asked about a website"));
    const day = await loadDay();
    expect(alreadyRecommended(day, "future-web")).toBe(true);
    expect(alreadyRecommended(day, "future-paper")).toBe(false);
    expect(alreadyEvaluated(day, messageHash("asked about a website"))).toBe(true);
    expect(alreadyEvaluated(day, messageHash("something else"))).toBe(false);
  });

  it("persists the record under one versioned key", async () => {
    await recordShown("future-web", "hash");
    expect(storage.setItem).toHaveBeenCalledWith(
      STORAGE_KEY,
      expect.stringContaining("future-web"),
    );
    const written = JSON.parse(storageStore.get(STORAGE_KEY) as string);
    expect(written.day).toBe(today());
    expect(written.skills).toEqual(["future-web"]);
    expect(written.messages).toEqual(["hash"]);
  });
});

describe("degrading safely", () => {
  it("reads a record from another day as empty", () => {
    const stale = JSON.stringify({ day: "2020-01-01", skills: ["future-web"], messages: ["x"] });
    const day = parseDay(stale);
    expect(count(day)).toBe(0);
    expect(day.day).toBe(today());
  });

  it("reads corrupt or wrong-typed storage as empty", () => {
    for (const raw of ["not json", "[]", "null", '"text"', "{}", '{"day":"x"}']) {
      expect(count(parseDay(raw))).toBe(0);
    }
  });

  it("cannot exceed the limit even if the record was hand-edited", () => {
    const over = JSON.stringify({
      day: today(),
      skills: ["a", "b", "c", "d", "e"],
      messages: ["1", "2", "3", "4", "5"],
    });
    const day = parseDay(over);
    expect(count(day)).toBe(DAILY_LIMIT);
    expect(exhausted(day)).toBe(true);
  });

  it("treats a storage read failure as an empty day", async () => {
    storage.getItem.mockRejectedValueOnce(new Error("storage unavailable"));
    expect(count(await loadDay())).toBe(0);
  });

  it("still reports the shown recommendation when the write fails", async () => {
    // The card is displayed either way; losing the record costs one extra
    // recommendation at most, which must not fail the send.
    storage.setItem.mockRejectedValueOnce(new Error("disk full"));
    const day = await recordShown("future-web", "hash");
    expect(day.skills).toEqual(["future-web"]);
  });

  it("clears the record", async () => {
    await clearDay();
    expect(storage.removeItem).toHaveBeenCalledWith(STORAGE_KEY);
  });
});

describe("the message hash", () => {
  it("is stable, ignores surrounding space, and does not carry the text", () => {
    expect(messageHash("hello")).toBe(messageHash("hello"));
    expect(messageHash("  hello  ")).toBe(messageHash("hello"));
    expect(messageHash("hello")).not.toBe(messageHash("hello!"));
    const hash = messageHash("单细胞测序");
    expect(hash).toHaveLength(16);
    expect(hash).not.toContain("单");
    // Multi-byte input hashes differently from its latin rendering, i.e. the
    // bytes are actually encoded rather than the code points stringified.
    expect(hash).not.toBe(messageHash("single cell"));
  });

  it("hashes the same bytes as the desktop and TUI clients", () => {
    // FNV-1a over UTF-8; the value is shared so a message recognised on one
    // client reads the same in the others' logs and records.
    expect(messageHash("")).toBe("cbf29ce484222325");
    expect(messageHash("a")).toBe("af63dc4c8601ec8c");
  });
});

describe("emptyDay", () => {
  it("is today with nothing recorded", () => {
    expect(emptyDay()).toEqual({ day: today(), skills: [], messages: [] });
  });
});

describe("the message hash is over UTF-8 bytes, not UTF-16 code units", () => {
  /** An independent FNV-1a over an explicit byte list. */
  function fnvBytes(bytes: number[]): string {
    let hash = 0xcbf29ce484222325n;
    const mask = 0xffffffffffffffffn;
    for (const byte of bytes) hash = ((hash ^ BigInt(byte)) * 0x100000001b3n) & mask;
    return hash.toString(16).padStart(16, "0");
  }
  const utf8 = (text: string) => Array.from(new TextEncoder().encode(text.trim()));

  // Two-byte, three-byte and four-byte code points take three different arms of
  // the hand-rolled encoder; each must match the platform encoder byte for byte.
  test.each([
    ["ascii", "please search the web"],
    ["two-byte", "café résumé"],
    ["three-byte", "请帮我搜索一下这个内容"],
    ["four-byte", "check this out 🙂🙃"],
    ["mixed with astral", "a🙂中é z"],
    ["trimmed", "  spaced  "],
  ])("%s text hashes identically to a byte-wise FNV-1a over its UTF-8 bytes", (_label, text) => {
    expect(messageHash(text)).toBe(fnvBytes(utf8(text)));
  });

  test("a two-byte character contributes its two UTF-8 bytes, not one Latin-1 byte", () => {
    // "é" is U+00E9 → 0xC3 0xA9. Hashing the code unit instead would give the
    // single byte 0xE9 and silently collide different messages.
    expect(utf8("é")).toEqual([0xc3, 0xa9]);
    expect(messageHash("é")).toBe(fnvBytes([0xc3, 0xa9]));
    expect(messageHash("é")).not.toBe(fnvBytes([0xe9]));
    expect(messageHash("é")).not.toBe(messageHash("e"));
  });

  test("a four-byte code point contributes four UTF-8 bytes, not two surrogates", () => {
    expect(utf8("🙂")).toEqual([0xf0, 0x9f, 0x99, 0x82]);
    expect(messageHash("🙂")).toBe(fnvBytes([0xf0, 0x9f, 0x99, 0x82]));
    // The surrogate pair U+1F642 is encoded in UTF-16 as ED A0 BD ED B9 82 under
    // a naive unit-wise encoder — neither surrogate's own bytes may appear.
    expect(messageHash("🙂")).not.toBe(fnvBytes([0xd8, 0x3d, 0xde, 0x42]));
    expect(messageHash("🙂")).not.toBe(fnvBytes([0xed, 0xa0, 0xbd, 0xed, 0xb9, 0x82]));
  });

  test("a three-byte character contributes three UTF-8 bytes", () => {
    expect(utf8("中")).toEqual([0xe4, 0xb8, 0xad]);
    expect(messageHash("中")).toBe(fnvBytes([0xe4, 0xb8, 0xad]));
    // A two-byte reading of the same string would give a different digest.
    expect(messageHash("中")).not.toBe(fnvBytes([0x4e, 0x2d]));
  });
});

/**
 * The budget resets on the user's local midnight, not UTC's. This distinction is
 * invisible for most of the day — UTC and a positive-offset local date agree
 * until the offset rolls over — so it has to be pinned at a fixed instant where
 * they disagree, or a switch to `toISOString` would pass CI for 16 hours a day
 * and quietly reset (or extend) the budget for the rest.
 */
/**
 * The budget resets on the user's local midnight, not UTC's. The distinction is
 * invisible for most of the day (and always invisible on a UTC host, which is
 * what CI runs), so it has to be pinned by forcing a disagreement rather than
 * by trusting the runner's timezone.
 */
describe("the day is the local calendar day", () => {
  it("reads local date parts, never the UTC date", () => {
    const now = new Date("2026-09-25T16:47:00Z");
    // 16:47Z is already the 26th at UTC+8. Force that disagreement, then assert
    // the local parts win — on a UTC host they otherwise agree and the point
    // would be unobservable.
    const year = jest.spyOn(Date.prototype, "getFullYear").mockReturnValue(2026);
    const month = jest.spyOn(Date.prototype, "getMonth").mockReturnValue(8);
    const date = jest.spyOn(Date.prototype, "getDate").mockReturnValue(26);
    try {
      expect(now.toISOString().slice(0, 10)).toBe("2026-09-25");
      expect(today(now)).toBe("2026-09-26");
    } finally {
      year.mockRestore();
      month.mockRestore();
      date.mockRestore();
    }
  });

  it("treats a record from another day as nothing spent", () => {
    // The consequence of the two calendars disagreeing: a record stamped with a
    // different day reads as stale, so the budget is silently reset. This is
    // what made the recommendation feature re-offer a skill it had already shown
    // in the first hours of every local day.
    const now = new Date("2026-09-25T16:47:00Z");
    const recorded = JSON.stringify({ day: "2026-09-24", skills: ["future-web"], messages: ["h"] });
    const year = jest.spyOn(Date.prototype, "getFullYear").mockReturnValue(2026);
    const month = jest.spyOn(Date.prototype, "getMonth").mockReturnValue(8);
    const date = jest.spyOn(Date.prototype, "getDate").mockReturnValue(26);
    try {
      expect(parseDay(recorded, now)).toEqual({ day: "2026-09-26", skills: [], messages: [] });
      // And the record is honoured once it carries the local day.
      const sameDay = JSON.stringify({ day: "2026-09-26", skills: ["future-web"], messages: ["h"] });
      expect(parseDay(sameDay, now)).toEqual({ day: "2026-09-26", skills: ["future-web"], messages: ["h"] });
    } finally {
      year.mockRestore();
      month.mockRestore();
      date.mockRestore();
    }
  });
});
