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

test("an action may hand over to another menu by leaving its token behind", () => {
  // The `/` menu is how the phone reaches the `#` one: tapping "Reference a
  // conversation" must leave `#` in the draft, because that token is what opens
  // the conversation list. It must also keep the menu open — dismissing would
  // close the list before it was ever shown.
  const onAction = jest.fn();
  function Harness() {
    const [text, setText] = useState("");
    const input = useRef({ focus } as unknown as TextInput);
    const result = useSkillCompletion(text, setText, true, input, onAction);
    useLayoutEffect(() => { message = text; completion = result; });
    return null;
  }
  act(() => { tree = create(createElement(Harness)); });
  act(() => completion.onFocus());
  type("/ref");
  expect(completion.query?.query).toBe("ref");
  act(() => completion.runAction({
    id: "reference",
    label: "Reference a conversation",
    description: "",
    searchText: "#",
    insert: "#",
  }));
  expect(message).toBe("#");
  expect(completion.query).toBeNull();
  expect(completion.session?.query).toBe("");
  expect(onAction).toHaveBeenCalledTimes(1);
  // The caret sits after the token, ready to narrow the conversation list.
  expect(completion.selection).toEqual({ start: 1, end: 1 });
});

test("an action that leaves a token keeps the surrounding draft", () => {
  function Harness() {
    const [text, setText] = useState("看一下 ");
    const input = useRef({ focus } as unknown as TextInput);
    const result = useSkillCompletion(text, setText, true, input);
    useLayoutEffect(() => { message = text; completion = result; });
    return null;
  }
  act(() => { tree = create(createElement(Harness)); });
  act(() => completion.onFocus());
  act(() => completion.onChangeText("看一下 /ref"));
  act(() => completion.runAction({ id: "reference", label: "", description: "", searchText: "", insert: "#" }));
  expect(message).toBe("看一下 #");
  expect(completion.selection).toEqual({ start: 5, end: 5 });
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

test("typed # opens the conversation query and inserting one writes the reference token", () => {
  const remember = jest.fn();
  const refs = {};
  function Harness() {
    const [text, setText] = useState("");
    const input = useRef({ focus } as unknown as TextInput);
    const result = useSkillCompletion(text, setText, true, input, undefined, { refs, remember });
    useLayoutEffect(() => { message = text; completion = result; });
    return null;
  }
  act(() => { tree = create(createElement(Harness)); });
  act(() => completion.onFocus());
  type("ask #fla");
  expect(completion.session?.query).toBe("fla");
  expect(completion.query).toBeNull();
  act(() => completion.selectSession({ sessionId: "s-1", title: "Fix the flaky test" }));
  // The draft shows what the desktop's pill shows; the id is kept aside for the
  // send (a phone TextInput cannot style part of its text).
  expect(message).toBe("ask #Fix the flaky test ");
  expect(remember).toHaveBeenCalledWith("#Fix the flaky test", { sessionId: "s-1", title: "Fix the flaky test" });
  expect(completion.selection).toEqual({ start: 24, end: 24 });
  expect(completion.session).toBeNull();
  expect(focus).toHaveBeenCalled();
});

test("a second conversation sharing a title gets its own token", () => {
  // Without the disambiguated token the map would hold one entry for two
  // conversations and the send would expand both to the same one.
  const first = { sessionId: "s-1", title: "未命名" };
  const refs = { "#未命名": first };
  const remember = jest.fn();
  function Harness() {
    const [text, setText] = useState("#未命名 ");
    const input = useRef({ focus } as unknown as TextInput);
    const result = useSkillCompletion(text, setText, true, input, undefined, { refs, remember });
    useLayoutEffect(() => { message = text; completion = result; });
    return null;
  }
  act(() => { tree = create(createElement(Harness)); });
  act(() => completion.onFocus());
  act(() => completion.onChangeText("看到 #未命名 和 #未"));
  expect(completion.session?.query).toBe("未");
  act(() => completion.selectSession({ sessionId: "s-2", title: "未命名" }));
  expect(message).toBe("看到 #未命名 和 #未命名·s-2 ");
  expect(remember).toHaveBeenCalledWith("#未命名·s-2", { sessionId: "s-2", title: "未命名" });
});

test("a `#` that is not opening a token stays literal", () => {
  act(() => { tree = create(createElement(Harness, {})); });
  act(() => completion.onFocus());
  type("## heading");
  expect(completion.session).toBeNull();
  type("issue#12");
  expect(completion.session).toBeNull();
  // Neither trigger accepts a token containing the other's sigil, so `#/`
  // opens no menu at all rather than guessing which one was meant.
  type("x #/");
  expect(completion.query).toBeNull();
  expect(completion.session).toBeNull();
  type("x #/f");
  expect(completion.session).toBeNull();
});

test("closing the conversation menu keeps the draft, and a disabled composer cannot insert", () => {
  act(() => { tree = create(createElement(Harness, {})); });
  act(() => completion.onFocus());
  type("#fix");
  act(() => completion.close());
  expect(completion.session).toBeNull();
  expect(message).toBe("#fix");
  type("#fix");
  act(() => tree.update(createElement(Harness, { enabled: false })));
  act(() => completion.selectSession({ sessionId: "s-1", title: "Fix" }));
  expect(message).toBe("#fix");
});

