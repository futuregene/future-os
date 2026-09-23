import { createElement, useState } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { useRemote } from "../../../remote/RemoteContext";
import { draftPicksSkill, utf8Length, useSkillRecommendation, type SkillRecommendationApi } from "../useSkillRecommendation";
import { messageHash } from "../skillRecoBudget";

jest.mock("../../../../src/remote/RemoteContext", () => ({ useRemote: jest.fn() }), { virtual: true });
jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("@react-native-async-storage/async-storage", () => {
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

const remoteMock = useRemote as jest.MockedFunction<typeof useRemote>;
const store = (AsyncStorage as unknown as { __store: Map<string, string> }).__store;
const STORAGE_KEY = "futureos.mobile.skill-reco.v1";

/** A draft comfortably past the 30-byte gate. */
const DRAFT = "please search the web for this thing";

function remote(overrides: Record<string, unknown> = {}) {
  return {
    suggestSkill: jest.fn(async () => null),
    listAvailableSkills: jest.fn(async () => [
      { id: "future-web", description: "search the web", latestVersion: "1.0" },
      { id: "future-paper", description: "find papers", latestVersion: "2.0" },
    ]),
    listInstalledSkills: jest.fn(async () => [{ id: "future-paper" }]),
    installSkill: jest.fn(async () => undefined),
    ...overrides,
  } as unknown as ReturnType<typeof useRemote>;
}

function mount(api: { current: SkillRecommendationApi | null }, enabled = true, online = true) {
  function Harness() {
    api.current = useSkillRecommendation(enabled, online);
    const [, force] = useState(0);
    (api as { rerender?: () => void }).rerender = () => force(value => value + 1);
    return null;
  }
  let tree!: ReactTestRenderer;
  act(() => {
    tree = create(createElement(Harness));
  });
  return tree;
}

beforeEach(() => {
  jest.clearAllMocks();
  store.clear();
  remoteMock.mockReturnValue(remote());
});

/** Evaluate and flush the state update, returning the answer. */
async function evaluate(api: { current: SkillRecommendationApi | null }, draft: string) {
  let answer = false;
  await act(async () => {
    answer = await api.current!.evaluate(draft);
  });
  return answer;
}

describe("the trigger gates", () => {
  it("asks only for real messages inside the length window", async () => {
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      // The long draft is askable; the recommender declines it here (the mock
      // returns null), so the message is sent normally.
      expect(await evaluate(api, DRAFT)).toBe(false);
      expect(remoteMock()!.suggestSkill).toHaveBeenCalledTimes(1);
      // The message is a slash command, not a message to recommend for.
      expect(await evaluate(api, "/skills")).toBe(false);
      // Too short: 9 汉字 is 27 bytes, under the 30-byte gate.
      expect(await evaluate(api, "单细胞测序如何分析")).toBe(false);
      // The user already picked a skill.
      expect(await evaluate(api, "帮我查一下 /future-web 这个基因")).toBe(false);
      // Only the askable draft reaches the recommender.
      expect(remoteMock()!.suggestSkill).toHaveBeenCalledTimes(1);
    } finally {
      act(() => tree.unmount());
    }
  });

  it("does not ask when the toggle is off or the desktop is offline", async () => {
    const off: { current: SkillRecommendationApi | null } = { current: null };
    const offline: { current: SkillRecommendationApi | null } = { current: null };
    const offTree = mount(off, false, true);
    const offlineTree = mount(offline, true, false);
    try {
      expect(await off.current!.evaluate(DRAFT)).toBe(false);
      expect(await offline.current!.evaluate(DRAFT)).toBe(false);
    } finally {
      act(() => offTree.unmount());
      act(() => offlineTree.unmount());
    }
  });

  it("does not ask when the desktop has no uninstalled skills", async () => {
    remoteMock.mockReturnValue(
      remote({
        listInstalledSkills: jest.fn(async () => [{ id: "future-web" }, { id: "future-paper" }]),
      }),
    );
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      expect(await evaluate(api, DRAFT)).toBe(false);
    } finally {
      act(() => tree.unmount());
    }
  });
});

describe("showing a recommendation", () => {
  it("offers the skill and holds the draft on a hit", async () => {
    remoteMock.mockReturnValue(
      remote({ suggestSkill: jest.fn(async () => ({ name: "future-web", description: "search the web" })) }),
    );
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      expect(await evaluate(api, DRAFT)).toBe(true);
      expect(api.current!.suggestion?.skill.name).toBe("future-web");
      expect(api.current!.suggestion?.draft).toBe(DRAFT);
      // Spends the budget when shown.
      const stored = JSON.parse(store.get(STORAGE_KEY) as string);
      expect(stored.skills).toEqual(["future-web"]);
      expect(stored.messages).toEqual([messageHash(DRAFT)]);
    } finally {
      act(() => tree.unmount());
    }
  });

  it("sends normally when the desktop declines, errors, or is unreachable", async () => {
    for (const suggestSkill of [
      jest.fn(async () => null),
      jest.fn(async () => {
        throw new Error("desktop busy");
      }),
    ]) {
      remoteMock.mockReturnValue(remote({ suggestSkill }));
      const api: { current: SkillRecommendationApi | null } = { current: null };
      const tree = mount(api);
      try {
        expect(await evaluate(api, DRAFT)).toBe(false);
        expect(api.current!.suggestion).toBeNull();
        expect(store.has(STORAGE_KEY)).toBe(false);
      } finally {
        act(() => tree.unmount());
      }
    }
  });

  it("skips a skill already shown today instead of offering another", async () => {
    store.set(
      STORAGE_KEY,
      JSON.stringify({
        day: new Date().toISOString().slice(0, 10),
        skills: ["future-web"],
        messages: [],
      }),
    );
    remoteMock.mockReturnValue(
      remote({ suggestSkill: jest.fn(async () => ({ name: "future-web", description: "search the web" })) }),
    );
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      expect(await evaluate(api, DRAFT)).toBe(false);
      expect(api.current!.suggestion).toBeNull();
    } finally {
      act(() => tree.unmount());
    }
  });

  it("does not ask twice about the same message", async () => {
    const suggestSkill = jest.fn(async () => null);
    remoteMock.mockReturnValue(remote({ suggestSkill }));
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      // The first evaluation records nothing (no recommendation), so the second
      // is a genuine retry — but a message that DID produce one is skipped.
      await evaluate(api, DRAFT);
      store.set(
        STORAGE_KEY,
        JSON.stringify({ day: localDay(), skills: ["future-web"], messages: [messageHash(DRAFT)] }),
      );
      expect(await evaluate(api, DRAFT)).toBe(false);
    } finally {
      act(() => tree.unmount());
    }
  });

  it("stops asking once the day's budget is spent", async () => {
    store.set(
      STORAGE_KEY,
      JSON.stringify({ day: localDay(), skills: ["a", "b", "c"], messages: [] }),
    );
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      expect(await evaluate(api, DRAFT)).toBe(false);
      expect(remoteMock()!.suggestSkill).not.toHaveBeenCalled();
    } finally {
      act(() => tree.unmount());
    }
  });
});

describe("accepting and dismissing", () => {
  it("installs the suggestion and returns the draft with the command appended", async () => {
    remoteMock.mockReturnValue(
      remote({ suggestSkill: jest.fn(async () => ({ name: "future-web", description: "search the web" })) }),
    );
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      await evaluate(api, DRAFT);
      let composed: string | null = null;
      await act(async () => {
        composed = await api.current!.installAndUse();
      });
      expect(composed).toBe(`${DRAFT} /future-web`);
      // Installed at the catalogue's latest version, and the card is gone.
      expect(remoteMock()!.installSkill).toHaveBeenCalledWith("future-web", "1.0");
      expect(api.current!.suggestion).toBeNull();
    } finally {
      act(() => tree.unmount());
    }
  });

  it("keeps the card up when the install fails", async () => {
    remoteMock.mockReturnValue(
      remote({
        suggestSkill: jest.fn(async () => ({ name: "future-web", description: "search the web" })),
        installSkill: jest.fn(async () => {
          throw new Error("desktop offline");
        }),
      }),
    );
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      await evaluate(api, DRAFT);
      let composed: string | null = "unset";
      await act(async () => {
        composed = await api.current!.installAndUse();
      });
      expect(composed).toBeNull();
      // The user can retry or dismiss; the draft is untouched.
      expect(api.current!.suggestion?.skill.name).toBe("future-web");
    } finally {
      act(() => tree.unmount());
    }
  });

  it("dismiss clears the suggestion without installing", async () => {
    remoteMock.mockReturnValue(
      remote({ suggestSkill: jest.fn(async () => ({ name: "future-web", description: "search the web" })) }),
    );
    const api: { current: SkillRecommendationApi | null } = { current: null };
    const tree = mount(api);
    try {
      await evaluate(api, DRAFT);
      act(() => api.current!.dismiss());
      expect(api.current!.suggestion).toBeNull();
      expect(remoteMock()!.installSkill).not.toHaveBeenCalled();
    } finally {
      act(() => tree.unmount());
    }
  });
});

function localDay(now = new Date()): string {
  const pad = (value: number) => String(value).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

describe("helpers", () => {
  it("detects a picked skill only on a real slash token", () => {
    expect(draftPicksSkill("/future-web")).toBe(true);
    expect(draftPicksSkill("帮我查一下 /future-web")).toBe(true);
    expect(draftPicksSkill("帮我查一下")).toBe(false);
    expect(draftPicksSkill("/")).toBe(false);
  });

  it("measures UTF-8 bytes without TextEncoder", () => {
    expect(utf8Length("abc")).toBe(3);
    expect(utf8Length("单细胞测序如何分析")).toBe(27);
    expect(utf8Length("单细胞测序如何分析流")).toBe(30);
  });
});
