import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { RemoteProvider, useRemoteControls, useRemote } from "../RemoteContext";
import { notifyTaskFinished, prepareTaskNotifications } from "../../notifications/taskNotifications";
import type { RemoteCredentials, RemoteSession, StreamEvent } from "../types";

// A full provider harness: the five hooks it composes are replaced so the
// provider's *own* wiring — the event router, the conversation resets and the
// notification bridge — is what these tests exercise.
const mockCatalog: {
  sessions: RemoteSession[];
  models: unknown[];
  workspaces: unknown[];
  unreadSessions: Set<string>;
  observeRunEvent: jest.Mock;
  handleEvent: jest.Mock;
  refreshSessions: jest.Mock;
  refreshWorkspaces: jest.Mock;
  refreshSettings: jest.Mock;
  setCatalogEpoch: jest.Mock;
  setSelectedSessionId: jest.Mock;
  reconcileSession: jest.Mock;
  recordError: jest.Mock;
} = {
  sessions: [],
  models: [],
  workspaces: [],
  unreadSessions: new Set<string>(),
  observeRunEvent: jest.fn(),
  handleEvent: jest.fn(),
  refreshSessions: jest.fn(async () => {}),
  refreshWorkspaces: jest.fn(async () => {}),
  refreshSettings: jest.fn(async () => {}),
  setCatalogEpoch: jest.fn(),
  setSelectedSessionId: jest.fn(),
  reconcileSession: jest.fn(),
  recordError: jest.fn(),
};
let mockTimelineHandleEvent: jest.Mock = jest.fn();
let timelineOptions: {
  onSessionState?: (sessionId: string, state: { model?: string; thinkingLevel?: string; usage?: unknown }) => void;
  resetConversation?: () => void;
} = {};
let conversationOptions: { setSelectedSessionId?: (id: string) => void; setDraft?: (value: boolean) => void } = {};
let mockCredentialsRef: { current: unknown } | undefined;
let connectionOptions: { resetConversation?: () => void } = {};
const mockConnection: {
  phase: string;
  desktopOnline: boolean;
  presence: { agentAvailable: boolean };
  credentials: RemoteCredentials | null;
} = {
  phase: "ready",
  desktopOnline: true,
  presence: { agentAvailable: true },
  credentials: null,
};
const mockConversation = {
  modelId: "",
  thinkingLevel: "off",
  openingSession: false,
  selectSession: jest.fn(async () => {}),
  applySessionSettings: jest.fn(),
  handleSessionSettingsEvent: jest.fn(),
};
let catalogTaskFinished: ((s: RemoteSession) => void) | undefined;
let mockHandleEvent: (event: StreamEvent, sessionId: string) => void;

jest.mock("../useSessionCatalog", () => ({
  useSessionCatalog: (_clientRef: unknown, _selectedRef: unknown, onTaskFinished: (s: RemoteSession) => void) => {
    catalogTaskFinished = onTaskFinished;
    return mockCatalog;
  },
}));
jest.mock("../useRemoteConnection", () => ({
  useRemoteConnection: (options: {
    handleEvent: typeof mockHandleEvent;
    credentialsRef?: { current: unknown };
    resetConversation?: () => void;
  }) => {
    mockHandleEvent = options.handleEvent;
    connectionOptions = options;
    // The connection hook owns the credentials ref; without this the provider's
    // notification guard has no pairing identity to fence on.
    if (options.credentialsRef) {
      mockCredentialsRef = options.credentialsRef;
      options.credentialsRef.current = mockConnection.credentials;
    }
    return mockConnection;
  },
}));
jest.mock("../useConversationController", () => ({
  useConversationController: (options: { setSelectedSessionId?: (id: string) => void }) => {
    conversationOptions = options;
    return mockConversation;
  },
}));
jest.mock("../usePromptOutbox", () => ({ usePromptOutbox: () => ({ sending: false, sendMessage: jest.fn(), continueRun: jest.fn() }) }));
jest.mock("../useDesktopManagement", () => ({ useDesktopManagement: () => ({}) }));
jest.mock("../useTimelineController", () => {
  const React = jest.requireActual<typeof import("react")>("react");
  const { emptyTimeline } = jest.requireActual<typeof import("../timeline")>("../timeline");
  return {
    useTimelineController: (options: typeof timelineOptions) => {
      timelineOptions = options;
      const [timeline, setTimeline] = React.useState(emptyTimeline);
      return {
        timeline, setTimeline, timelinePending: false, timelineError: null,
        canLoadOlderTimeline: false, loadingOlderTimeline: false,
        loadOlderTimeline: jest.fn(), reloadTimeline: jest.fn(), handleEvent: mockTimelineHandleEvent,
        retryTimeline: jest.fn(), listSessionFiles: jest.fn(), sessionUsage: null,
        refreshSessionUsage: jest.fn(), lastSyncedAt: 0, timelineSyncStatus: "idle",
      };
    },
  };
});
jest.mock("../../notifications/taskNotifications", () => ({
  notifyTaskFinished: jest.fn(),
  prepareTaskNotifications: jest.fn(async () => {}),
}));

function mount(child?: ReturnType<typeof createElement>) {
  let renderer!: ReactTestRenderer;
  act(() => {
    renderer = create(createElement(RemoteProvider, null, child ?? null));
  });
  return renderer;
}

beforeEach(() => {
  jest.clearAllMocks();
  mockCatalog.sessions = [];
  mockConnection.credentials = null;
  mockConnection.phase = "ready";
});
const credentials = { pairId: "pair", userJwt: "jwt" } as RemoteCredentials;

test("a settings event bumps the desktop revision and re-reads the settings", () => {
  const renderer = mount();
  try {
    act(() => mockHandleEvent({ type: "app_settings_changed", data: "{}" } as StreamEvent, "s1"));
    // The settings page keys its read off that revision, so a desktop-side
    // change must not be missed; the re-read is what makes the page agree.
    expect(mockCatalog.refreshSettings).toHaveBeenCalledTimes(1);

    act(() => mockHandleEvent({ type: "skills_changed", data: "{}" } as StreamEvent, "s1"));
    expect(mockCatalog.refreshSettings).toHaveBeenCalledTimes(1);

    act(() => mockHandleEvent({ type: "provider_config_changed", data: "{}" } as StreamEvent, "s1"));
    act(() => mockHandleEvent({ type: "model_visibility_changed", data: "{}" } as StreamEvent, "s1"));
    // Those two do bump the desktop revision and then fall through to the
    // session router; the settings-only events above returned early instead.
    expect(mockCatalog.refreshSettings).toHaveBeenCalledTimes(1);
    expect(mockCatalog.handleEvent).not.toHaveBeenCalled();
    expect(mockTimelineHandleEvent).toHaveBeenCalledTimes(2);
  } finally {
    act(() => renderer.unmount());
  }
});

test("an ordinary stream event reaches the session settings, the run observer and the timeline", () => {
  const renderer = mount();
  try {
    const event = { type: "model_changed", data: '{"model":"p/x"}' } as StreamEvent;
    act(() => mockHandleEvent(event, "s1"));
    // One event, three consumers: the routing must not short-circuit after the
    // first, or the transcript or the run bookkeeping would silently stop.
    expect(mockConversation.handleSessionSettingsEvent).toHaveBeenCalledWith(event, "s1");
    expect(mockCatalog.observeRunEvent).toHaveBeenCalledWith(event, "s1");
    expect(mockTimelineHandleEvent).toHaveBeenCalledWith(event, "s1");
  } finally {
    act(() => renderer.unmount());
  }
});

test("a finished run notifies only while the same desktop is still paired", () => {
  const renderer = mount();
  try {
    expect(catalogTaskFinished).toBeDefined();
    const session = { sessionId: "s1", threadId: "t1", title: "R", streaming: false } as RemoteSession;

    // Without credentials there is nobody to notify on behalf of.
    act(() => catalogTaskFinished!(session));
    expect(notifyTaskFinished).not.toHaveBeenCalled();

    mockConnection.credentials = credentials;
    act(() => renderer.unmount());
    const paired = mount();
    try {
      act(() => catalogTaskFinished!(session));
      expect(notifyTaskFinished).toHaveBeenCalledWith(session, expect.any(Function));
      // The guard the notification is given is the pairing identity: a task
      // that finishes after a re-pair must not be announced for the old one.
      const stillPaired = jest.mocked(notifyTaskFinished).mock.calls.at(-1)![1];
      expect(stillPaired()).toBe(true);
      // The ref is the pairing identity the connection hook maintains; a re-pair
      // replaces it, and the queued notification must not outlive its desktop.
      mockCredentialsRef!.current = { pairId: "other" } as RemoteCredentials;
      expect(stillPaired()).toBe(false);
    } finally {
      act(() => paired.unmount());
    }
  } finally {
    act(() => renderer.unmount());
  }
});

test("the notification channel is prepared once the connection is ready and paired", () => {
  mockConnection.credentials = credentials;
  const renderer = mount();
  try {
    expect(prepareTaskNotifications).toHaveBeenCalledTimes(1);
  } finally {
    act(() => renderer.unmount());
  }
});

test("closing a conversation clears the draft and refreshes the catalogue", () => {
  let consumer!: ReturnType<typeof useRemoteControls>;
  function Consumer() {
    consumer = useRemoteControls();
    return null;
  }
  const renderer = mount(createElement(Consumer));
  try {
    expect(conversationOptions.setDraft).toBeDefined();
    act(() => conversationOptions.setDraft!(true));
    expect(consumer.draft).toBe(true);
    act(() => consumer.closeConversation());
    // Leaving a conversation must not leave a draft armed for the next one,
    // and the catalogue is re-read so the list reflects what just happened.
    expect(consumer.draft).toBe(false);
    expect(consumer.selectedSessionId).toBe("");
    expect(mockCatalog.refreshSessions).toHaveBeenCalledTimes(1);
    expect(mockCatalog.refreshWorkspaces).toHaveBeenCalledTimes(1);
  } finally {
    act(() => renderer.unmount());
  }
});

test("resetting a conversation clears the draft without re-reading the catalogue", () => {
  let consumer!: ReturnType<typeof useRemoteControls>;
  function Consumer() {
    consumer = useRemoteControls();
    return null;
  }
  const renderer = mount(createElement(Consumer));
  try {
    act(() => conversationOptions.setDraft!(true));
    // The timeline controller owns this reset: a re-pair or a desktop switch
    // drops the open conversation without asking the *old* desktop for its
    // catalogue, which is what closeConversation is for instead.
    expect(connectionOptions.resetConversation).toBeDefined();
    act(() => connectionOptions.resetConversation!());
    expect(consumer.draft).toBe(false);
    expect(consumer.selectedSessionId).toBe("");
    expect(mockCatalog.refreshSessions).not.toHaveBeenCalled();
  } finally {
    act(() => renderer.unmount());
  }
});

test("the open conversation's title comes from the catalogue", () => {
  mockCatalog.sessions = [
    { sessionId: "s1", threadId: "t1", title: "The open one", streaming: false },
  ];
  const titles: string[] = [];
  function Consumer() {
    titles.push(useRemoteControls().selectedTitle);
    return null;
  }
  const renderer = mount(createElement(Consumer));
  try {
    // The header shows the desktop's title for the *selected* session, so the
    // lookup has to follow the id rather than the first row in the list.
    expect(titles.at(-1)).toBe("");
    expect(conversationOptions.setSelectedSessionId).toBeDefined();
    act(() => conversationOptions.setSelectedSessionId!("s1"));
    expect(titles.at(-1)).toBe("The open one");
  } finally {
    act(() => renderer.unmount());
  }
});

test("a session-state push reaches the conversation controls that own the settings", () => {
  const renderer = mount();
  try {
    expect(timelineOptions.onSessionState).toBeDefined();
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
  } finally {
    act(() => renderer.unmount());
  }
});

test("the transcript hook composes the controls with the timeline inside the provider", () => {
  const seen: { modelId?: string; timeline?: boolean }[] = [];
  function Transcript() {
    const value = useRemote();
    // `timeline` only exists on the merged value, so seeing it proves both
    // contexts were combined rather than the controls being returned alone.
    seen.push({ modelId: value.modelId, timeline: value.timeline !== undefined });
    return null;
  }
  const renderer = mount(createElement(Transcript));
  try {
    expect(seen.at(-1)).toEqual({ modelId: "", timeline: true });
  } finally {
    act(() => renderer.unmount());
  }
});

test("both context hooks refuse to be used outside the provider", () => {
  function Controls() {
    useRemoteControls();
    return null;
  }
  function Transcript() {
    useRemote();
    return null;
  }
  // A consumer outside the provider must fail loudly at render time: silently
  // returning a blank context would surface as "nothing loads" instead.
  for (const Component of [Controls, Transcript]) {
    const error = jest.spyOn(console, "error").mockImplementation(() => {});
    try {
      expect(() => act(() => { create(createElement(Component)); })).toThrow(/RemoteProvider/);
    } finally {
      error.mockRestore();
    }
  }
});
