import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  ActivityIndicator,
  BackHandler,
  FlatList,
  Keyboard,
  StyleSheet,
  Text,
} from "react-native";
import { ChatTopBar } from "../components/ChatTopBar";
import { SessionFilesPanel } from "../components/SessionFilesPanel";
import { SessionUsageSheet } from "../components/SessionUsageSheet";
import { RenameModal } from "../components/RenameModal";
import { ChatScreen } from "../ChatScreen";
import { ComposerDock } from "../components/ComposerDock";
import type { PendingSuggestion } from "../useSkillRecommendation";
import { FloatingTimelineButton } from "../components/FloatingTimelineButton";
import type { TimelineSyncStatus } from "../../../remote/syncEngine";
import type { RemoteSession, RemoteSessionUsage, RemoteWorkspace } from "../../../remote/types";

const mockRemote = {
  credentials: { expectedDesktopId: "desktop" },
  selectedSessionId: "history",
  selectedTitle: "History",
  // No usage reported until a get_state read lands; the sheet must still open
  // and say so rather than invent a ¥0 breakdown.
  sessionUsage: null as RemoteSessionUsage | null,
  closeConversation: jest.fn(),
  // The app re-reads the session as the sheet opens.
  refreshSessionUsage: jest.fn(async () => {}),
  sessions: [] as RemoteSession[],
  workspaces: [] as RemoteWorkspace[],
  models: [] as { id: string; provider: string; label?: string }[],
  modelId: undefined as string | undefined,
  // The approval decision the composer's card submits.
  decideApproval: jest.fn<Promise<void>, [string, string]>(),
  capabilities: new Set(),
  timeline: {
    items: [
      { id: "u-last", kind: "message", role: "user", text: "Question" },
      {
        id: "a-last",
        kind: "message",
        role: "assistant",
        text: "Short answer",
      },
    ],
  },
  canLoadOlderTimeline: true,
  loadingOlderTimeline: false,
  loadOlderTimeline: jest.fn<Promise<false | string[]>, []>(),
  reloadTimeline: jest.fn(),
  // A failed history read and the listing the file panel asks for.
  timelineError: null as string | null,
  retryTimeline: jest.fn(),
  listSessionFiles: jest.fn(async () => null),
  desktopOnline: true,
  timelineSyncStatus: "idle" as TimelineSyncStatus,
  connectionPresentation: { level: "connected" },
};
jest.mock("../../../remote/RemoteContext", () => ({
  useRemote: () => mockRemote,
  useRemoteControls: () => mockRemote,
}));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }),
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ bottom: 0 }),
}));
jest.mock("lucide-react-native", () => ({ History: "History" }));
jest.mock("../../../components/TimelineCard", () => ({
  TimelineCard: "TimelineCard",
}));
jest.mock("../../../components/ErrorBanner", () => ({
  ErrorBanner: "ErrorBanner",
}));
// Mutable so a test can supply a draft, a live send and a skill suggestion;
// the hooks are read at render time, after this module has initialised.
// `setMessage` mirrors the real hook's contract (it writes the draft back) so a
// test can tell a composed send from one that only stored the text.
const mockDraft: { message: string; attachments: unknown[]; setMessage: jest.Mock } = {
  message: "",
  attachments: [],
  setMessage: jest.fn(),
};
jest.mock("../useComposerDraft", () => ({
  useComposerDraft: () => mockDraft,
}));
jest.mock("../useAttachmentPicker", () => ({
  useAttachmentPicker: () => ({}),
}));
// The download controller's entry points are called with `void` by the screen
// (fire-and-forget by design). They are synchronous jest.fn()s on purpose: an
// async mock leaves a promise the test cannot await, and its continuation lands
// after the renderer is gone (§8.18).
const mockFileDownload: {
  preview: unknown;
  activeDownload: unknown;
  fileAction: unknown;
  openAttachment: jest.Mock;
  openFileLink: jest.Mock;
  openOrShare: jest.Mock;
  setFileAction: jest.Mock;
} = {
  preview: null,
  activeDownload: null,
  fileAction: null,
  openAttachment: jest.fn(),
  openFileLink: jest.fn(),
  openOrShare: jest.fn(),
  setFileAction: jest.fn(),
};
jest.mock("../useFileDownload", () => ({ useFileDownload: () => mockFileDownload }));
const mockSendApi: { send: jest.Mock; retryMessage: jest.Mock; continueMessage: jest.Mock } = {
  send: jest.fn(),
  retryMessage: jest.fn(),
  continueMessage: jest.fn(),
};
// The send/retry pair the timeline rows drive. `send` is asserted directly;
// the two others exist so the row wiring has a real controller to reach.
jest.mock("../useSendMessage", () => ({ useSendMessage: () => mockSendApi }));
const mockSkillReco: {
  suggestion: PendingSuggestion | null;
  evaluating: boolean;
  evaluate: jest.Mock;
  installAndUse: jest.Mock;
  dismiss: jest.Mock;
} = {
  suggestion: null,
  evaluating: false,
  evaluate: jest.fn(async () => false),
  installAndUse: jest.fn(async () => null),
  dismiss: jest.fn(),
};
jest.mock("../useSkillRecommendation", () => ({
  useSkillRecommendation: () => mockSkillReco,
}));
jest.mock("../components/SessionUsageSheet", () => ({
  SessionUsageSheet: "SessionUsageSheet",
}));
jest.mock("../components/ChatTopBar", () => ({ ChatTopBar: "ChatTopBar" }));
jest.mock("../components/SessionFilesPanel", () => ({
  SessionFilesPanel: "SessionFilesPanel",
}));
jest.mock("../components/ComposerDock", () => ({
  ComposerDock: "ComposerDock",
}));
jest.mock("../components/ModelSelectorSheet", () => ({
  ModelSelectorSheet: "ModelSelectorSheet",
}));
jest.mock("../components/DownloadProgressModal", () => ({
  DownloadProgressModal: "DownloadProgressModal",
}));
jest.mock("../components/PreviewModal", () => ({
  PreviewModal: "PreviewModal",
}));
jest.mock("../components/RenameModal", () => ({ RenameModal: "RenameModal" }));
jest.mock("../components/SessionUsageSheet", () => ({
  SessionUsageSheet: "SessionUsageSheet",
}));
jest.mock("../components/NativeFileActionSheet", () => ({
  NativeFileActionSheet: "NativeFileActionSheet",
}));

let tree: ReactTestRenderer;
const removeBack = jest.fn();
beforeEach(() => {
  jest.clearAllMocks();
  jest
    .spyOn(BackHandler, "addEventListener")
    .mockReturnValue({ remove: removeBack });
  jest.useFakeTimers();
  mockFileDownload.preview = null;
  mockFileDownload.activeDownload = null;
  mockFileDownload.fileAction = null;
  mockRemote.canLoadOlderTimeline = true;
  mockRemote.desktopOnline = true;
  mockRemote.timelineSyncStatus = "idle";
  mockRemote.timelineError = null;
  mockRemote.retryTimeline = jest.fn();
  mockRemote.listSessionFiles = jest.fn(async () => null);
  mockRemote.models = [];
  mockRemote.modelId = undefined;
  mockRemote.decideApproval = jest.fn<Promise<void>, [string, string]>();
  mockRemote.loadingOlderTimeline = false;
  mockRemote.sessions = [];
  mockRemote.workspaces = [];
  mockRemote.sessionUsage = null;
  mockRemote.loadOlderTimeline.mockReset().mockResolvedValue([]);
  mockDraft.message = "";
  mockDraft.attachments = [];
  mockDraft.setMessage.mockImplementation((value: string) => { mockDraft.message = value; });
  mockSkillReco.suggestion = null;
  mockSkillReco.evaluating = false;
  act(() => {
    tree = create(createElement(ChatScreen));
  });
});
afterEach(() => {
  act(() => tree.unmount());
  jest.useRealTimers();
  jest.restoreAllMocks();
});

/** The native pull-to-refresh indicator the transcript hands to the platform. */
const refreshControl = () =>
  tree.root.findByType(FlatList).props.refreshControl;

const olderCollision = () => {
  const list = tree.root.findByType(FlatList);
  act(() => list.props.onScrollBeginDrag({
    nativeEvent: {
      contentOffset: { x: 0, y: 1400 },
      contentSize: { width: 320, height: 2000 },
      layoutMeasurement: { width: 320, height: 600 },
    },
  }));
};

test.each(["files", "preview", "download"])("%s retains the list while covered and catches up without remounting", surface => {
  const original = mockRemote.timeline;
  const list = tree.root.findByType(FlatList);
  const before = list.props.data;
  try {
    if (surface === "files") act(() => tree.root.findByType(ChatTopBar).props.onFiles());
    else {
      if (surface === "preview") mockFileDownload.preview = {};
      else mockFileDownload.activeDownload = {};
      act(() => tree.update(createElement(ChatScreen)));
    }
    mockRemote.timeline = { items: [...original.items, { id: "covered", kind: "message", role: "assistant", text: "arrived while covered" }] };
    act(() => tree.update(createElement(ChatScreen)));
    expect(tree.root.findByType(FlatList).props.data).toBe(before);
    if (surface === "files") {
      expect(tree.root.findAll(node => node.props.accessibilityElementsHidden === true).length).toBeGreaterThan(0);
      act(() => tree.root.findByType(ChatTopBar).props.onBack());
    } else {
      mockFileDownload.preview = null;
      mockFileDownload.activeDownload = null;
      act(() => tree.update(createElement(ChatScreen)));
    }
    expect(tree.root.findByType(FlatList)).toBe(list);
    expect(list.props.data[0]).toMatchObject({ id: "covered", text: "arrived while covered" });
  } finally { mockRemote.timeline = original; }
});

test("file browsing owns system back while the header still returns directly to chat", () => {
  expect(BackHandler.addEventListener).toHaveBeenCalledTimes(1);
  act(() => tree.root.findByType(ChatTopBar).props.onFiles());
  expect(tree.root.findByType(SessionFilesPanel)).toBeDefined();
  expect(removeBack).toHaveBeenCalledTimes(1);
  // The parent must not install a later listener that swallows folder back.
  expect(BackHandler.addEventListener).toHaveBeenCalledTimes(1);
  act(() => tree.root.findByType(ChatTopBar).props.onBack());
  expect(tree.root.findAllByType(SessionFilesPanel)).toHaveLength(0);
  expect(mockRemote.closeConversation).not.toHaveBeenCalled();
  expect(BackHandler.addEventListener).toHaveBeenCalledTimes(2);
  act(() => tree.root.findByType(ChatTopBar).props.onBack());
  expect(mockRemote.closeConversation).toHaveBeenCalledTimes(1);
});

test("the spend icon opens the usage sheet, and renaming is not duplicated here", () => {
  mockRemote.sessionUsage = {
    inputTokens: 2_000, outputTokens: 500, cacheReadTokens: 0, cacheWriteTokens: 0,
    costCny: 0.5, costInputCny: 0.3, costOutputCny: 0.2, costCacheReadCny: 0, costCacheWriteCny: 0,
  };
  act(() => tree.update(createElement(ChatScreen)));
  const bar = tree.root.findByType(ChatTopBar);
  // The session list owns renaming, so the conversation header exposes neither
  // a rename action nor a rename modal.
  expect(bar.props.onRename).toBeUndefined();
  expect(tree.root.findAllByType(RenameModal)).toHaveLength(0);
  expect(tree.root.findByType(SessionUsageSheet).props.visible).toBe(false);

  act(() => bar.props.onUsage());
  const sheet = tree.root.findByType(SessionUsageSheet);
  expect(sheet.props.visible).toBe(true);
  expect(sheet.props.usage).toBe(mockRemote.sessionUsage);
  // Opening the sheet re-reads the session: the cached amount can be a run
  // behind, and the sheet is the moment it is looked at.
  expect(mockRemote.refreshSessionUsage).toHaveBeenCalledTimes(1);
  // The sheet is an accounting view: closing it is its only action.
  expect(sheet.props.onRename).toBeUndefined();
});

test("a conversation without reported usage still opens the sheet", () => {
  act(() => tree.root.findByType(ChatTopBar).props.onUsage());
  const sheet = tree.root.findByType(SessionUsageSheet);
  expect(sheet.props.visible).toBe(true);
  expect(sheet.props.usage).toBeNull();
});

test("an ordinary conversation does not expose its internal storage workspace", () => {
  mockRemote.sessions = [{
    sessionId: "history",
    threadId: "thread",
    title: "History",
    mode: "chat",
    workspaceId: "internal-chat-workspace",
    streaming: false,
  }];
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(ChatTopBar).props.contextLabel).toBe(
    "sessions.conversations",
  );
});

test("a legacy conversation can infer its visible workspace when mode is absent", () => {
  mockRemote.sessions = [{
    sessionId: "history",
    threadId: "thread",
    title: "History",
    workspaceId: "workspace",
    streaming: false,
  }];
  mockRemote.workspaces = [{ id: "workspace", name: "Project", path: "/project" }];
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(ChatTopBar).props.contextLabel).toBe(
    "chat.workspaceNamed",
  );
});

test("an older-history collision shows only a one-second loading indicator", async () => {
  mockRemote.loadOlderTimeline.mockResolvedValueOnce(["older"]);
  const list = tree.root.findByType(FlatList);
  expect(list.props.data).toHaveLength(2);
  expect(list.props.inverted).toBe(true);
  expect(list.props.ListFooterComponent).toBeUndefined();
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
  olderCollision();
  const button = tree.root.findByType(FloatingTimelineButton);
  expect(button.props.busy).toBe(true);
  expect(StyleSheet.flatten(button.props.style).top).toBeGreaterThanOrEqual(0);
  expect(mockRemote.loadOlderTimeline).toHaveBeenCalledTimes(1);
  expect(
    tree.root.findByType(FlatList).props.maintainVisibleContentPosition,
  ).toEqual({ minIndexForVisible: 0 });
  expect(tree.root.findByType(FloatingTimelineButton).props.busy).toBe(true);
  expect(tree.root.findByType(FloatingTimelineButton).props.label).toBe(
    "chat.loadingOlder",
  );
  expect(tree.root.findAllByType(ActivityIndicator).length).toBeGreaterThan(0);
  await act(async () => {});
  act(() => list.props.onContentSizeChange(320, 2500));
  act(() => jest.advanceTimersByTime(999));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(1);
  act(() => jest.advanceTimersByTime(1));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
});

test("failed history hides after one second and retries only on a new collision", async () => {
  mockRemote.loadOlderTimeline.mockResolvedValueOnce(false);
  olderCollision();
  await act(async () => {});
  expect(tree.root.findByType(FloatingTimelineButton).props.label).toBe(
    "chat.loadingOlder",
  );
  act(() => jest.advanceTimersByTime(1000));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
  olderCollision();
  expect(mockRemote.loadOlderTimeline).toHaveBeenCalledTimes(2);
});

test("cached messages remain visible and the sync notice stays for at least 750ms", () => {
  const data = tree.root.findByType(FlatList).props.data;
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(FlatList).props.data).toBe(data);
  expect(
    tree.root.findAll((node) => node.props.children === "chat.syncingLatest")
      .length,
  ).toBeGreaterThan(0);
  mockRemote.timelineSyncStatus = "retrying";
  act(() => tree.update(createElement(ChatScreen)));
  expect(
    tree.root.findAll((node) => node.props.children === "chat.syncRetrying")
      .length,
  ).toBeGreaterThan(0);
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(
    tree.root.findAll(
      (node) => node.props.children === "chat.syncWaitingNetwork",
    ).length,
  ).toBeGreaterThan(0);
  mockRemote.desktopOnline = true;
  mockRemote.timelineSyncStatus = "idle";
  act(() => tree.update(createElement(ChatScreen)));
  expect(
    tree.root.findAll(
      (node) => node.props.accessibilityLiveRegion === "polite",
    ),
  ).not.toHaveLength(0);
  act(() => jest.advanceTimersByTime(749));
  expect(
    tree.root.findAll(
      (node) => node.props.accessibilityLiveRegion === "polite",
    ),
  ).not.toHaveLength(0);
  act(() => jest.advanceTimersByTime(1));
  expect(
    tree.root.findAll(
      (node) => node.props.accessibilityLiveRegion === "polite",
    ),
  ).toHaveLength(0);
  expect(tree.root.findByType(FlatList).props.data).toBe(data);
});

test("the pull spinner belongs to the pull, not to the lane", () => {
  // An automatic sync already says so in the notice above the transcript; the
  // native indicator at the bottom must not report the same wait a second time.
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(refreshControl().props.refreshing).toBe(false);

  // A pull spins it, and it stays up until the lane it restarted settles.
  act(() => refreshControl().props.onRefresh());
  expect(mockRemote.reloadTimeline).toHaveBeenCalledTimes(1);
  expect(refreshControl().props.refreshing).toBe(true);

  mockRemote.timelineSyncStatus = "idle";
  act(() => tree.update(createElement(ChatScreen)));
  expect(refreshControl().props.refreshing).toBe(false);
});

test("the pull's spinner is the only report of the wait it restarts", () => {
  // The pull restarts the same lane the notice above the transcript reports, so
  // both come up at once and say the same thing twice. The spinner is at the
  // finger; the pill stands down for the state it would duplicate.
  const notice = (key: string) =>
    tree.root.findAll(node => node.props.children === key).length > 0;
  act(() => refreshControl().props.onRefresh());
  mockRemote.timelineSyncStatus = "syncing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(refreshControl().props.refreshing).toBe(true);
  expect(notice("chat.syncingLatest")).toBe(false);
  act(() => jest.advanceTimersByTime(2000));
  expect(notice("chat.syncingLatest")).toBe(false);

  // The two richer states stay: the spinner cannot say "retrying" or "waiting
  // for the connection", so the pill is no longer a repeat of it.
  mockRemote.timelineSyncStatus = "retrying";
  act(() => tree.update(createElement(ChatScreen)));
  expect(notice("chat.syncRetrying")).toBe(true);
  mockRemote.desktopOnline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(notice("chat.syncWaitingNetwork")).toBe(true);
  // Waiting for the connection is not work in flight: the notice drops its own
  // spinner (the lane cannot make progress on its own here).
  const waiting = tree.root.find(
    node => node.props.accessibilityLiveRegion === "polite",
  );
  expect(waiting.findAllByType(ActivityIndicator)).toHaveLength(0);
});

test("text selection cannot trigger Android focus-driven transcript scrolling", () => {
  const list = tree.root.findByType(FlatList);
  expect(list.props.inverted).toBe(true);
  expect(list.props.scrollsChildToFocus).toBe(false);
  // Do not solve selection jumps by disabling manual reading/scrolling.
  expect(list.props.scrollEnabled).not.toBe(false);
});

test("no floating history button when the history is exhausted", () => {
  mockRemote.canLoadOlderTimeline = false;
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
});

test("an external loading flag does not create an extra prompt without a collision", () => {
  mockRemote.loadingOlderTimeline = true;
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findAllByType(FloatingTimelineButton)).toHaveLength(0);
});

// The card holds the draft, but the send button stays live: pressing it is the
// same as the card's "send without it" — dismiss the suggestion, send as typed.
// Swallowing the press (or only toasting) read as a dead send button.
test("a plain send with a skill card up dismisses it and sends the draft", async () => {
  mockDraft.message = "please search the web for this";
  mockSkillReco.suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: mockDraft.message,
  };
  act(() => tree.update(createElement(ChatScreen)));

  // `send` is async: the press has to be awaited or its state updates land after
  // this test finished and poison whichever test runs next (§8.18).
  await act(async () => {
    await tree.root.findByType(ComposerDock).props.send();
  });

  expect(mockSkillReco.dismiss).toHaveBeenCalledTimes(1);
  expect(mockSendApi.send).toHaveBeenCalledTimes(1);
});

test("an empty composer sends nothing and asks no skill oracle", async () => {
  mockDraft.message = "   ";
  mockDraft.attachments = [];
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.send();
  });

  // No draft means nothing to evaluate and nothing to send: an empty turn would
  // burn a desktop round trip and put a blank bubble in the thread.
  expect(mockSkillReco.evaluate).not.toHaveBeenCalled();
  expect(mockSendApi.send).not.toHaveBeenCalled();
});

test("a skill the oracle does not recommend falls through to a normal send", async () => {
  mockDraft.message = "summarise this";
  mockSkillReco.evaluate.mockResolvedValueOnce(false);
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.send();
  });

  // "No match, timeout, error" all take this path: the message still goes out.
  expect(mockSkillReco.evaluate).toHaveBeenCalledWith("summarise this");
  expect(mockSendApi.send).toHaveBeenCalledTimes(1);
});

test("a recommended skill holds the draft instead of sending it", async () => {
  mockDraft.message = "search the web for the changelog";
  mockSkillReco.evaluate.mockResolvedValueOnce(true);
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.send();
  });

  // The card is on screen asking the user to decide, so sending now would both
  // pre-empt the choice and leave the card describing a draft it no longer has.
  expect(mockSkillReco.evaluate).toHaveBeenCalledTimes(1);
  expect(mockSendApi.send).not.toHaveBeenCalled();
});

test("installing the suggested skill sends the composed draft, not the raw one", async () => {
  mockDraft.message = "search the web";
  mockSkillReco.suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: mockDraft.message,
  };
  mockSkillReco.installAndUse.mockResolvedValueOnce("use future-web: search the web");
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.onInstallSkill();
  });

  // The send has to carry the composed text explicitly: the composer state still
  // holds the pre-install draft, and sending *that* would silently drop the
  // skill the user just installed for this message.
  expect(mockDraft.setMessage).toHaveBeenCalledWith("use future-web: search the web");
  expect(mockSendApi.send).toHaveBeenCalledWith("use future-web: search the web");
  expect(tree.root.findByType(ComposerDock).props.skillInstalling).toBe(false);
});

test("a failed skill install keeps the card up and sends nothing", async () => {
  mockDraft.message = "search the web";
  mockSkillReco.suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: mockDraft.message,
  };
  mockSkillReco.installAndUse.mockResolvedValueOnce(null);
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.onInstallSkill();
  });

  // PRD v1.6 §6.2: the user must be able to retry or send without it, so the
  // draft stays unsent and the installing flag is released.
  expect(mockSendApi.send).not.toHaveBeenCalled();
  expect(tree.root.findByType(ComposerDock).props.skillSuggestion).not.toBeNull();
  expect(tree.root.findByType(ComposerDock).props.skillInstalling).toBe(false);
});

test("a second install tap cannot start a second install", async () => {
  mockDraft.message = "search the web";
  mockSkillReco.suggestion = {
    skill: { name: "future-web", description: "search the web" },
    draft: mockDraft.message,
  };
  let release!: (value: string | null) => void;
  mockSkillReco.installAndUse.mockReturnValueOnce(
    new Promise<string | null>(resolve => { release = resolve; }),
  );
  act(() => tree.update(createElement(ChatScreen)));

  let first!: Promise<void>;
  await act(async () => {
    first = tree.root.findByType(ComposerDock).props.onInstallSkill();
  });
  expect(tree.root.findByType(ComposerDock).props.skillInstalling).toBe(true);
  // The dock keeps the button mounted while it spins; a second tap must be
  // absorbed rather than queue a second install of the same skill.
  await act(async () => {
    await tree.root.findByType(ComposerDock).props.onInstallSkill();
  });
  expect(mockSkillReco.installAndUse).toHaveBeenCalledTimes(1);
  await act(async () => {
    release("composed");
    await first;
  });
  expect(mockSendApi.send).toHaveBeenCalledTimes(1);
});

test("an approval decision reports its own failure and clears the spinner", async () => {
  const decide = jest.fn<Promise<void>, [string, string]>();
  decide.mockRejectedValueOnce(new Error("desktop offline"));
  mockRemote.decideApproval = decide as never;
  act(() => tree.update(createElement(ChatScreen)));

  await act(async () => {
    await tree.root.findByType(ComposerDock).props.decideApproval("a1", "approved");
  });

  // A refused decision must keep the card actionable: the error names the card
  // it belongs to, and the submitting flag is released so it can be retried.
  expect(decide).toHaveBeenCalledWith("a1", "approved");
  expect(tree.root.findByType(ComposerDock).props.approvalError).toMatchObject({ id: "a1" });
  expect(tree.root.findByType(ComposerDock).props.approvalSubmitting).toBeNull();

  decide.mockResolvedValueOnce(undefined);
  await act(async () => {
    await tree.root.findByType(ComposerDock).props.decideApproval("a1", "approved");
  });
  // A later success clears the previous failure rather than leaving it pinned.
  expect(tree.root.findByType(ComposerDock).props.approvalError).toBeNull();
  expect(tree.root.findByType(ComposerDock).props.approvalSubmitting).toBeNull();
});

test("the model selector receives the model the reference resolves to", () => {
  mockRemote.models = [
    { id: "a", provider: "p", label: "Model A" },
    { id: "b", provider: "p", label: "Model B" },
  ];
  mockRemote.modelId = "p/b";
  act(() => tree.update(createElement(ChatScreen)));

  // The composer labels the *labelled* active model, which only works when the
  // provider-qualified reference is matched against the catalogue.
  expect(tree.root.findByType(ComposerDock).props.activeModelLabel).toBe("Model B");

  // A reference the catalogue does not list falls back to the raw id, never to
  // a blank label the user cannot act on.
  mockRemote.modelId = "p/missing";
  act(() => tree.update(createElement(ChatScreen)));
  expect(tree.root.findByType(ComposerDock).props.activeModelLabel).toBe("p/missing");
});

/**
 * The screen is mostly wiring: props it hands to the platform and to the mocked
 * surfaces. A lambda that is never invoked is indistinguishable from one that
 * was deleted, so each is driven here and its effect asserted.
 */
describe("ChatScreen wiring", () => {
  // A renderer and a keyboard-listener capture of its own. Mounting once per
  // test with the spy already installed means no test has to unmount and remount
  // to observe a listener, which is what kept corrupting the shared tree.
  let screen!: ReactTestRenderer;
  const keyHandlers: ((event: { endCoordinates: { height: number } }) => void)[] = [];
  let keySpy!: jest.SpyInstance;
  beforeEach(() => {
    keyHandlers.length = 0;
    keySpy = jest.spyOn(Keyboard, "addListener").mockImplementation(((
      _name: string,
      handler: (event: { endCoordinates: { height: number } }) => void,
    ) => {
      keyHandlers.push(handler);
      return { remove: jest.fn() };
    }) as never);
    act(() => { screen = create(createElement(ChatScreen)); });
  });
  afterEach(() => {
    act(() => screen.unmount());
    keySpy.mockRestore();
  });
  const view = () => screen.root;

  test("the transcript's own callbacks reach the scroll and layout controllers", () => {
    const list = () => view().findByType(FlatList);
    // These are the platform's calls, not the app's: a handler that stopped
    // forwarding would leave paging and the reading position silently frozen.
    act(() => list().props.onLayout({ nativeEvent: { layout: { width: 320, height: 600, x: 0, y: 0 } } }));
    act(() => list().props.onScroll({
      nativeEvent: {
        contentOffset: { x: 0, y: 0 },
        contentSize: { width: 320, height: 2000 },
        layoutMeasurement: { width: 320, height: 600 },
      },
    }));
    act(() => list().props.onScrollBeginDrag({
      nativeEvent: {
        contentOffset: { x: 0, y: 1400 },
        contentSize: { width: 320, height: 2000 },
        layoutMeasurement: { width: 320, height: 600 },
      },
    }));
    act(() => list().props.onContentSizeChange(320, 2000));
    expect(list().props.data).toBeDefined();
  });

  test("a conversation ending on the user's own prompt has no live reply to highlight", () => {
    // Only a trailing *assistant* message is the streaming one; the loop that
    // finds it has to fall through to null rather than pick the user's turn.
    const original = mockRemote.timeline;
    try {
      mockRemote.timeline = { items: [{ id: "u", kind: "message", role: "user", text: "Q" }] };
      act(() => screen.update(createElement(ChatScreen)));
      const card = view().findAllByType("TimelineCard" as never)[0]!;
      expect(card.props.isLatestAssistant).toBe(false);
    } finally {
      mockRemote.timeline = original;
    }
  });

  test("failed history hands the user a retry that re-reads the session", () => {
    // This test is the only one that rewrites the shared transcript, so it puts
    // it back: a leaked empty timeline makes every later test render nothing.
    const original = mockRemote.timeline;
    const originalError = mockRemote.timelineError;
    try {
      mockRemote.timeline = { items: [] };
      mockRemote.timelineError = "history_timeout";
      mockRemote.retryTimeline = jest.fn();
      act(() => screen.update(createElement(ChatScreen)));

      const retry = view().findAll(node =>
        node.props.accessibilityRole === "button" && typeof node.props.onPress === "function"
        && node.findAllByType(Text).some(child => child.props.children === "common.retry"))[0];
      expect(retry).toBeDefined();
      retry!.props.onPress();
      // The empty transcript would otherwise be a dead end: this is the only
      // affordance that can bring the history back.
      expect(mockRemote.retryTimeline).toHaveBeenCalledTimes(1);
    } finally {
      mockRemote.timeline = original;
      mockRemote.timelineError = originalError;
    }
  });

  test("system back returns to the conversation list", () => {
    const handler = jest.mocked(BackHandler.addEventListener).mock.calls.at(-1)![1];
    let handled: boolean | null | undefined;
    act(() => { handled = handler({ type: "hardwareBackPress", timeStamp: 1 }); });
    // Handled *and* acted on: returning true alone would swallow the gesture.
    expect(handled).toBe(true);
    expect(mockRemote.closeConversation).toHaveBeenCalledTimes(1);
  });

  test("both keyboard events move or release the suggestion offset", () => {
    // A keyboard that opens must raise the card above it and one that closes
    // must release the offset; leaving either unhandled parks the card behind
    // the keys or leaves the composer permanently raised.
    expect(keyHandlers.length).toBeGreaterThanOrEqual(2);
    for (const handler of keyHandlers) {
      act(() => handler({ endCoordinates: { height: 320 } }));
      act(() => handler({ endCoordinates: { height: 0 } }));
    }
    expect(view().findByType(ComposerDock)).toBeDefined();
  });

  test("the transcript's row actions reach the controllers behind them", () => {
    // `TimelineCard` is mocked to a host element, so its props are readable and
    // its callbacks can be driven exactly as the real card drives them.
    const card = view().findAllByType("TimelineCard" as never)[0]!;
    expect(card).toBeDefined();
    const attachment = { path: "C:\\work\\a.png", name: "a.png" };
    const item = { id: "a-last", kind: "message" as const, role: "assistant" as const, text: "answer" };
    // Each callback is a stable indirection onto the newest controller closure
    // (memoized cards must not capture a stale one), so the assertion is that
    // the prop reaches a live controller rather than throwing.
    act(() => card.props.onOpenAttachment(attachment));
    expect(mockFileDownload.openAttachment).toHaveBeenCalledWith(attachment);
    act(() => card.props.onOpenFile("C:\\work\\a.png"));
    expect(mockFileDownload.openFileLink).toHaveBeenCalledWith("C:\\work\\a.png");
    act(() => card.props.onRetry(item));
    expect(mockSendApi.retryMessage).toHaveBeenCalledWith(item);
    act(() => card.props.onContinue(item));
    expect(mockSendApi.continueMessage).toHaveBeenCalledWith(item);
  });

  test("the file panel is told about the conversation and routes its file opens", () => {
    mockRemote.sessions = [
      { sessionId: "history", threadId: "t", title: "History", mode: "workspace", streaming: false },
    ];
    mockRemote.listSessionFiles = jest.fn(async () => null);
    act(() => view().findByType(ChatTopBar).props.onFiles());
    act(() => screen.update(createElement(ChatScreen)));

    const panel = view().findByType(SessionFilesPanel);
    // The panel is told the conversation is a workspace, so it can offer the
    // workspace-rooted listing rather than the session root.
    expect(panel.props.isWorkspace).toBe(true);
    expect(panel.props.listFiles).toBe(mockRemote.listSessionFiles);
    act(() => panel.props.onOpenFile("C:\\work\\notes.md"));
    expect(mockFileDownload.openFileLink).toHaveBeenCalledWith("C:\\work\\notes.md", true);
    // Closing and reopening must not stack a second panel: the pane is a single
    // surface the header toggles, and the transcript underneath stays frozen
    // (PausedTimeline) rather than remounting.
    act(() => panel.props.onClose());
    act(() => view().findByType(ChatTopBar).props.onFiles());
    act(() => screen.update(createElement(ChatScreen)));
    expect(view().findAllByType(SessionFilesPanel)).toHaveLength(1);
  });

  test("a sync that resumes inside the notice's minimum window cancels the queued hide", () => {
    // The pill is held for a minimum window so it cannot flash; if the lane
    // starts working again inside that window the queued hide must be
    // cancelled, or the notice would vanish while a sync is running.
    const polite = () => view().findAll(node => node.props.accessibilityLiveRegion === "polite").length;
    mockRemote.timelineSyncStatus = "syncing";
    act(() => screen.update(createElement(ChatScreen)));
    expect(polite()).toBeGreaterThan(0);
    mockRemote.timelineSyncStatus = "idle";
    act(() => screen.update(createElement(ChatScreen)));
    // The hide is queued but has not fired: no timer has been advanced yet.
    mockRemote.timelineSyncStatus = "syncing";
    act(() => screen.update(createElement(ChatScreen)));
    mockRemote.timelineSyncStatus = "idle";
    act(() => screen.update(createElement(ChatScreen)));

    // Advancing well past the window proves the earlier hide was cancelled:
    // a leaked timer would have dropped the pill mid-sync and this one would
    // then be waiting on its own fresh minimum.
    act(() => jest.advanceTimersByTime(200));
    expect(polite()).toBeGreaterThan(0);
  });

  test("the load-older hint dedupes a tap while a page is in flight", () => {
    const original = mockRemote.timeline;
    try {
      // Nothing collided yet: no hint at all.
      expect(view().findAllByType(FloatingTimelineButton)).toHaveLength(0);
      // A drag that runs into the top of the loaded window is what asks for more.
      act(() => view().findByType(FlatList).props.onScrollBeginDrag({
        nativeEvent: {
          contentOffset: { x: 0, y: 1400 },
          contentSize: { width: 320, height: 2000 },
          layoutMeasurement: { width: 320, height: 600 },
        },
      }));
      act(() => screen.update(createElement(ChatScreen)));
      const hint = () => view().findAllByType(FloatingTimelineButton)[0]!;
      expect(hint()).toBeDefined();
      // The collision started a page, so the button reports the paging
      // controller's real state rather than a fixed label.
      expect(hint().props.busy).toBe(true);
      const queued = mockRemote.loadOlderTimeline.mock.calls.length;
      act(() => hint().props.onPress());
      // `loadOlder` waits for a fresh collision, so the button's own press
      // cannot smuggle in a second page behind the controller's back.
      expect(mockRemote.loadOlderTimeline.mock.calls.length).toBe(queued);
    } finally {
      mockRemote.timeline = original;
    }
  });

  test("the spend sheet opens from the header and closes from its own dismiss", () => {
    const sheet = () => view().findAll(node =>
      node.props.visible !== undefined && typeof node.props.onClose === "function")[0]!;
    const usage = () => view().findAll(node =>
      node.props.usage !== undefined && node.props.title !== undefined)[0]!;
    expect(usage().props.visible).toBe(false);
    act(() => view().findByType(ChatTopBar).props.onUsage());
    act(() => screen.update(createElement(ChatScreen)));
    expect(usage().props.visible).toBe(true);
    // The sheet closes through its own callback, so a dismissal gesture cannot
    // leave it mounted over the transcript.
    act(() => sheet().props.onClose());
    act(() => screen.update(createElement(ChatScreen)));
    expect(usage().props.visible).toBe(false);
  });

  test("the native action sheet hands its selection back to the download controller", async () => {
    const fileAction = { info: { transferId: "t1", name: "a.pdf" }, cachedFile: null };
    mockFileDownload.fileAction = fileAction;
    act(() => screen.update(createElement(ChatScreen)));

    const sheet = view().find(node =>
      node.props.action !== undefined && typeof node.props.onSelect === "function");
    expect(sheet).toBeDefined();
    await act(async () => { await sheet.props.onSelect(fileAction, "share"); });
    // The sheet owns no policy: it hands the chosen operation and file back to
    // the controller, which owns the download, the preview and the handoff.
    expect(mockFileDownload.openOrShare).toHaveBeenCalledWith(fileAction.info, null, "share");
    act(() => sheet.props.onClose());
    expect(mockFileDownload.setFileAction).toHaveBeenCalledWith(null);
  });
});
