import { createElement, useEffect, type ReactElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { RemoteProvider, useRemote, useRemoteControls } from "../RemoteContext";
import type { DesktopRuntime } from "../useDesktopRuntime";

/**
 * A streamed text delta must not re-render every control on the screen.
 *
 * The transcript is the part that has to follow the stream; the composer, the
 * menus and the list are not. The provider therefore publishes two contexts,
 * and this pins the property that keeps them apart: a timeline that changes
 * hands the transcript a new value and the controls the same one, while
 * something the controls do read — whether the open machine is streaming, or
 * which machine it is — reaches them.
 *
 * With a runtime per desktop this is the part of the old single-runtime wiring
 * easiest to lose: the provider now reads its value out of a runtime object
 * that is rebuilt on every render, so depending on "the whole object" is
 * exactly the mistake that would break it.
 *
 * The children are created once, as a real app creates them once, so a
 * re-render observed here is one a context change *caused* — not the harness
 * rebuilding its own tree.
 */

const mockRuntime = {} as DesktopRuntime;
let forceRuntimeRender: () => void = () => undefined;

/** Stable across renders, as the real hook's callbacks are. */
const mockPairedDesktops = {
  booting: false,
  desktops: [{ desktopId: "desk-a", pairId: "pair-a" }],
  activeDesktopId: "desk-a",
  generations: {},
  error: null,
  pair: jest.fn(async () => {}),
  unpair: jest.fn(async () => {}),
  removeDesktop: jest.fn(async () => {}),
  renameDesktop: jest.fn(async () => {}),
  labelDesktop: jest.fn(async () => {}),
  selectDesktop: jest.fn(async () => {}),
  reconnect: jest.fn(),
  drainRevokes: jest.fn(async () => {}),
  clearError: jest.fn(),
};

jest.mock("../usePairedDesktops", () => ({
  // A fresh surface every render, exactly as the real hook returns: its
  // callbacks are stable, the object that carries them is not. Reading the
  // object itself in a dependency list is the mistake this guards against.
  usePairedDesktops: () => ({ ...mockPairedDesktops }),
}));

jest.mock("../useDesktopRuntime", () => {
  const React = jest.requireActual<typeof import("react")>("react");
  return {
    // A runtime object rebuilt on every render, as a real one is, with whatever
    // the test has mutated in `mockRuntime`'s leaves.
    useDesktopRuntime: () => {
      const [, setTick] = React.useState(0);
      forceRuntimeRender = () => setTick((value: number) => value + 1);
      return { ...mockRuntime };
    },
  };
});

jest.mock("../../notifications/taskNotifications", () => ({
  __esModule: true,
  notifyTaskFinished: jest.fn(),
  prepareTaskNotifications: jest.fn(async () => {}),
}));

jest.mock("expo-network", () => ({
  __esModule: true,
  NetworkStateType: { NONE: "NONE", UNKNOWN: "UNKNOWN", WIFI: "WIFI", CELLULAR: "CELLULAR", OTHER: "OTHER" },
  getNetworkStateAsync: jest.fn(async () => ({
    type: "WIFI", isConnected: true, isInternetReachable: true,
  })),
  addNetworkStateListener: jest.fn(() => ({ remove: jest.fn() })),
}));

jest.mock("react-native", () => ({
  __esModule: true,
  AppState: {
    currentState: "active",
    addEventListener: jest.fn(() => ({ remove: jest.fn() })),
  },
  Platform: {
    OS: "ios",
    select: (specifics: Record<string, unknown>) =>
      specifics?.ios ?? specifics?.native ?? specifics?.default,
  },
  TurboModuleRegistry: {
    get: () => null,
    getEnforcing: () => {
      throw new Error("native module not found");
    },
  },
  NativeEventEmitter: class {},
}));

/**
 * Render counts, recorded in an effect rather than during render: the rule that
 * forbids writing to a component-external binding mid-render is the same one
 * that keeps this measurement honest — an effect runs exactly once per commit.
 */
const renders = { controls: 0, transcripts: 0 };

function Controls(): null {
  const controls = useRemoteControls();
  useEffect(() => { renders.controls += 1; });
  void controls.phase;
  return null;
}

function Transcript(): null {
  const value = useRemote();
  useEffect(() => { renders.transcripts += 1; });
  void value.timeline;
  return null;
}

/** One streamed delta, as the timeline controller reports it. */
function streamDelta(index: number): void {
  // A new controller object each time, as the real hook returns: the provider
  // compares the leaves of what it published, so mutating in place would be a
  // change no reader could see.
  const runtime = mockRuntime as unknown as { timeline: Record<string, unknown> };
  runtime.timeline = { ...runtime.timeline, timeline: { items: [], index } };
}

/** The runtime stand-in, with only the fields the provider reads. */
function seedRuntime(): void {
  const noop = jest.fn(async () => {});
  Object.assign(mockRuntime as unknown as Record<string, unknown>, {
    desktopId: "desk-a",
    label: "Studio",
    connection: {
      phase: "ready",
      error: null,
      credentials: { pairId: "pair-a", expectedDesktopId: "desk-a" },
      presence: { agentAvailable: true },
      desktopOnline: true,
      hasConnectedContent: true,
      capabilities: new Set<string>(),
      fileTransferSupported: false,
      promptReceiptSupported: false,
      clearError: jest.fn(),
    },
    catalog: {
      sessions: [],
      workspaces: [],
      models: [],
      unreadSessions: new Set<string>(),
      catalogSync: { sessions: "ready", workspaces: "ready", models: "ready", settings: "ready" },
      approvalTier: "off",
      sandboxAvailable: false,
      refreshSessions: noop,
      refreshWorkspaces: noop,
      refreshModels: noop,
      rename: noop,
      generateTitle: noop,
      createWorkspace: noop,
      setSessionPinned: noop,
      setWorkspacePinned: noop,
      renameWorkspace: noop,
    },
    timeline: {
      timeline: { items: [] },
      timelinePending: false,
      timelineSyncStatus: "idle",
      timelineError: null,
      canLoadOlderTimeline: false,
      loadingOlderTimeline: false,
      loadOlderTimeline: jest.fn(),
      reloadTimeline: jest.fn(),
      retryTimeline: jest.fn(),
      awaitCompactionOutcome: jest.fn(),
    },
    conversation: {
      modelId: "",
      thinkingLevel: "off",
      sessionUsage: null,
      selectSession: noop,
      newConversation: noop,
      listSkills: noop,
      listSessionFiles: noop,
      resolveToolCallTarget: noop,
      prepareAttachment: noop,
      cachedAttachment: jest.fn(() => null),
      downloadAttachment: noop,
      abort: noop,
      compactContext: noop,
      setModel: noop,
      setThinkingLevel: noop,
      setApprovalTier: noop,
      deleteSession: noop,
      deleteWorkspace: noop,
      forkConversation: noop,
      decideApproval: noop,
      refreshSessionUsage: noop,
    },
    outbox: { sending: false, sendMessage: noop, continueRun: noop },
    management: {},
    selectedSessionId: "",
    selectedTitle: "",
    draft: false,
    draftMode: "chat",
    draftWorkspaceId: "",
    streaming: false,
    compacting: false,
    busy: false,
    captureConversationOperation: () => () => false,
    closeConversation: jest.fn(),
    desktopSettingsRevision: 0,
    skillsRevision: 0,
  });
}

describe("render isolation with a runtime per desktop", () => {
  let renderer: ReactTestRenderer | null;

  /**
   * Mounted with elements created *once*, as an app creates its tree once: an
   * element React sees again is one it can skip, so a re-render counted here is
   * one a context change caused rather than the harness rebuilding itself.
   */
  async function mount({ controls = false, transcript = false }): Promise<void> {
    const children: ReactElement[] = [];
    if (controls) children.push(createElement(Controls));
    if (transcript) children.push(createElement(Transcript));
    await act(async () => {
      renderer = create(createElement(RemoteProvider, null, ...children));
    });
  }

  /** Let the runtime's host re-render, as its own state change would. */
  async function tick(): Promise<void> {
    await act(async () => { forceRuntimeRender(); });
  }

  beforeEach(() => {
    seedRuntime();
    renders.controls = 0;
    renders.transcripts = 0;
    renderer = null;
  });

  afterEach(async () => {
    if (renderer) {
      await act(async () => { renderer!.unmount(); });
      renderer = null;
    }
  });

  test("100 streamed deltas re-render the transcript and not the controls", async () => {
    await mount({ transcript: true });
    const transcriptBefore = renders.transcripts;

    for (let index = 0; index < 100; index += 1) {
      streamDelta(index);
      await tick();
    }

    expect(renders.transcripts).toBeGreaterThan(transcriptBefore);
  });

  test("a streamed delta leaves a control-only consumer alone", async () => {
    await mount({ controls: true, transcript: true });
    const controlsBefore = renders.controls;

    for (let index = 0; index < 100; index += 1) {
      streamDelta(index);
      await tick();
    }

    expect(renders.transcripts).toBeGreaterThan(1);
    // The composer and the menus read controls, and a reply arriving one token
    // at a time is not a reason for any of them to re-render.
    expect(renders.controls).toBe(controlsBefore);
  });

  test("a machine starting to stream does reach the controls", async () => {
    await mount({ controls: true });
    const controlsBefore = renders.controls;
    (mockRuntime as unknown as { streaming: boolean }).streaming = true;
    await tick();
    expect(renders.controls).toBeGreaterThan(controlsBefore);
  });

  test("the machine being read changing reaches the controls", async () => {
    await mount({ controls: true });
    const controlsBefore = renders.controls;
    // Replaced rather than mutated, as a runtime replaces it: the provider
    // compares these fields by identity.
    (mockRuntime as unknown as { connection: Record<string, unknown> }).connection = {
      ...(mockRuntime as unknown as { connection: Record<string, unknown> }).connection,
      credentials: { pairId: "pair-b", expectedDesktopId: "desk-b" },
    };
    await tick();
    expect(renders.controls).toBeGreaterThan(controlsBefore);
  });
});
