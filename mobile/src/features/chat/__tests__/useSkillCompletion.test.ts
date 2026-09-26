import { createElement, useLayoutEffect, useRef, useState } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { BackHandler, Keyboard, type TextInput } from "react-native";
import { useSkillCompletion } from "../useSkillCompletion";

let completion: ReturnType<typeof useSkillCompletion>;
let message: string;
const focus = jest.fn();
function Harness({ initial = "", enabled = true }) {
  const [text, setText] = useState(initial);
  const input = useRef({ focus } as unknown as TextInput);
  const result = useSkillCompletion(text, setText, enabled, input);
  useLayoutEffect(() => { message = text; completion = result; });
  return null;
}
let tree: ReactTestRenderer;
afterEach(() => { act(() => tree.unmount()); jest.restoreAllMocks(); });
function type(text: string, cursor = text.length) {
  act(() => { completion.onChangeText(text); completion.onSelectionChange({ start: cursor, end: cursor }); });
}

test("typed and button slash use the same completion, preserving surrounding text", () => {
  act(() => { tree = create(createElement(Harness, { initial: "你好 世界" })); });
  act(() => completion.onSelectionChange({ start: 3, end: 3 }));
  act(() => completion.insertSlash());
  expect(message).toBe("你好 / 世界");
  expect(completion.query?.query).toBe("");
  expect(focus).toHaveBeenCalled();
  act(() => completion.insertSlash());
  expect(message).toBe("你好 / 世界");
  type("你好 /研究 世界", 6);
  expect(completion.query?.query).toBe("研究");
  act(() => completion.select("future-web"));
  expect(message).toBe("你好 /future-web 世界");
  expect(completion.selection).toEqual({ start: 15, end: 15 });
  expect(completion.query).toBeNull();
});

test("dismissal preserves draft; further typing or a button press reopens suggestions", () => {
  act(() => { tree = create(createElement(Harness, {})); });
  act(() => completion.onFocus());
  type("/re");
  expect(completion.query?.query).toBe("re");
  act(() => completion.close());
  expect(message).toBe("/re");
  expect(completion.query).toBeNull();
  act(() => completion.onSelectionChange({ start: 3, end: 3 }));
  expect(completion.query).toBeNull();
  type("/research");
  expect(completion.query?.query).toBe("research");
  act(() => completion.onBlur());
  expect(completion.query).toBeNull();
  act(() => completion.insertSlash());
  expect(message).toBe("/research");
  expect(completion.query).not.toBeNull();
});

test("typing, deleting and pasting filter without waiting for native selection events", () => {
  act(() => { tree = create(createElement(Harness, {})); });
  act(() => completion.onFocus());
  for (const text of ["/", "/w", "/web", "/we", "/研究", "/research"]) {
    act(() => completion.onChangeText(text));
    expect(completion.query?.query).toBe(text.slice(1));
    expect(completion.inputSelection).toBeUndefined();
  }
  act(() => completion.onChangeText("/research "));
  expect(completion.query).toBeNull();
  act(() => completion.onChangeText(""));
  expect(completion.query).toBeNull();
});

test("edits in the middle preserve the suffix and respect subsequent caret movement", () => {
  act(() => { tree = create(createElement(Harness, { initial: "你好 /ww 后文" })); });
  act(() => completion.onFocus());
  act(() => completion.onSelectionChange({ start: 5, end: 5 }));
  act(() => completion.onChangeText("你好 /www 后文"));
  expect(completion.query?.query).toBe("ww");
  expect(completion.selection).toEqual({ start: 6, end: 6 });
  act(() => completion.onSelectionChange({ start: 4, end: 7 }));
  expect(completion.query).toBeNull();
  act(() => completion.onChangeText("你好 /web 后文"));
  expect(completion.query?.query).toBe("web");
  act(() => completion.select("future-web"));
  expect(message).toBe("你好 /future-web 后文");
});

test("selection arriving before text also produces the current query", () => {
  act(() => { tree = create(createElement(Harness, { initial: "/" })); });
  act(() => completion.onFocus());
  act(() => completion.onSelectionChange({ start: 4, end: 4 }));
  act(() => completion.onChangeText("/web"));
  expect(completion.query?.query).toBe("web");
});

test("disabled composer cannot insert or select skills", () => {
  act(() => { tree = create(createElement(Harness, {})); });
  act(() => completion.insertSlash());
  expect(completion.query).not.toBeNull();
  act(() => tree.update(createElement(Harness, { enabled: false })));
  act(() => { completion.select("web"); completion.insertSlash(); });
  expect(message).toBe("/");
  expect(completion.query).toBeNull();
});

test("Android Back closes suggestions first; IME dismissal also closes them", () => {
  let back!: Parameters<typeof BackHandler.addEventListener>[1];
  let hide!: () => void;
  const remove = jest.fn();
  jest.spyOn(BackHandler, "addEventListener").mockImplementation((_, callback) => { back = callback; return { remove }; });
  jest.spyOn(Keyboard, "addListener").mockImplementation((_, callback) => {
    hide = callback as () => void;
    return { remove } as unknown as ReturnType<typeof Keyboard.addListener>;
  });
  act(() => { tree = create(createElement(Harness, {})); });
  act(() => completion.insertSlash());
  act(() => { expect(back({ type: "hardwareBackPress", timeStamp: 1 })).toBe(true); });
  expect(completion.query).toBeNull();
  expect(message).toBe("/");
  expect(remove).toHaveBeenCalled();
  act(() => completion.insertSlash());
  act(() => hide());
  expect(completion.query).toBeNull();
});

test("an edit that keeps a common tail places the caret after it", () => {
  // A paste/replace whose new text shares a trailing run with the draft (the
  // common JS case: rewriting the middle of a line). Nothing about the prefix
  // matches, so the caret is derived from the shared tail.
  act(() => { tree = create(createElement(Harness, { initial: "abcdef" })); });
  act(() => completion.onSelectionChange({ start: 0, end: 0 }));
  act(() => completion.onChangeText("xyzcdef"));
  expect(message).toBe("xyzcdef");
  expect(completion.selection).toEqual({ start: 3, end: 3 });
  // The tail was consumed, not the whole string: a full-common-tail read would
  // put the caret at the start of the new text.
  expect(completion.selection.start).not.toBe(0);
  expect(completion.selection.start).not.toBe(7);
});

test("an edit with nothing in common places the caret at the end", () => {
  act(() => { tree = create(createElement(Harness, { initial: "abcdef" })); });
  act(() => completion.onSelectionChange({ start: 0, end: 0 }));
  act(() => completion.onChangeText("zzz"));
  expect(message).toBe("zzz");
  expect(completion.selection).toEqual({ start: 3, end: 3 });
});

test("a slash action with no open query is ignored instead of clearing the draft", () => {
  const onAction = jest.fn();
  function ActionHarness() {
    const [text, setText] = useState("");
    const input = useRef({ focus } as unknown as TextInput);
    const result = useSkillCompletion(text, setText, true, input, onAction);
    useLayoutEffect(() => { message = text; completion = result; });
    return null;
  }
  act(() => { tree = create(createElement(ActionHarness)); });
  // No query yet: the button must not run an action against a half-typed draft.
  act(() => completion.runAction({ id: "compact", label: "Compact", insert: "" } as never));
  expect(onAction).not.toHaveBeenCalled();
  expect(message).toBe("");

  // With a query open the action runs and the typed command is removed.
  act(() => completion.insertSlash());
  act(() => completion.onChangeText("/compact"));
  expect(completion.query?.query).toBe("compact");
  act(() => completion.runAction({ id: "compact", label: "Compact", insert: "" } as never));
  expect(onAction).toHaveBeenCalledTimes(1);
  expect(message).toBe("");
});

