// @vitest-environment jsdom
import { act } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { renderHook } from "../../test/renderHook";
import { MAX_QUERY_CHARS, useSkillRecommendation } from "./useSkillRecommendation";

vi.mock("../../integrations/skills/skillsClient", () => ({
  listAvailableSkills: vi.fn(),
  listInstalledSkills: vi.fn(),
  suggestSkill: vi.fn(),
}));

import { listAvailableSkills, listInstalledSkills, suggestSkill } from "../../integrations/skills/skillsClient";

const available = vi.mocked(listAvailableSkills);
const installed = vi.mocked(listInstalledSkills);
const suggest = vi.mocked(suggestSkill);

function baseOptions(overrides: Partial<Parameters<typeof useSkillRecommendation>[0]> = {}) {
  return {
    enabled: true,
    sessionStatus: "authenticated",
    balance: 10,
    ...overrides,
  };
}

beforeEach(() => {
  available.mockResolvedValue([
    { id: "future-web", name: "future-web", description: "web", nameZh: "", descriptionZh: "", category: "", categoryZh: "", latestVersion: "1.0" },
    { id: "future-paper", name: "future-paper", description: "paper", nameZh: "", descriptionZh: "", category: "", categoryZh: "", latestVersion: "1.0" },
    { id: "future-slides", name: "future-slides", description: "slides", nameZh: "", descriptionZh: "", category: "", categoryZh: "", latestVersion: "1.0" },
  ] as never);
  installed.mockResolvedValue([
    { id: "future-slides", name: "future-slides", description: "slides", nameZh: null, descriptionZh: null, version: "1.0" },
  ] as never);
  suggest.mockResolvedValue(null);
});

afterEach(() => {
  vi.clearAllMocks();
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
  const reco = await act(() => hook.current.evaluate("search the web"));
  expect(reco).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("does not evaluate when logged out", async () => {
  const hook = await renderActive(baseOptions({ sessionStatus: "signed_out" }));
  const reco = await act(() => hook.current.evaluate("search the web"));
  expect(reco).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("does not evaluate when the balance is zero", async () => {
  const hook = await renderActive(baseOptions({ balance: 0 }));
  const reco = await act(() => hook.current.evaluate("search the web"));
  expect(reco).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("does not evaluate for an over-long draft", async () => {
  const hook = await renderActive();
  const reco = await act(() => hook.current.evaluate("x".repeat(MAX_QUERY_CHARS + 1)));
  expect(reco).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("does not evaluate when the draft already picks a skill", async () => {
  const hook = await renderActive();
  const reco = await act(() => hook.current.evaluate("please /future-web search this"));
  expect(reco).toBeNull();
  expect(suggest).not.toHaveBeenCalled();
});

it("returns and stores a recommendation on a hit", async () => {
  suggest.mockResolvedValue({ name: "future-web", description: "web" });
  const hook = await renderActive();
  const reco = await act(() => hook.current.evaluate("search the web for this"));
  expect(reco?.name).toBe("future-web");
  expect(hook.current.state.recommendation?.name).toBe("future-web");
  expect(suggest).toHaveBeenCalledWith("search the web for this", hook.current.candidates);
});

it("returns null (submit normally) when the recommender errors", async () => {
  suggest.mockRejectedValue(new Error("agent down"));
  const hook = await renderActive();
  const reco = await act(() => hook.current.evaluate("search the web"));
  expect(reco).toBeNull();
  expect(hook.current.state.recommendation).toBeNull();
});

it("returns null on a slow recommender (timeout budget)", async () => {
  vi.useFakeTimers();
  try {
    suggest.mockImplementation(() => new Promise(() => {})); // never resolves
    const hook = await renderActive();
    let resolved: unknown = "unset";
    await act(() => {
      const p = hook.current.evaluate("search the web").then((r) => {
        resolved = r;
      });
      return vi.advanceTimersByTimeAsync(1100).then(() => p);
    });
    expect(resolved).toBeNull();
  }
  finally {
    vi.useRealTimers();
  }
});

it("dismiss clears the shown card", async () => {
  suggest.mockResolvedValue({ name: "future-web", description: "web" });
  const hook = await renderActive();
  await act(() => hook.current.evaluate("search the web"));
  expect(hook.current.state.recommendation).not.toBeNull();
  act(() => hook.current.dismiss());
  expect(hook.current.state.recommendation).toBeNull();
});
