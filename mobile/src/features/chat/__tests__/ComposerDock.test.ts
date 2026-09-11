import { createElement, type ComponentProps } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ComposerDock } from "../components/ComposerDock";
import { PendingApprovalCard } from "../../../components/TimelineCard";
import { resources } from "../../../i18n/locales";

jest.mock("lucide-react-native", () => Object.fromEntries(["ArrowDown", "ChevronDown", "CircleAlert", "FileText", "Paperclip", "Send", "Square", "X"].map(name => [name, () => null])));
jest.mock("../../../components/TimelineCard", () => ({ PendingApprovalCard: jest.fn(() => null) }));
jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));
jest.mock("../../../remote/files", () => ({ deleteTemporaryAttachment: jest.fn() }));

test("only the approval which failed receives the error", () => {
  type Props = ComponentProps<typeof ComposerDock>;
  const props = {
    message: "", setMessage: jest.fn(), attachments: [], setAttachments: jest.fn(),
    supportsImages: true, activeModelLabel: "model", t: (key: string) => key,
    remote: { draft: false, desktopOnline: true, connectionPresentation: { customerState: "connected" }, models: [], modelId: "model", timeline: { streaming: false } },
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
  } finally {
    act(() => renderer!.unmount());
  }
});

test("both languages define the history retry label", () => {
  expect(resources.en.translation.common.retry).toBe("Retry");
  expect(resources.zh.translation.common.retry).toBe("重试");
});
