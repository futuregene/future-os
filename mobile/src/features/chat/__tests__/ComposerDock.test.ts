import { createElement, type ComponentProps } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ComposerDock } from "../components/ComposerDock";
import { PendingApprovalCard } from "../../../components/TimelineCard";
import { resources } from "../../../i18n/locales";
import { StyleSheet, Text, TextInput, View } from "react-native";

let mockDimensions = { width: 390, height: 844, scale: 1, fontScale: 1 };
jest.mock("react-native", () => {
  const actual = jest.requireActual("react-native");
  return new Proxy(actual, {
    get: (target, key) => key === "useWindowDimensions" ? () => mockDimensions : Reflect.get(target, key),
  });
});

jest.mock("lucide-react-native", () => Object.fromEntries(["ArrowDown", "ChevronDown", "CircleAlert", "FileText", "Paperclip", "Send", "Square", "X"].map(name => [name, () => null])));
jest.mock("../../../components/TimelineCard", () => ({ PendingApprovalCard: jest.fn(() => null) }));
jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("../../../remote/files", () => ({ deleteTemporaryAttachment: jest.fn() }));

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
    expect(button("chat.model: A very long model label").findByType(Text).props.children).toBe("A very long model label");
    expect(button("chat.thinkingLevel: thinking.high").findByType(Text).props.children).toBe("thinking.high");
    act(() => button("chat.thinkingLevel: thinking.high").props.onPress());
    expect(setSelector).toHaveBeenCalledWith("thinking");
    act(() => button("chat.model: A very long model label").props.onPress());
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

test("both languages define the history retry label", () => {
  expect(resources.en.translation.common.retry).toBe("Retry");
  expect(resources.zh.translation.common.retry).toBe("重试");
});
