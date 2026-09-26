import { createElement, useState, type ComponentProps } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ComposerDock } from "../components/ComposerDock";
import { useCompactContext } from "../useCompactContext";
import { emptyTimeline } from "../../../remote/timeline";
import type { CompactionOutcome } from "../../../remote/types";
import { SkillPicker } from "../components/SkillPicker";
import { SkillSuggestionCard } from "../components/SkillSuggestionCard";
import { SkillDetailsDialog } from "../components/SkillDetailsDialog";
import { PendingApprovalCard } from "../../../components/TimelineCard";
import { deleteTemporaryAttachment } from "../../../remote/files";
import { resources } from "../../../i18n/locales";
import { CircleAlert } from "lucide-react-native";
import { StyleSheet, Text, TextInput, View } from "react-native";

let mockDimensions = { width: 390, height: 844, scale: 1, fontScale: 1 };
jest.mock("react-native", () => {
  const actual = jest.requireActual("react-native");
  return new Proxy(actual, {
    get: (target, key) => key === "useWindowDimensions" ? () => mockDimensions : Reflect.get(target, key),
  });
});

jest.mock("lucide-react-native", () => Object.fromEntries(["ArrowDown", "ChevronDown", "CircleAlert", "FileText", "Info", "Lightbulb", "Paperclip", "Send", "Slash", "Square", "X"].map(name => [name, () => null])));
// The composer takes `t` as a prop, but the details dialog and the surfaces
// below it call `useTranslation` themselves. Standing up the real i18n module
// would drag expo-localization into a component test; the translations are
// covered by src/i18n/__tests__, so here they only need to not throw.
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }),
}));
// The details dialog renders a full-screen surface, which reads the safe-area
// insets. Outside a provider that hook throws, so use the package's own mock.
jest.mock("react-native-safe-area-context", () =>
  jest.requireActual("react-native-safe-area-context/jest/mock").default);
jest.mock("../../../components/TimelineCard", () => ({ PendingApprovalCard: jest.fn(() => null) }));
jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("../../../remote/files", () => ({ deleteTemporaryAttachment: jest.fn() }));
// The picker itself is replaced, but its sizing helpers are the real ones: the
// composer's height budget is what's under test here.
jest.mock("../components/SkillPicker", () => ({
  ...jest.requireActual("../components/SkillPicker"),
  SkillPicker: jest.fn(() => null),
}));
test("streaming allows drafting while one stop press sends a request and exposes its outcome", async () => {
  let resolve!: () => void;
  const abort = jest.fn(() => new Promise<void>(done => { resolve = done; }));
  const info = jest.spyOn(console, "info").mockImplementation(() => {});
  const props = {
    message: "", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: false, selectedSessionId: "s1", desktopOnline: true, connectionPresentation: { level: "connected" }, models: [], modelId: "model", streaming: true, abort },
    openAttachmentMenu: jest.fn(), send: jest.fn(), atLatest: true, scrollToLatest: jest.fn(),
    pendingApprovals: [], approvalSubmitting: null, approvalError: null,
    decideApproval: jest.fn(), selector: null, setSelector: jest.fn(),
  } as unknown as ComponentProps<typeof ComposerDock>;
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(ComposerDock, props)); });
  const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0]!;
  try {
    expect(tree.root.findByType(TextInput).props.editable).toBe(true);
    expect(button("chat.stop").props.disabled).toBe(false);
    act(() => button("chat.stop").props.onPress());
    expect(abort).toHaveBeenCalledTimes(1);
    expect(button("chat.stopping").props.disabled).toBe(true);
    expect(button("chat.stopping").props.accessibilityState.busy).toBe(true);
    for (let i = 0; i < 100; i++) {
      act(() => tree.update(createElement(ComposerDock, { ...props, pendingApprovals: [] })));
    }
    expect(abort).toHaveBeenCalledTimes(1);
    await act(async () => resolve());
    expect(button("chat.stop").props.disabled).toBe(false);
    expect(tree.root.findAllByType(Text).some(node => node.props.children === "chat.stopRequested")).toBe(true);
    abort.mockRejectedValueOnce(new Error("not_connected"));
    await act(async () => button("chat.stop").props.onPress());
    expect(tree.root.findAllByType(Text).some(node => node.props.children === "chat.stopFailed")).toBe(true);
    expect(button("chat.stop").props.disabled).toBe(false);
    act(() => tree.update(createElement(ComposerDock, { ...props, remote: { ...props.remote, streaming: false } })));
    expect(tree.root.findAllByType(Text).some(node => node.props.children === "chat.stopFailed")).toBe(false);
  } finally {
    act(() => tree.unmount());
    info.mockRestore();
  }
});

test("the recommendation wait locks the input and spins the send button", () => {
  const send = jest.fn(async () => {});
  const props = {
    message: "查一下这个基因在人群中的频率", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: false, selectedSessionId: "s1", desktopOnline: true,
      connectionPresentation: { level: "connected" }, models: [], modelId: "model",
      streaming: false, busy: false, abort: jest.fn() },
    openAttachmentMenu: jest.fn(), send, atLatest: true, scrollToLatest: jest.fn(),
    pendingApprovals: [], approvalSubmitting: null, approvalError: null,
    decideApproval: jest.fn(), selector: null, setSelector: jest.fn(),
  } as unknown as ComponentProps<typeof ComposerDock>;
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(ComposerDock, { ...props, skillEvaluating: true })); });
  const input = () => tree.root.findByType(TextInput);
  const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0];
  try {
    // The message that goes out must be the one being evaluated, so drafting is
    // closed too — and the send button says why instead of looking dead.
    expect(input().props.editable).toBe(false);
    expect(button("chat.send")).toBeUndefined();
    const evaluating = button("chat.findingSkill")!;
    expect(evaluating.props.disabled).toBe(true);
    expect(evaluating.props.accessibilityState.busy).toBe(true);
    act(() => evaluating.props.onPress());
    act(() => input().props.onSubmitEditing());
    expect(send).not.toHaveBeenCalled();

    act(() => tree.update(createElement(ComposerDock, { ...props, skillEvaluating: false })));
    expect(input().props.editable).toBe(true);
    expect(button("chat.send")!.props.disabled).toBe(false);
    act(() => button("chat.send")!.props.onPress());
    expect(send).toHaveBeenCalledTimes(1);
  } finally {
    act(() => tree.unmount());
  }
});

test("streaming keeps the next draft through updates and completion without submitting it", () => {
  const send = jest.fn(async () => {});
  const props = {
    attachments: [], setAttachments: jest.fn(), supportsImages: true,
    activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: false, selectedSessionId: "s1", desktopOnline: true,
      connectionPresentation: { level: "connected" }, models: [], modelId: "model",
      streaming: true, busy: false, abort: jest.fn() },
    openAttachmentMenu: jest.fn(), send, atLatest: true, scrollToLatest: jest.fn(),
    pendingApprovals: [], approvalSubmitting: null, approvalError: null,
    decideApproval: jest.fn(), selector: null, setSelector: jest.fn(),
  } as unknown as ComponentProps<typeof ComposerDock>;
  function Harness({ streaming, busy = false, desktopOnline = true }: { streaming: boolean; busy?: boolean; desktopOnline?: boolean }) {
    const [message, setMessage] = useState("");
    return createElement(ComposerDock, {
      ...props, message, setMessage,
      remote: { ...props.remote, streaming, busy, desktopOnline },
    });
  }
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(Harness, { streaming: true })); });
  const input = () => tree.root.findByType(TextInput);
  const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0];
  try {
    const originalInput = input();
    expect(input().props.editable).toBe(true);
    act(() => input().props.onChangeText("下一条消息\n  保留空格 "));
    act(() => input().props.onSubmitEditing());
    expect(send).not.toHaveBeenCalled();
    expect(button("chat.send")).toBeUndefined();
    expect(button("chat.stop")).toBeDefined();
    for (let i = 0; i < 20; i++) {
      act(() => tree.update(createElement(Harness, { streaming: true })));
    }
    act(() => tree.update(createElement(Harness, { streaming: false })));
    expect(input()).toBe(originalInput);
    expect(input().props.value).toBe("下一条消息\n  保留空格 ");
    expect(send).not.toHaveBeenCalled();
    expect(button("chat.send")!.props.disabled).toBe(false);
    act(() => button("chat.send")!.props.onPress());
    expect(send).toHaveBeenCalledTimes(1);
    act(() => input().props.onSubmitEditing());
    expect(send).toHaveBeenCalledTimes(2);
    for (const state of [{ busy: true }, { desktopOnline: false }]) {
      act(() => tree.update(createElement(Harness, { streaming: false, ...state })));
      expect(button("chat.send")!.props.disabled).toBe(true);
      act(() => input().props.onSubmitEditing());
      expect(send).toHaveBeenCalledTimes(2);
      expect(input().props.value).toBe("下一条消息\n  保留空格 ");
    }
  } finally { act(() => tree.unmount()); }
});

test("compaction blocks button and keyboard sends without clearing or locking the draft", () => {
  const send = jest.fn(async () => {});
  const props = {
    message: "next message", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: false, selectedSessionId: "s1", desktopOnline: true,
      connectionPresentation: { level: "connected" }, models: [], modelId: "model",
      streaming: false, compacting: true, busy: false, abort: jest.fn() },
    openAttachmentMenu: jest.fn(), send, atLatest: true, scrollToLatest: jest.fn(),
    pendingApprovals: [], approvalSubmitting: null, approvalError: null,
    decideApproval: jest.fn(), selector: null, setSelector: jest.fn(),
  } as unknown as ComponentProps<typeof ComposerDock>;
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(ComposerDock, props)); });
  const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!;
  try {
    expect(tree.root.findByType(TextInput).props.editable).toBe(true);
    expect(button("chat.compacting").props.disabled).toBe(true);
    act(() => {
      button("chat.compacting").props.onPress();
      tree.root.findByType(TextInput).props.onSubmitEditing();
    });
    expect(send).not.toHaveBeenCalled();
    expect(props.setMessage).not.toHaveBeenCalled();
    act(() => tree.update(createElement(ComposerDock, { ...props, remote: { ...props.remote, compacting: false } })));
    expect(tree.root.findByType(TextInput).props.value).toBe("next message");
    expect(button("chat.send").props.disabled).toBe(false);
    act(() => button("chat.send").props.onPress());
    expect(send).toHaveBeenCalledTimes(1);
  } finally { act(() => tree.unmount()); }
});

test("manual compaction is a slash action, gated by the Desktop and never a toolbar button", () => {
  const onCompactContext = jest.fn();
  const props = {
    message: "/", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: false, selectedSessionId: "s1", desktopOnline: true,
      connectionPresentation: { level: "connected" }, models: [], modelId: "model",
      streaming: false, compacting: false, busy: false, abort: jest.fn(),
      capabilities: new Set(["skills_v1", "compaction_v1"]), listSkills: jest.fn(async () => []) },
    openAttachmentMenu: jest.fn(), send: jest.fn(), atLatest: true, scrollToLatest: jest.fn(),
    pendingApprovals: [], approvalSubmitting: null, approvalError: null,
    decideApproval: jest.fn(), selector: null, setSelector: jest.fn(),
    onCompactContext,
  } as unknown as ComponentProps<typeof ComposerDock>;
  const picker = jest.requireMock("../components/SkillPicker").SkillPicker as jest.Mock;
  let tree!: ReactTestRenderer;
  const openMenu = () => {
    act(() => tree.root.findByType(TextInput).props.onFocus());
    // A real caret sits after the typed token; running an action clears the
    // draft, so the caret must be put back for the menu to reopen.
    act(() => tree.root.findByType(TextInput).props.onSelectionChange({ nativeEvent: { selection: { start: 1, end: 1 } } }));
    act(() => tree.root.findByType(TextInput).props.onChangeText("/"));
  };
  const actions = () => picker.mock.calls.at(-1)?.[0].actions ?? [];
  const runAction = () => picker.mock.calls.at(-1)?.[0].onActionSelect;
  try {
    act(() => { tree = create(createElement(ComposerDock, props)); });
    openMenu();
    expect(actions()).toHaveLength(1);
    expect(actions()[0]).toMatchObject({ id: "compact", label: "chat.compactContext" });
    act(() => runAction()(actions()[0]));
    expect(onCompactContext).toHaveBeenCalledTimes(1);
    // The typed command is a control, not draft text: running the action must
    // remove it, or the next send would post "/压缩" as a message.
    expect(props.setMessage).toHaveBeenLastCalledWith("");

    // Each gate is checked on a freshly opened menu: running the action closed
    // it (and cleared the draft), so the picker's last props are stale.
    for (const remote of [
      { capabilities: new Set(["skills_v1"]) },
      { compacting: true },
      { streaming: true },
    ]) {
      act(() => tree.update(createElement(ComposerDock, {
        ...props,
        remote: { ...props.remote, ...remote },
      })));
      openMenu();
      expect(actions()).toHaveLength(0);
    }
  } finally { act(() => tree.unmount()); }
});

test.each(["committed", "failed", "unchanged", "timeout", "unobserved"] as const)("real slash action locks sending before ACK, preserves the draft and releases on %s", async status => {
  let acknowledge!: (value: { sessionId: string; operationId: string }) => void;
  let finish!: (outcome: CompactionOutcome) => void;
  const ack = new Promise<{ sessionId: string; operationId: string }>(done => { acknowledge = done; });
  const terminal = new Promise<CompactionOutcome>(done => { finish = done; });
  const compact = jest.fn(() => ack);
  const send = jest.fn(async () => {});
  const remote = {
    selectedSessionId: "s1", draft: false, streaming: false, compacting: false, busy: false, desktopOnline: true,
    connectionPresentation: { level: "connected" }, models: [], modelId: "model", timeline: emptyTimeline(),
    credentials: { pairId: "pair" }, presence: { bridgeInstanceId: "bridge" },
    capabilities: new Set(["skills_v1", "compaction_v1"]), listSkills: jest.fn(async () => []),
    compactContext: compact, awaitCompactionOutcome: jest.fn(() => terminal), abort: jest.fn(),
  } as unknown as Parameters<typeof useCompactContext>[0];
  const props = {
    attachments: [], setAttachments: jest.fn(), supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote, openAttachmentMenu: jest.fn(), send, atLatest: true, scrollToLatest: jest.fn(),
    pendingApprovals: [], approvalSubmitting: null, approvalError: null, decideApproval: jest.fn(),
    selector: null, setSelector: jest.fn(),
  } as unknown as ComponentProps<typeof ComposerDock>;
  function Harness() {
    const [message, setMessage] = useState("keep /压缩 next");
    const operation = useCompactContext(remote, props.t);
    return createElement(ComposerDock, { ...props, message, setMessage, onCompactContext: operation.compact, compactionPending: operation.pending });
  }
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(Harness)); });
  const input = () => tree.root.findByType(TextInput);
  const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!;
  try {
    act(() => input().props.onFocus());
    act(() => input().props.onSelectionChange({ nativeEvent: { selection: { start: 8, end: 8 } } }));
    const picker = tree.root.findByType(SkillPicker);
    act(() => picker.props.onActionSelect(picker.props.actions[0]));
    expect(compact).toHaveBeenCalledTimes(1);
    expect(input().props.value).toBe("keep next");
    expect(input().props.editable).toBe(true);
    expect(button("chat.compacting").props.disabled).toBe(true);
    act(() => { button("chat.compacting").props.onPress(); input().props.onSubmitEditing(); });
    expect(send).not.toHaveBeenCalled();
    await act(async () => acknowledge({ sessionId: "s1", operationId: "cmp" }));
    expect(button("chat.compacting").props.disabled).toBe(true);
    const outcome: CompactionOutcome = status === "unchanged" ? { status, alreadyCompacted: true, reused: true } : { status };
    await act(async () => finish(outcome));
    expect(button("chat.send").props.disabled).toBe(false);
    expect(input().props.value).toBe("keep next");
    act(() => button("chat.send").props.onPress());
    expect(send).toHaveBeenCalledTimes(1);
  } finally { act(() => tree.unmount()); }
});

test("only the approval which failed receives the error", () => {
  type Props = ComponentProps<typeof ComposerDock>;
  const props = {
    message: "", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: false, desktopOnline: true, connectionPresentation: { customerState: "connected" }, models: [], modelId: "model", streaming: false },
    openAttachmentMenu: jest.fn(), send: jest.fn(), atLatest: true, scrollToLatest: jest.fn(),
    showOffline: false, pendingApprovals: ["a", "b"].map(id => ({
      id, kind: "approval", payload: { approval_request_id: id },
    })), approvalSubmitting: null, approvalError: { id: "a", message: "failed" },
    decideApproval: jest.fn(), setComposerHeight: jest.fn(), keyboardLift: 0,
    selector: null, setSelector: jest.fn(),
  } as unknown as Props;
  let renderer: ReactTestRenderer;
  act(() => { renderer = create(createElement(ComposerDock, props)); });
  try {
    const cards = renderer!.root.findAllByType(PendingApprovalCard);
    expect(cards).toHaveLength(2);
    expect(cards[0]!.props.error).toBe("failed");
    expect(cards[1]!.props.error).toBeNull();

    // Transcript text updates recreate the filtered array, not the approval
    // objects. They must not re-render the input/docked cards 100 times.
    const card = PendingApprovalCard as jest.Mock;
    const initialCalls = card.mock.calls.length;
    for (let i = 0; i < 100; i++) {
      act(() => renderer!.update(createElement(ComposerDock, {
        ...props, pendingApprovals: [...props.pendingApprovals],
      })));
    }
    expect(card).toHaveBeenCalledTimes(initialCalls);

    const decide = jest.fn(async () => {});
    act(() => renderer!.update(createElement(ComposerDock, {
      ...props, decideApproval: decide, remote: { ...props.remote, streaming: true },
    })));
    expect(card).toHaveBeenCalledTimes(initialCalls + 2);
    act(() => renderer!.root.findAllByType(PendingApprovalCard)[0]!.props.onDecision("approved"));
    expect(decide).toHaveBeenCalledWith("a", "approved");
  } finally {
    act(() => renderer!.unmount());
  }
});

test.each([
  [320, 1], [375, 1], [390, 1], [430, 1], [820, 1], [320, 1.6], [430, 1.6],
])("composer keeps all controls on one toolbar at width %i / font scale %f", (width, fontScale) => {
  mockDimensions = { width, fontScale, height: 844, scale: 1 };
  const send = jest.fn(async () => {});
  const setSelector = jest.fn();
  const props = {
    message: "Hello", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "A very long model label", t: (key: string) => key,
    remote: { draft: true, desktopOnline: true, connectionPresentation: { customerState: "connected" }, models: [], modelId: "model", thinkingLevel: "high", streaming: false, fileTransferSupported: true },
    openAttachmentMenu: jest.fn(), send, atLatest: true, scrollToLatest: jest.fn(),
    showOffline: false, pendingApprovals: [], approvalSubmitting: null, approvalError: null,
    decideApproval: jest.fn(), selector: null, setSelector,
  } as unknown as ComponentProps<typeof ComposerDock>;
  let tree: ReactTestRenderer;
  act(() => { tree = create(createElement(ComposerDock, props)); });
  try {
    const button = (label: string) => tree!.root.findAll(node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0]!;
    for (const label of ["chat.send", "attachment.add"]) {
      const control = button(label);
      expect(StyleSheet.flatten(control.props.style({ pressed: false }))).toMatchObject({ width: 44, height: 44 });
    }
    const selectors = tree!.root.findAllByType(View).find(node =>
      StyleSheet.flatten(node.props.style)?.flexBasis === "100%",
    );
    expect(selectors).toBeUndefined();
    expect(tree!.root.findAllByType(View).some(node => StyleSheet.flatten(node.props.style)?.flexWrap === "wrap")).toBe(false);
    const slash = button("skills.choose");
    expect(StyleSheet.flatten(slash.props.style({ pressed: false }))).toMatchObject({ width: 32, height: 44 });
    expect(slash.props.hitSlop).toEqual({ left: 6, right: 6 });
    const model = button("chat.model: A very long model label");
    expect(StyleSheet.flatten(model.props.style({ pressed: false })).paddingHorizontal).toBe(8);
    expect(StyleSheet.flatten(tree!.root.findByType(TextInput).props.style).paddingHorizontal).toBe(16);
    expect(model.findByType(Text).props.children).toBe("A very long model label");
    expect(button("chat.thinkingLevel: thinking.high").findByType(Text).props.children).toBe("thinking.high");
    act(() => button("chat.thinkingLevel: thinking.high").props.onPress());
    expect(setSelector).toHaveBeenCalledWith("thinking");
    act(() => model.props.onPress());
    expect(setSelector).toHaveBeenCalledWith("model");
    act(() => button("chat.send").props.onPress());
    expect(send).toHaveBeenCalledTimes(1);
  } finally {
    act(() => tree!.unmount());
    mockDimensions = { width: 390, height: 844, scale: 1, fontScale: 1 };
  }
});

test("composer grows with measured text, scrolls at its cap and shrinks when cleared", () => {
  const props = {
    message: "A restored or pasted multiline draft\n", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: true, desktopOnline: true, connectionPresentation: { customerState: "connected" }, models: [], modelId: "model", streaming: false },
    openAttachmentMenu: jest.fn(), send: jest.fn(), atLatest: true, scrollToLatest: jest.fn(),
    showOffline: false, pendingApprovals: [], approvalSubmitting: null, approvalError: null,
    decideApproval: jest.fn(), selector: null, setSelector: jest.fn(),
  } as unknown as ComponentProps<typeof ComposerDock>;
  let tree: ReactTestRenderer;
  act(() => { tree = create(createElement(ComposerDock, props)); });
  try {
    const input = () => tree!.root.findByType(TextInput);
    const height = () => StyleSheet.flatten(input().props.style).height;
    const measure = () => tree!.root.findAllByType(Text).find(node => node.props.onLayout)!;
    const layoutText = (height: number) => act(() => measure().props.onLayout({ nativeEvent: { layout: { height } } }));
    expect(height()).toBe(46);
    // Extra top clearance must also be included in multiline measurement.
    const textInsets = { paddingTop: 12, paddingBottom: 8, paddingHorizontal: 16 };
    expect(StyleSheet.flatten(input().props.style)).toMatchObject(textInsets);
    expect(StyleSheet.flatten(measure().props.style)).toMatchObject(textInsets);
    // A sentinel makes the trailing empty line measurable; it never enters the input value.
    expect(measure().props.children).toBe(`${props.message}\u200b`);
    expect(input().props.value).toBe(props.message);
    expect(measure().parent!.props.importantForAccessibility).toBe("no-hide-descendants");
    for (const size of [82, 150, 210]) {
      layoutText(size);
      expect(height()).toBe(size);
      expect(input().props.scrollEnabled).toBe(false);
    }
    layoutText(500);
    expect(height()).toBe(240);
    expect(input().props.scrollEnabled).toBe(true);

    // Rotation / narrower wrapping updates the same input rather than losing focus.
    const originalInput = input();
    mockDimensions = { ...mockDimensions, width: 320, height: 500, fontScale: 1.6 };
    act(() => tree!.update(createElement(ComposerDock, { ...props, message: "Shorter text" })));
    expect(height()).toBe(150);
    expect(input()).toBe(originalInput);
    layoutText(90);
    expect(height()).toBe(90);
    expect(input().props.scrollEnabled).toBe(false);
    act(() => tree!.update(createElement(ComposerDock, { ...props, message: "" })));
    expect(height()).toBe(46);
    expect(input().props.scrollEnabled).toBe(false);
    layoutText(20);
    expect(height()).toBe(46);
  } finally {
    act(() => tree!.unmount());
    mockDimensions = { width: 390, height: 844, scale: 1, fontScale: 1 };
  }
});

test("slash button opens above the composer and skill selection edits without sending", () => {
  const send = jest.fn();
  const listSkills = jest.fn(async () => []);
  const props = {
    attachments: [], setAttachments: jest.fn(), supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: true, desktopOnline: true, connectionPresentation: { customerState: "connected" }, models: [], modelId: "model", streaming: false, capabilities: new Set(["skills_v1"]), listSkills },
    openAttachmentMenu: jest.fn(), send, atLatest: true, scrollToLatest: jest.fn(),
    showOffline: false, pendingApprovals: [], approvalSubmitting: null, approvalError: null,
    decideApproval: jest.fn(), selector: null, setSelector: jest.fn(), keyboardHeight: 300,
  } as unknown as ComponentProps<typeof ComposerDock>;
  function Harness() {
    const [message, setMessage] = useState("Hello");
    return createElement(ComposerDock, { ...props, message, setMessage });
  }
  let tree: ReactTestRenderer;
  act(() => { tree = create(createElement(Harness)); });
  try {
    const button = () => tree!.root.findAll(node => node.props.accessibilityLabel === "skills.choose" && node.props.onPress)[0]!;
    act(() => button().props.onPress());
    const picker = tree!.root.findByType(SkillPicker);
    expect(picker.props.supported).toBe(true);
    expect(picker.props.load).toBe(listSkills);
    expect(picker.props.query).toBe("");
    expect(tree!.root.findByType(TextInput).props.value).toBe("Hello /");
    // Text updates must filter immediately, even before native selection events.
    act(() => tree!.root.findByType(TextInput).props.onChangeText("Hello /web"));
    expect(tree!.root.findByType(SkillPicker).props.query).toBe("web");
    act(() => picker.props.onSelect("future-web"));
    expect(tree!.root.findByType(TextInput).props.value).toBe("Hello /future-web ");
    expect(tree!.root.findAllByType(SkillPicker)).toHaveLength(0);
    expect(send).not.toHaveBeenCalled();
    act(() => tree!.update(createElement(ComposerDock, { ...props, message: "draft", setMessage: jest.fn(), remote: { ...props.remote, busy: true } })));
    expect(button().props.disabled).toBe(true);
    expect(tree!.root.findByType(TextInput).props.editable).toBe(false);
  } finally { act(() => tree!.unmount()); }
});

test("both languages define the history retry label", () => {
  expect(resources.en.translation.common.retry).toBe("Retry");
  expect(resources.zh.translation.common.retry).toBe("重试");
});

/** The props every case below needs; a test overrides just what it asserts. */
function baseProps(overrides: Record<string, unknown> = {}) {
  return {
    message: "", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: {
      draft: false, selectedSessionId: "s1", desktopOnline: true,
      connectionPresentation: { level: "connected" }, models: [], modelId: "model",
      streaming: false, busy: false, abort: jest.fn(), capabilities: new Set(),
    },
    openAttachmentMenu: jest.fn(), send: jest.fn(async () => {}), atLatest: true,
    scrollToLatest: jest.fn(), pendingApprovals: [], approvalSubmitting: null,
    approvalError: null, decideApproval: jest.fn(), selector: null, setSelector: jest.fn(),
    ...overrides,
  } as unknown as ComponentProps<typeof ComposerDock>;
}

function mountDock(overrides: Record<string, unknown> = {}) {
  const props = baseProps(overrides);
  let tree!: ReactTestRenderer;
  act(() => { tree = create(createElement(ComposerDock, props)); });
  const pressable = (label: string) => tree.root.findAll(node =>
    node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0];
  // The `/` menu is focus-gated (`useSkillCompletion` only computes a query
  // while the input is focused), so a test that expects the picker has to put
  // the caret in the field first — mounting alone leaves it closed.
  const focusComposer = () => act(() => tree.root.findByType(TextInput).props.onFocus());
  return { props, tree, pressable, focusComposer, unmount: () => act(() => tree.unmount()) };
}

test("the suggestion card's buttons reach the screen's callbacks", () => {
  // The card is rendered by the dock, but the *policy* lives in ChatScreen:
  // the dock only forwards. A dropped arrow here leaves the card's buttons
  // inert with no test failing.
  const onInstallSkill = jest.fn();
  const onDismissSkill = jest.fn();
  const suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: "search the web",
  };
  const { tree, unmount } = mountDock({
    skillSuggestion: suggestion, skillInstalling: true,
    onInstallSkill, onDismissSkill,
  });
  try {
    const card = tree.root.findByType(SkillSuggestionCard);
    expect(card.props.installing).toBe(true);
    expect(card.props.suggestion).toBe(suggestion);
    act(() => card.props.onInstall());
    expect(onInstallSkill).toHaveBeenCalledTimes(1);
    act(() => card.props.onDismiss());
    expect(onDismissSkill).toHaveBeenCalledTimes(1);
  } finally {
    unmount();
  }
});

test("the suggestion card's buttons are optional, so a caller that omits them does not throw", () => {
  const { tree, unmount } = mountDock({
    skillSuggestion: { skill: { name: "future-web", description: "" }, draft: "x" },
  });
  try {
    const card = tree.root.findByType(SkillSuggestionCard);
    // No `onInstallSkill` / `onDismissSkill` were passed: the dock is used
    // read-only in some hosts, and pressing must be a no-op rather than a crash.
    expect(() => act(() => card.props.onInstall())).not.toThrow();
    expect(() => act(() => card.props.onDismiss())).not.toThrow();
  } finally {
    unmount();
  }
});

test("the skill picker's details toggle opens on one tap and closes on the second", () => {
  const { tree, focusComposer, unmount } = mountDock({
    message: "/web",
    remote: {
      draft: false, selectedSessionId: "s1", desktopOnline: true,
      connectionPresentation: { level: "connected" }, models: [], modelId: "model",
      streaming: false, busy: false, abort: jest.fn(),
      capabilities: new Set(["skills_v1"]), listSkills: jest.fn(async () => []),
    },
  });
  try {
    focusComposer();
    const picker = tree.root.findByType(SkillPicker);
    const skill = { name: "future-web", description: "search the web" };
    act(() => picker.props.onShowDetails(skill));
    expect(tree.root.findByType(SkillDetailsDialog).props.skill).toMatchObject({ name: "future-web" });
    // The picker's row is told which skill is open so it can show it as
    // expanded; that flag is part of the same state, so it has to move with it.
    expect(tree.root.findByType(SkillPicker).props.detailsName).toBe("future-web");
    // Tapping the same skill again closes it: the toggle is what makes the
    // name row do something other than re-open a dialog that is already up.
    act(() => tree.root.findByType(SkillPicker).props.onShowDetails(skill));
    expect(tree.root.findByType(SkillDetailsDialog).props.skill).toBeNull();
    expect(tree.root.findByType(SkillPicker).props.detailsName).toBeNull();
    // A *different* skill replaces the open one rather than toggling it away.
    act(() => tree.root.findByType(SkillPicker).props.onShowDetails(skill));
    act(() => tree.root.findByType(SkillPicker).props.onShowDetails({ name: "other", description: "" }));
    expect(tree.root.findByType(SkillDetailsDialog).props.skill).toMatchObject({ name: "other" });
  } finally {
    unmount();
  }
});

test("the details dialog's own dismiss closes it, and an absent skill keeps it closed", () => {
  const { tree, focusComposer, unmount } = mountDock({ message: "/web", remote: {
    draft: false, selectedSessionId: "s1", desktopOnline: true,
    connectionPresentation: { level: "connected" }, models: [], modelId: "model",
    streaming: false, busy: false, abort: jest.fn(),
    capabilities: new Set(["skills_v1"]), listSkills: jest.fn(async () => []),
  } });
  try {
    focusComposer();
    expect(tree.root.findByType(SkillDetailsDialog).props.skill).toBeNull();
    const picker = tree.root.findByType(SkillPicker);
    act(() => picker.props.onShowDetails({ name: "future-web", description: "d" }));
    act(() => tree.root.findByType(SkillDetailsDialog).props.onClose());
    expect(tree.root.findByType(SkillDetailsDialog).props.skill).toBeNull();
  } finally {
    unmount();
  }
});

test("removing an attachment deletes its temporary file and drops only that row", () => {
  const removal = jest.fn();
  const attachment = (name: string) => ({
    localUri: `file:///cache/${name}`, name, mimeType: "image/jpeg",
    kind: "image" as const, originalSize: 10, transferSize: 10, temporary: true,
  });
  const setAttachments = jest.fn((update: unknown) => removal(update));
  const { tree, unmount } = mountDock({
    attachments: [attachment("a.png"), attachment("b.png"), attachment("c.png")],
    setAttachments,
  });
  const deleteTemporary = jest.mocked(deleteTemporaryAttachment);
  try {
    // The middle row: a filter keyed on the wrong index would drop the wrong
    // file, and an off-by-one here is invisible unless the *identity* is
    // asserted rather than the count. The label is bare because the test's `t`
    // returns the key; the rows are told apart by position.
    const removes = tree.root.findAll(node =>
      node.props.accessibilityLabel === "attachment.remove" && typeof node.props.onPress === "function");
    expect(removes).toHaveLength(3);
    act(() => removes[1]!.props.onPress());
    expect(setAttachments).toHaveBeenCalledTimes(1);
    const next = removal.mock.calls[0]![0] as (current: unknown[]) => unknown[];
    const current = [attachment("a.png"), attachment("b.png"), attachment("c.png")];
    const kept = next(current) as { name: string }[];
    expect(kept.map(item => item.name)).toEqual(["a.png", "c.png"]);
    // The row's own local copy is deleted, and only that one: the other two
    // must stay on disk for the send that follows.
    expect(deleteTemporary).toHaveBeenCalledTimes(1);
    expect(deleteTemporary).toHaveBeenCalledWith(current[1]);
  } finally {
    unmount();
  }
});

test("images the model cannot take are called out on the pending row and above the composer", () => {
  const image = {
    localUri: "file:///cache/a.png", name: "a.png", mimeType: "image/png",
    kind: "image" as const, originalSize: 10, transferSize: 10,
  };
  const text = {
    localUri: "file:///cache/a.txt", name: "a.txt", mimeType: "text/plain",
    kind: "file" as const, originalSize: 4, transferSize: 4,
  };
  const shown = (tree: ReactTestRenderer, key: string) =>
    tree.root.findAllByType(Text).some(node => node.props.children === key);
  const withImage = mountDock({ attachments: [image], supportsImages: false });
  try {
    // Both the row's warning icon and the composer-level line: the user has to
    // see it before sending, not after the desktop rejects the message.
    expect(withImage.tree.root.findAllByType(CircleAlert).length).toBeGreaterThan(0);
    expect(shown(withImage.tree, "attachment.imagesUnsupported")).toBe(true);
  } finally {
    withImage.unmount();
  }
  const filesOnly = mountDock({ attachments: [text], supportsImages: false });
  try {
    // A non-image attachment is unaffected by the image capability, so the
    // warning must not appear for it.
    expect(shown(filesOnly.tree, "attachment.imagesUnsupported")).toBe(false);
  } finally {
    filesOnly.unmount();
  }
  const supported = mountDock({ attachments: [image], supportsImages: true });
  try {
    expect(shown(supported.tree, "attachment.imagesUnsupported")).toBe(false);
  } finally {
    supported.unmount();
  }
});
