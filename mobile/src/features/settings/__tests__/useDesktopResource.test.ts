import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState } from "react-native";
import { useDesktopResource } from "../useDesktopResource";
import { isSkillUpgrade } from "../skillVersion";

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}

test("a late read cannot undo an invalidation, and returning to foreground rereads desktop", async () => {
  const old = deferred<number>();
  const load = jest.fn().mockReturnValueOnce(old.promise).mockResolvedValue(2);
  let state!: ReturnType<typeof useDesktopResource<number>>;
  function Host({ revision }: { revision: number }) { state = useDesktopResource(load, revision, true); return null; }
  let resume!: (state: string) => void;
  const remove = jest.fn();
  const spy = jest.spyOn(AppState, "addEventListener").mockImplementation((_type, handler) => {
    resume = handler as typeof resume;
    return { remove };
  });
  let tree!: ReactTestRenderer;
  try {
    await act(async () => { tree = create(createElement(Host, { revision: 0 })); });
    await act(async () => tree.update(createElement(Host, { revision: 1 })));
    expect(state.data).toBe(2);
    await act(async () => old.resolve(1));
    expect(state.data).toBe(2);
    load.mockResolvedValueOnce(3);
    await act(async () => resume("active"));
    expect(state.data).toBe(3);
    act(() => tree.unmount());
    expect(remove).toHaveBeenCalled();
  } finally { spy.mockRestore(); }
});

test.each([
  ["1.9", "1.10", true], ["1.2", "1.2.0", false], ["2", "1.9", false],
  [null, "1.2", false], ["", "1.2", false], ["1.0", null, false],
])("upgrade detection matches desktop: %s -> %s", (installed, latest, expected) => {
  expect(isSkillUpgrade(installed as string | null, latest as string | null)).toBe(expected);
});
