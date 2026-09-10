import AsyncStorage from "@react-native-async-storage/async-storage";
import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  parseCollapsedWorkspaces,
  rememberedCollapsedWorkspaces,
  useCollapsedWorkspaces,
} from "../useCollapsedWorkspaces";

const STORAGE_KEY = "futureos.mobile.collapsed-workspaces.v1";
const mockData = new Map<string, string>();

jest.mock("@react-native-async-storage/async-storage", () => ({
  __esModule: true,
  default: {
    getItem: jest.fn(async (key: string) => mockData.get(key) ?? null),
    setItem: jest.fn(async (key: string, value: string) => {
      mockData.set(key, value);
    }),
  },
}));

const mockedAsync = AsyncStorage as jest.Mocked<typeof AsyncStorage>;

type Hook = ReturnType<typeof useCollapsedWorkspaces>;

function mount(): { result: { current: Hook }; renderer: ReactTestRenderer } {
  const result = { current: undefined as unknown as Hook };
  function Harness(): null {
    result.current = useCollapsedWorkspaces();
    return null;
  }
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(createElement(Harness));
  });
  return { result, renderer };
}

/** Let the storage read resolve inside act(). */
async function settle(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
  });
}

describe("collapsed workspaces", () => {
  beforeEach(() => {
    mockData.clear();
    jest.clearAllMocks();
  });

  test("parses stored ids and rejects corrupt or non-string values", () => {
    expect([...parseCollapsedWorkspaces(null)]).toEqual([]);
    expect([...parseCollapsedWorkspaces("{not json")]).toEqual([]);
    expect([...parseCollapsedWorkspaces('{"ws":true}')]).toEqual([]);
    expect([...parseCollapsedWorkspaces(JSON.stringify(["ws-a", 7, null, "ws-b"]))]).toEqual([
      "ws-a",
      "ws-b",
    ]);
  });

  test("restores what a previous launch wrote", async () => {
    mockData.set(STORAGE_KEY, JSON.stringify(["ws-a"]));
    const { result, renderer } = mount();
    await settle();
    expect([...result.current.collapsed]).toEqual(["ws-a"]);
    act(() => renderer.unmount());
  });

  test("a toggle survives a remount through storage and the in-session cache", async () => {
    const first = mount();
    await settle();
    act(() => first.result.current.toggleWorkspaceCollapsed("ws-a"));
    expect([...first.result.current.collapsed]).toEqual(["ws-a"]);
    // AsyncStorage is written on the next effect flush.
    await settle();
    expect(mockData.get(STORAGE_KEY)).toBe(JSON.stringify(["ws-a"]));
    act(() => first.renderer.unmount());

    // The synchronous cache keeps the next mount correct on its very first
    // frame, before the storage read resolves — SessionList remounts on every
    // return from a conversation, and AsyncStorage cannot be read synchronously.
    expect([...rememberedCollapsedWorkspaces()]).toEqual(["ws-a"]);
    const second = mount();
    expect([...second.result.current.collapsed]).toEqual(["ws-a"]);
    await settle();
    expect([...second.result.current.collapsed]).toEqual(["ws-a"]);
    act(() => second.renderer.unmount());
  });

  test("toggling twice expands again", async () => {
    const { result, renderer } = mount();
    await settle();
    act(() => result.current.toggleWorkspaceCollapsed("ws-a"));
    act(() => result.current.toggleWorkspaceCollapsed("ws-a"));
    await settle();
    expect([...result.current.collapsed]).toEqual([]);
    expect(mockData.get(STORAGE_KEY)).toBe("[]");
    act(() => renderer.unmount());
  });

  test("never writes before hydrating, so stored folds are not erased", async () => {
    mockData.set(STORAGE_KEY, JSON.stringify(["ws-a"]));
    mount();
    // The first effect pass persists only after the read has landed.
    expect(mockedAsync.setItem).not.toHaveBeenCalled();
    await settle();
    expect(mockData.get(STORAGE_KEY)).toBe(JSON.stringify(["ws-a"]));
  });

  test("corrupt storage starts fully expanded instead of throwing", async () => {
    mockData.set(STORAGE_KEY, "{not json");
    const { result, renderer } = mount();
    await settle();
    expect([...result.current.collapsed]).toEqual([]);
    act(() => renderer.unmount());
  });

  test("a toggle during hydration wins over the stored value", async () => {
    mockData.set(STORAGE_KEY, JSON.stringify(["ws-a"]));
    const { result, renderer } = mount();
    act(() => result.current.toggleWorkspaceCollapsed("ws-b"));
    await settle();
    expect([...result.current.collapsed]).toEqual(["ws-b"]);
    act(() => renderer.unmount());
  });
});
