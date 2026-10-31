import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Keyboard, Platform } from "react-native";
import { useKeyboardHeight } from "../useKeyboardHeight";

/**
 * The keyboard's height on Android, as a screen that scrolls itself needs it.
 *
 * Android runs this app edge-to-edge, where the window no longer shrinks for the
 * keyboard and `KeyboardAvoidingView` measures from the wrong origin — so the
 * only way a form's last row stays reachable is if the screen reserves the space
 * itself. That means the listener has to report the platform's own number, and
 * iOS, which is handled by the same view, must get nothing to double-count.
 */
describe("useKeyboardHeight", () => {
  const originalOs = Platform.OS;
  let renderer: ReactTestRenderer | null = null;
  let height = -1;
  const listeners = new Map<string, (event: { endCoordinates: { height: number } }) => void>();
  const remove = jest.fn((name: string) => listeners.delete(name));

  function Harness(): null {
    height = useKeyboardHeight();
    return null;
  }

  beforeEach(() => {
    listeners.clear();
    remove.mockClear();
    jest.spyOn(Keyboard, "addListener").mockImplementation(((
      name: string,
      callback: (event: { endCoordinates: { height: number } }) => void,
    ) => {
      listeners.set(name, callback);
      return { remove: () => remove(name) };
    }) as never);
  });

  afterEach(() => {
    if (renderer) act(() => renderer!.unmount());
    renderer = null;
    (Platform as { OS: string }).OS = originalOs;
    jest.restoreAllMocks();
  });

  function mount(): void {
    act(() => { renderer = create(createElement(Harness)); });
  }

  test("follows an Android keyboard up and back down", () => {
    (Platform as { OS: string }).OS = "android";
    mount();
    expect(height).toBe(0);

    act(() => listeners.get("keyboardDidShow")!({ endCoordinates: { height: 312 } }));
    expect(height).toBe(312);

    act(() => listeners.get("keyboardDidHide")!({ endCoordinates: { height: 0 } }));
    expect(height).toBe(0);
  });

  test("listens nowhere on iOS, and unsubscribes on the way out", () => {
    (Platform as { OS: string }).OS = "ios";
    mount();
    // iOS gets its inset from `KeyboardAvoidingView`: a second subscription here
    // would subtract the keyboard twice.
    expect(Keyboard.addListener).not.toHaveBeenCalled();
    expect(height).toBe(0);

    act(() => renderer!.unmount());
    renderer = null;
    (Platform as { OS: string }).OS = "android";
    mount();
    expect(listeners.size).toBe(2);
    act(() => renderer!.unmount());
    renderer = null;
    // Both listeners go with the screen that registered them.
    expect(remove).toHaveBeenCalledWith("keyboardDidShow");
    expect(remove).toHaveBeenCalledWith("keyboardDidHide");
    expect(listeners.size).toBe(0);
  });
});
