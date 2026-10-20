import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { notifyTaskFinished } from "../../notifications/taskNotifications";
import { useDesktopRuntime } from "../useDesktopRuntime";
import type { DesktopRuntime } from "../useDesktopRuntime";
import type { RemoteCredentials, RemoteSession, StreamEvent } from "../types";

/**
 * The per-desktop runtime's own wiring: how one desktop's events, catalogue and
 * conversation are joined together.
 *
 * The hooks it composes are replaced so what is under test is the runtime's own
 * routing — one event reaching the settings sink, the run observer and the
 * transcript — rather than any of the sub-hooks, which have their own suites.
 */

const mockConnection = {
  clientRef: { current: null },
  credentialsRef: { current: null as RemoteCredentials | null },
  credentials: null as RemoteCredentials | null,
  phase: "ready",
  error: null as string | null,
  presence: { agentAvailable: true } as { agentAvailable: boolean } | null,
  desktopOnline: true,
  hasConnectedContent: true,
  capabilities: new Set<string>(),
  fileTransferSupported: false,
  promptReceiptSupported: false,
  recordError: jest.fn(),
  clearError: jest.fn(),
  recover: jest.fn(async () => {}),
  setAppActive: jest.fn(),
  setVisibleSession: jest.fn(),
  notifyUnpair: jest.fn(async () => {}),
};
let connectionOptions: {
  handleEvent: (event: StreamEvent, sessionId: string) => void;
  credentialsRef: { current: RemoteCredentials | null };
  resetConversation?: () => void;
  applySessionStreaming?: (sessionId: string, streaming: boolean) => void;
  controlChanged?: (control: "active" | "standby") => void;
} = { handleEvent: jest.fn(), credentialsRef: { current: null } };

const mockCatalog = {
  sessions: [] as RemoteSession[],
  workspaces: [],
  models: [],
  unreadSessions: new Set<string>(),
  setUnreadSessions: jest.fn(),
  catalogSync: { sessions: "ready", workspaces: "ready", models: "ready", settings: "ready" },
  setCatalogEpoch: jest.fn(),
  applySessionSnapshot: jest.fn(),
  setWorkspaces: jest.fn(),
  refreshModels: jest.fn(async () => {}),
  refreshSessions: jest.fn(async () => {}),
  refreshSettings: jest.fn(async () => {}),
  refreshWorkspaces: jest.fn(async () => {}),
  noteCatalogRevisions: jest.fn(),
  forgetCatalogue: jest.fn(),
  observeRunEvent: jest.fn(),
  rename: jest.fn(),
  generateTitle: jest.fn(),
  deleteSession: jest.fn(),
  deleteWorkspace: jest.fn(),
  createWorkspace: jest.fn(),
  setSessionPinned: jest.fn(),
  setWorkspacePinned: jest.fn(),
  renameWorkspace: jest.fn(),
  reset: jest.fn(),
  approvalTier: "off",
  setApprovalTier: jest.fn(),
  sandboxAvailable: false,
  titleOverrides: {},
  setTitleOverrides: jest.fn(),
};
let catalogTaskFinished: ((session: RemoteSession) => void) | undefined;

let mockTimelineHandleEvent: jest.Mock = jest.fn();
let timelineOptions: {
  onSessionState?: (sessionId: string, state: { model?: string; thinkingLevel?: string }) => void;
} = {};

const mockConversation = {
  modelId: "",
  thinkingLevel: "off",
  sessionUsage: null,
  openingSession: false,
  selectSession: jest.fn(async (_sessionId: string) => {}),
  newConversation: jest.fn(async (_mode?: string, _workspaceId?: string) => {}),
  abort: jest.fn(async () => {}),
  compactContext: jest.fn(),
  setModel: jest.fn(async () => {}),
  setThinkingLevel: jest.fn(async () => {}),
  setApprovalTier: jest.fn(async () => {}),
  deleteSession: jest.fn(async () => {}),
  deleteWorkspace: jest.fn(async () => {}),
  forkConversation: jest.fn(async () => {}),
  decideApproval: jest.fn(async () => {}),
  listSkills: jest.fn(async () => []),
  listSessionFiles: jest.fn(),
  resolveToolCallTarget: jest.fn(),
  prepareAttachment: jest.fn(),
  cachedAttachment: jest.fn(),
  downloadAttachment: jest.fn(),
  refreshSessionUsage: jest.fn(async () => {}),
  applySessionSettings: jest.fn(),
  handleSessionSettingsEvent: jest.fn(),
};
let conversationOptions: { setSelectedSessionId?: (id: string) => void; setDraft?: (value: boolean) => void } = {};

const mockOutbox = { sending: false, sendMessage: jest.fn(async () => {}), continueRun: jest.fn(async () => {}) };

jest.mock("../useDesktopConnection", () => ({
  useDesktopConnection: (options: typeof connectionOptions) => {
    connectionOptions = options;
    // The connection owns the credentials slot the runtime's notification guard
    // fences on; without adopting it there is no pairing identity.
    if (mockConnection.credentials) {
      options.credentialsRef.current = mockConnection.credentials;
      mockConnection.credentialsRef = options.credentialsRef;
    }
    return mockConnection;
  },
}));
jest.mock("../useSessionCatalog", () => ({
  useSessionCatalog: (
    _clientRef: unknown,
    _selectedRef: unknown,
    onTaskFinished: (session: RemoteSession) => void,
  ) => {
    catalogTaskFinished = onTaskFinished;
    return mockCatalog;
  },
}));
jest.mock("../useTimelineController", () => {
  const React = jest.requireActual<typeof import("react")>("react");
  const { emptyTimeline } = jest.requireActual<typeof import("@future-os/remote-client/timeline")>("@future-os/remote-client/timeline");
  return {
    useTimelineController: (options: typeof timelineOptions) => {
      timelineOptions = options;
      const [timeline] = React.useState(emptyTimeline);
      return {
        timeline,
        timelinePending: false,
        timelineError: null,
        timelineSyncStatus: "idle",
        canLoadOlderTimeline: false,
        loadingOlderTimeline: false,
        loadOlderTimeline: jest.fn(),
        reloadTimeline: jest.fn(),
        retryTimeline: jest.fn(),
        prepareTimelineOpen: jest.fn(),
        syncEngineRef: { current: null },
        streamingRef: { current: {} },
        compactingRef: { current: {} },
        reconcileSession: jest.fn(),
        handleEvent: mockTimelineHandleEvent,
        applySessionStreaming: jest.fn(),
        awaitCompactionOutcome: jest.fn(),
        resetTimeline: jest.fn(),
        ensureDraftTimeline: jest.fn(),
      };
    },
  };
});
jest.mock("../useConversationController", () => ({
  useConversationController: (options: typeof conversationOptions) => {
    conversationOptions = options;
    return mockConversation;
  },
}));
jest.mock("../usePromptOutbox", () => ({ usePromptOutbox: () => mockOutbox }));
jest.mock("../useDesktopManagement", () => ({ useDesktopManagement: () => ({}) }));
jest.mock("../../notifications/taskNotifications", () => ({
  notifyTaskFinished: jest.fn(),
  prepareTaskNotifications: jest.fn(async () => {}),
}));

const credentials = { pairId: "pair", expectedDesktopId: "desk-a", userJwt: "jwt" } as RemoteCredentials;

let runtime: { current: DesktopRuntime };
let renderer: ReactTestRenderer | null;

function Harness({ active = true, foreground = true }: { active?: boolean; foreground?: boolean }): null {
  const value = useDesktopRuntime({
    desktopId: "desk-a",
    active,
    foreground,
    label: "Lab",
  });
  // Captured in an effect, never during render: the published value is what a
  // reader (the provider's registry) sees once the render has settled.
  useEffect(() => { runtime.current = value; });
  return null;
}

function mount(props: { active?: boolean; foreground?: boolean } = {}): void {
  act(() => { renderer = create(createElement(Harness, props)); });
}

function rerender(props: { active?: boolean; foreground?: boolean } = {}): void {
  act(() => { renderer!.update(createElement(Harness, props)); });
}

beforeEach(() => {
  jest.clearAllMocks();
  mockCatalog.sessions = [];
  mockConnection.credentials = null;
  mockConnection.credentialsRef.current = null;
  mockConnection.phase = "ready";
  mockConnection.desktopOnline = true;
  mockConnection.presence = { agentAvailable: true };
  runtime = { current: undefined as unknown as DesktopRuntime };
  renderer = null;
});

afterEach(() => {
  if (renderer) {
    act(() => renderer!.unmount());
    renderer = null;
  }
});

test("a settings event re-reads the desktop's settings, and a skills notice does not", () => {
  mount();
  act(() => connectionOptions.handleEvent({ type: "app_settings_changed", data: "{}" } as StreamEvent, "s1"));
  // The settings page keys its read off the revision, so a desktop-side change
  // must not be missed; the re-read is what makes the page agree.
  expect(mockCatalog.refreshSettings).toHaveBeenCalledTimes(1);
  expect(runtime.current.desktopSettingsRevision).toBe(1);

  act(() => connectionOptions.handleEvent({ type: "skills_changed", data: "{}" } as StreamEvent, "s1"));
  expect(mockCatalog.refreshSettings).toHaveBeenCalledTimes(1);
  expect(runtime.current.skillsRevision).toBe(1);

  act(() => connectionOptions.handleEvent({ type: "provider_config_changed", data: "{}" } as StreamEvent, "s1"));
  expect(runtime.current.desktopSettingsRevision).toBe(2);
  // Those two bump the revision and then fall through to the session router.
  expect(mockTimelineHandleEvent).toHaveBeenCalledTimes(1);
});

test("an ordinary stream event reaches settings, the run observer and the timeline", () => {
  mount();
  const event = { type: "model_changed", data: '{"model":"p/x"}' } as StreamEvent;
  act(() => connectionOptions.handleEvent(event, "s1"));
  // One event, three consumers: the routing must not short-circuit after the
  // first, or the transcript or the run bookkeeping would silently stop.
  expect(mockConversation.handleSessionSettingsEvent).toHaveBeenCalledWith(event, "s1");
  expect(mockCatalog.observeRunEvent).toHaveBeenCalledWith(event, "s1");
  expect(mockTimelineHandleEvent).toHaveBeenCalledWith(event, "s1");
});

test("a session-state push reaches the conversation controls that own the settings", () => {
  mount();
  act(() => timelineOptions.onSessionState!("s1", { model: "p/x", thinkingLevel: "high" }));
  // The connection callbacks are built before the conversation controller, so
  // the push has to travel through the sink rather than a stale closure.
  expect(mockConversation.applySessionSettings).toHaveBeenCalledWith("s1", {
    model: "p/x",
    thinkingLevel: "high",
    usage: undefined,
  });
  // An empty state falls back to the defaults the controls expect, not to
  // `undefined` (which would leave the last model selected).
  act(() => timelineOptions.onSessionState!("s2", {}));
  expect(mockConversation.applySessionSettings).toHaveBeenLastCalledWith("s2", {
    model: "",
    thinkingLevel: "off",
    usage: undefined,
  });
});

test("a finished run is announced for the pairing that is still open, with its machine named", () => {
  mockConnection.credentials = credentials;
  mount();
  expect(catalogTaskFinished).toBeDefined();
  const session = { sessionId: "s1", threadId: "t1", title: "R", streaming: false } as RemoteSession;
  act(() => catalogTaskFinished!(session));
  // Both halves of the reminder's machine: the name it shows, and the id a tap
  // on it opens the conversation with (the phone may be reading another one by
  // then, and the conversation belongs to the machine that ran it).
  expect(notifyTaskFinished).toHaveBeenCalledWith(session, expect.any(Function), {
    machine: "Lab",
    desktopId: "desk-a",
  });
  const stillPaired = jest.mocked(notifyTaskFinished).mock.calls.at(-1)![1];
  expect(stillPaired()).toBe(true);
  // The guard is the pairing identity: with three desktops paired, a task that
  // finishes after this one was re-paired must not be announced for the old one.
  mockConnection.credentialsRef.current = { pairId: "other" } as RemoteCredentials;
  expect(stillPaired()).toBe(false);
});

test("a run that finishes with no pairing is not announced at all", () => {
  mount();
  const session = { sessionId: "s1", threadId: "t1", title: "R", streaming: false } as RemoteSession;
  act(() => catalogTaskFinished!(session));
  expect(notifyTaskFinished).not.toHaveBeenCalled();
});

test("closing a conversation clears the draft and refreshes this machine's catalogue", () => {
  mount();
  expect(conversationOptions.setDraft).toBeDefined();
  act(() => conversationOptions.setDraft!(true));
  act(() => conversationOptions.setSelectedSessionId!("s1"));
  act(() => runtime.current.closeConversation());
  // Leaving a conversation must not leave a draft armed for the next one, and
  // the catalogue is re-read so the list reflects what just happened.
  expect(runtime.current.draft).toBe(false);
  expect(runtime.current.selectedSessionId).toBe("");
  expect(mockCatalog.refreshSessions).toHaveBeenCalledTimes(1);
  expect(mockCatalog.refreshWorkspaces).toHaveBeenCalledTimes(1);
});

test("resetting a conversation off the back of a connection event does not ask the machine for its catalogue", () => {
  mount();
  act(() => conversationOptions.setDraft!(true));
  expect(connectionOptions.resetConversation).toBeDefined();
  act(() => connectionOptions.resetConversation!());
  expect(runtime.current.draft).toBe(false);
  expect(mockCatalog.refreshSessions).not.toHaveBeenCalled();
});

test("control moving off this machine drops its conversations, and moving back reads them again", () => {
  mount();
  expect(connectionOptions.controlChanged).toBeDefined();

  // Another device took the machine: the host refuses this client every read and
  // sends it no snapshot, so the conversations mirrored here are somebody else's
  // to see and come out of the merged list at once.
  act(() => connectionOptions.controlChanged!("standby"));
  expect(mockCatalog.forgetCatalogue).toHaveBeenCalledTimes(1);
  expect(mockCatalog.refreshSessions).not.toHaveBeenCalled();

  // Control came back. The catalogue was emptied, and a machine sends no
  // snapshot while it is in standby, so the empty list would read as "no
  // conversations" until this side asks for it.
  act(() => connectionOptions.controlChanged!("active"));
  expect(mockCatalog.refreshSessions).toHaveBeenCalledTimes(1);
  expect(mockCatalog.refreshWorkspaces).toHaveBeenCalledTimes(1);
});

test("the open conversation's title comes from this machine's catalogue", () => {
  mockCatalog.sessions = [
    { sessionId: "s1", threadId: "t1", title: "The open one", streaming: false },
  ];
  mount();
  expect(runtime.current.selectedTitle).toBe("");
  act(() => conversationOptions.setSelectedSessionId!("s1"));
  expect(runtime.current.selectedTitle).toBe("The open one");
});

test("a taken-over machine's conversation keeps its name when the catalogue goes", () => {
  mockCatalog.sessions = [
    { sessionId: "s1", threadId: "t1", title: "Deploy notes", streaming: false },
  ];
  mount();
  act(() => conversationOptions.setSelectedSessionId!("s1"));
  expect(runtime.current.selectedTitle).toBe("Deploy notes");

  // Control moved: the rows go, the conversation the user is reading stays
  // (with its standby notice). Renaming it to "unnamed" under them would be a
  // second, unrelated change.
  mockCatalog.sessions = [];
  rerender();
  expect(runtime.current.selectedTitle).toBe("Deploy notes");

  // Another conversation is named by the catalogue that is in force.
  mockCatalog.sessions = [
    { sessionId: "s2", threadId: "t2", title: "Other", streaming: false },
  ];
  rerender();
  act(() => conversationOptions.setSelectedSessionId!("s2"));
  expect(runtime.current.selectedTitle).toBe("Other");
});

test("only the desktop being read, in front, tells its machine which conversation to detail", () => {
  const seen: string[] = [];
  mockConnection.setVisibleSession.mockImplementation((sessionId: string) => { seen.push(sessionId); });
  mount();
  act(() => conversationOptions.setSelectedSessionId!("s1"));
  expect(seen.at(-1)).toBe("s1");

  // A backgrounded phone is reading nothing, so the machine should send nothing
  // but the low-rate notices.
  rerender({ foreground: false });
  act(() => conversationOptions.setSelectedSessionId!("s2"));
  expect(seen.at(-1)).toBe("");
});

test("a runtime that is not the open one declares no visible session", () => {
  const seen: string[] = [];
  mockConnection.setVisibleSession.mockImplementation((sessionId: string) => { seen.push(sessionId); });
  mount({ active: false });
  act(() => conversationOptions.setSelectedSessionId!("s1"));
  expect(seen.every((sessionId) => sessionId === "")).toBe(true);
});

test("busy follows this machine's own send and open work", () => {
  mount();
  expect(runtime.current.busy).toBe(false);
  mockOutbox.sending = true;
  rerender();
  expect(runtime.current.busy).toBe(true);
  mockOutbox.sending = false;
  mockConversation.openingSession = true;
  rerender();
  expect(runtime.current.busy).toBe(true);
  mockConversation.openingSession = false;
});

/**
 * A capture taken before an asynchronous step is what says whether that step may
 * still write: navigation and a replaced client end the conversation, while a
 * credential renewal keeps it.
 */
test("a captured conversation lifetime follows the client it was taken with", () => {
  mount();
  const capture = runtime.current.captureConversationOperation();
  expect(capture()).toBe(true);

  // The machine's transport was replaced under the step that is still running,
  // so its write belongs to a conversation that no longer exists.
  const ref = (connectionOptions as unknown as { clientRef: { current: unknown } }).clientRef;
  ref.current = { requestRetry: jest.fn() };
  expect(capture()).toBe(false);

  // A renewal that keeps the same pair and client leaves the step valid.
  ref.current = null;
  expect(capture()).toBe(true);
});
