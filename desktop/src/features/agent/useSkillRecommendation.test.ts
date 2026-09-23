// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import i18n from "../../i18n";
import {
  listAvailableSkills,
  listInstalledSkills,
  recordSkillReco,
  skillRecoToday,
  suggestSkill,
} from "../../integrations/skills/skillsClient";
import { renderHook } from "../../test/renderHook";
import {
  DAILY_RECOMMENDATION_LIMIT,
  MAX_QUERY_CHARS,
  messageHash,
  MIN_QUERY_BYTES,
  shownDescription,
  useSkillRecommendation,
} from "./useSkillRecommendation";

vi.mock("../../integrations/skills/skillsClient", () => ({
  // The hook goes through the shared cache, which assembles both lists from the
  // per-call mocks below, so a test still drives the lists it cares about.
  loadSkillCatalog: () => ({
    installed: listInstalledSkills(),
    catalogue: listAvailableSkills(),
  }),
  listAvailableSkills: vi.fn(),
  listInstalledSkills: vi.fn(),
  suggestSkill: vi.fn(),
  skillRecoToday: vi.fn(),
  recordSkillReco: vi.fn(),
}));

const available = vi.mocked(listAvailableSkills);
const installed = vi.mocked(listInstalledSkills);
const suggest = vi.mocked(suggestSkill);
const today = vi.mocked(skillRecoToday);
const record = vi.mocked(recordSkillReco);

/** A draft just long enough to pass the minimum-length gate. */
const LONG_ENOUGH = "please search the web for this";

function baseOptions(overrides: Partial<Parameters<typeof useSkillRecommendation>[0]> = {}) {
  return {
    enabled: true,
    sessionStatus: "authenticated",
    balance: 10,
    ...overrides,
  };
}

function catalogueEntry(id: string, descriptionZh = "") {
  return { id, name: id, description: `${id} description`, nameZh: "", descriptionZh, category: "", categoryZh: "", latestVersion: "1.0" };
}

beforeEach(() => {
  available.mockResolvedValue([
    catalogueEntry("future-web"),
    catalogueEntry("future-paper"),
    catalogueEntry("future-slides"),
  ] as never);
  installed.mockResolvedValue([
    { id: "future-slides", name: "future-slides", description: "slides", nameZh: null, descriptionZh: null, version: "1.0" },
  ] as never);
  suggest.mockResolvedValue(null);
  today.mockResolvedValue({ count: 0, skillIds: [], messageHashes: [] });
  record.mockResolvedValue(undefined);
});

afterEach(async () => {
  vi.clearAllMocks();
  // The setup file pins English; a test that switches languages puts it back so
  // the other tests in this file keep reading the English wording.
  await act(async () => {
    await i18n.changeLanguage("en");
  });
});

async function renderActive(options = baseOptions()) {
  const harness = renderHook(() => useSkillRecommendation(options));
  // Flush the candidate-loading effect.
  await act(async () => {});
  return harness;
}

it("offers only UNINSTALLED catalogue skills as candidates", async () => {
  const hook = await renderActive();
  expect(hook.current.candidates.map(c => c.name).sort()).toEqual(["future-paper", "future-web"]);
});

it("does not evaluate when the toggle is off", async () => {
  const hook = await renderActive(baseOptions({ enabled: false }));
  const reco = await act(() => hook.current.evaluate(LONG_ENOUGH));
  expect(reco).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
  expect(today).not.toHaveBeenCalled();
});

it("does not evaluate when logged out", async () => {
  const hook = await renderActive(baseOptions({ sessionStatus: "signed_out" }));
  expect(await act(() => hook.current.evaluate(LONG_ENOUGH))).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("does not evaluate when the balance is zero", async () => {
  const hook = await renderActive(baseOptions({ balance: 0 }));
  expect(await act(() => hook.current.evaluate(LONG_ENOUGH))).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("does not evaluate for an over-long draft", async () => {
  const hook = await renderActive();
  expect(await act(() => hook.current.evaluate("x".repeat(MAX_QUERY_CHARS + 1)))).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("does not evaluate a draft shorter than the minimum byte length", async () => {
  const hook = await renderActive();
  // "你好" is 6 bytes; 9 汉字 is 27 bytes — both under the 30-byte gate.
  expect(await act(() => hook.current.evaluate("你好"))).toBeNull();
  expect(await act(() => hook.current.evaluate("单细胞测序如何分析"))).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("evaluates exactly at the minimum byte length", async () => {
  const hook = await renderActive();
  // 10 汉字 = 30 bytes = the documented threshold.
  expect(new TextEncoder().encode("单细胞测序如何分析流").length).toBe(MIN_QUERY_BYTES);
  await act(() => hook.current.evaluate("单细胞测序如何分析流"));
  expect(suggest).toHaveBeenCalledTimes(1);
});

it("does not evaluate when the draft already picks a skill", async () => {
  const hook = await renderActive();
  expect(await act(() => hook.current.evaluate("please /future-web search this thing"))).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("returns, stores and records a recommendation on a hit", async () => {
  suggest.mockResolvedValue({ name: "future-web", description: "web" });
  const hook = await renderActive();
  const reco = await act(() => hook.current.evaluate(LONG_ENOUGH));
  expect(reco?.name).toBe("future-web");
  expect(hook.current.state.recommendation?.name).toBe("future-web");
  expect(suggest).toHaveBeenCalledWith(LONG_ENOUGH, hook.current.candidates);
  expect(record).toHaveBeenCalledWith("future-web", messageHash(LONG_ENOUGH));
});

it("does not consume the daily budget when the recommender finds nothing", async () => {
  suggest.mockResolvedValue(null);
  const hook = await renderActive();
  expect(await act(() => hook.current.evaluate(LONG_ENOUGH))).toBeNull();
  expect(record).not.toHaveBeenCalled();
});

it("returns null (submit normally) when the recommender errors", async () => {
  suggest.mockRejectedValue(new Error("agent down"));
  const hook = await renderActive();
  expect(await act(() => hook.current.evaluate(LONG_ENOUGH))).toBeNull();
  expect(hook.current.state.recommendation).toBeNull();
  expect(record).not.toHaveBeenCalled();
});

it("returns null on a slow recommender (timeout budget)", async () => {
  vi.useFakeTimers();
  try {
    suggest.mockImplementation(() => new Promise(() => {})); // never resolves
    const hook = await renderActive();
    let resolved: unknown = "unset";
    await act(() => {
      const pending = hook.current.evaluate(LONG_ENOUGH).then((r) => {
        resolved = r;
      });
      return vi.advanceTimersByTimeAsync(1600).then(() => pending);
    });
    expect(resolved).toBeNull();
    expect(record).not.toHaveBeenCalled();
  }
  finally {
    vi.useRealTimers();
  }
});

it("stops calling the recommender once the daily budget is spent", async () => {
  today.mockResolvedValue({
    count: DAILY_RECOMMENDATION_LIMIT,
    skillIds: ["future-web", "future-paper", "future-slides"],
    messageHashes: [],
  });
  const hook = await renderActive();
  expect(await act(() => hook.current.evaluate(LONG_ENOUGH))).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("still calls the recommender below the daily budget", async () => {
  today.mockResolvedValue({ count: DAILY_RECOMMENDATION_LIMIT - 1, skillIds: ["future-paper"], messageHashes: [] });
  suggest.mockResolvedValue({ name: "future-web", description: "web" });
  const hook = await renderActive();
  expect((await act(() => hook.current.evaluate(LONG_ENOUGH)))?.name).toBe("future-web");
});

it("skips a skill already recommended today without recording a second time", async () => {
  today.mockResolvedValue({ count: 1, skillIds: ["future-web"], messageHashes: ["other-hash"] });
  suggest.mockResolvedValue({ name: "future-web", description: "web" });
  const hook = await renderActive();
  expect(await act(() => hook.current.evaluate(LONG_ENOUGH))).toBeNull();
  expect(hook.current.state.recommendation).toBeNull();
  expect(record).not.toHaveBeenCalled();
});

it("does not re-evaluate a message that already produced a recommendation", async () => {
  today.mockResolvedValue({ count: 1, skillIds: ["future-web"], messageHashes: [messageHash(LONG_ENOUGH)] });
  const hook = await renderActive();
  expect(await act(() => hook.current.evaluate(LONG_ENOUGH))).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("dismiss clears the shown card", async () => {
  suggest.mockResolvedValue({ name: "future-web", description: "web" });
  const hook = await renderActive();
  await act(() => hook.current.evaluate(LONG_ENOUGH));
  expect(hook.current.state.recommendation).not.toBeNull();
  act(() => hook.current.dismiss());
  expect(hook.current.state.recommendation).toBeNull();
});

it("hashes messages stably and distinctly", () => {
  expect(messageHash("hello")).toBe(messageHash("hello"));
  expect(messageHash("hello")).not.toBe(messageHash("hello!"));
  expect(messageHash("单细胞测序")).not.toBe(messageHash("single cell"));
  expect(messageHash("")).toHaveLength(16);
});

it("shows the catalogue's Chinese description in a Chinese UI", async () => {
  available.mockResolvedValue([
    catalogueEntry("future-web", "搜索公开网页并核实信息"),
  ] as never);
  suggest.mockResolvedValue({ name: "future-web", description: "search the web" });
  const hook = await renderActive();
  await act(async () => {
    await i18n.changeLanguage("zh");
  });
  const reco = await act(() => hook.current.evaluate(LONG_ENOUGH));
  expect(reco?.description).toBe("搜索公开网页并核实信息");
  expect(hook.current.state.recommendation?.description).toBe("搜索公开网页并核实信息");
});

/**
 * The recommender's own payload is *not* localized: it stays the English text
 * the evaluation was tuned on, whatever the UI language is.
 */
it("still asks the recommender in English under a Chinese UI", async () => {
  available.mockResolvedValue([
    catalogueEntry("future-web", "搜索公开网页并核实信息"),
  ] as never);
  suggest.mockResolvedValue({ name: "future-web", description: "search the web" });
  const hook = await renderActive();
  await act(async () => {
    await i18n.changeLanguage("zh");
  });
  await act(() => hook.current.evaluate(LONG_ENOUGH));
  expect(suggest).toHaveBeenCalledWith(LONG_ENOUGH, [
    { name: "future-web", description: "future-web description" },
  ]);
});

it("shows the English description when there is no Chinese one (or the UI is English)", () => {
  const card = { name: "future-web", description: "search the web" };
  const zh = new Map([["future-web", "搜索公开网页"]]);
  expect(shownDescription(card, "zh", zh)).toBe("搜索公开网页");
  // An empty or whitespace-only Chinese line is the same as a missing one.
  expect(shownDescription(card, "zh", new Map([["future-web", "   "]]))).toBe(card.description);
  expect(shownDescription(card, "zh", new Map())).toBe(card.description);
  // An English UI ignores whatever the catalogue carries.
  expect(shownDescription(card, "en", zh)).toBe("search the web");
});
