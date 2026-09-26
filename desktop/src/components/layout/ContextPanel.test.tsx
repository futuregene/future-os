// @vitest-environment jsdom
import type { ReactElement } from "react";
import type {
  GitReview,
  StoredArtifact,
  StoredRun,
  StoredThread,
  StoredToolCall,
  StoredWorkspace,
} from "../../integrations/storage/threadStore";
import type { WorkspaceReviewCapabilities } from "../../integrations/storage/types";
import type { ContextTab } from "./hooks/useContextData";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ContextPanel } from "./ContextPanel";

// React only flushes state updates inside `act` when it is told the environment
// supports it; without this the selections below would never commit.
(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Props each child panel last received, and the input the data hook was given. */
const panels = vi.hoisted(() => ({
  context: null as unknown,
  detail: null as unknown,
  fileArtifacts: null as unknown,
  fileTree: null as unknown,
  inspect: null as unknown,
  review: null as unknown,
  runs: null as unknown,
}));

const mocks = vi.hoisted(() => ({
  abortRun: vi.fn(async (_input: unknown) => {}),
  archiveFinishedRuns: vi.fn(async (_threadId: string) => {}),
  data: {} as Record<string, unknown>,
  refreshContext: vi.fn(async () => {}),
}));

vi.mock("../../integrations/storage/threadStore", () => ({
  abortRun: (input: unknown) => mocks.abortRun(input),
  archiveFinishedRuns: (threadId: string) => mocks.archiveFinishedRuns(threadId),
}));

vi.mock("./hooks/useContextData", async () => {
  const { createElement } = await import("react");
  void createElement;
  return {
    useContextData: (input: unknown) => {
      panels.context = input;
      return {
        artifacts: [],
        gitReviews: { branch: null, uncommitted: null },
        loading: false,
        refreshContext: mocks.refreshContext,
        reviewCapabilities: null,
        runs: [],
        runsScope: null,
        toolsByRun: {},
        ...mocks.data,
      };
    },
  };
});

vi.mock("../../lib/windowDrag", () => ({ startWindowDrag: vi.fn() }));

vi.mock("../../features/runs/RunsPanel", async () => {
  const { createElement } = await import("react");
  return {
    RunsPanel: (props: unknown) => {
      panels.runs = props;
      return createElement("div", { "data-panel": "runs" });
    },
  };
});
vi.mock("../../features/runs/RunInspectPanel", async () => {
  const { createElement } = await import("react");
  return {
    RunInspectPanel: (props: unknown) => {
      panels.inspect = props;
      return createElement("div", { "data-panel": "inspect" });
    },
  };
});
vi.mock("../../features/review/ReviewPanel", async () => {
  const { createElement } = await import("react");
  return {
    ReviewPanel: (props: unknown) => {
      panels.review = props;
      return createElement("div", { "data-panel": "review" });
    },
  };
});
vi.mock("../../features/artifacts/ArtifactsPanel", async () => {
  const { createElement } = await import("react");
  return {
    ArtifactsPanel: (props: unknown) => {
      panels.fileArtifacts = props;
      return createElement("div", { "data-panel": "artifacts" });
    },
  };
});
vi.mock("../../features/artifacts/ArtifactDetailPanel", async () => {
  const { createElement } = await import("react");
  return {
    ArtifactDetailPanel: (props: unknown) => {
      panels.detail = props;
      return createElement("div", { "data-panel": "artifact-detail" });
    },
  };
});
vi.mock("../../features/filetree/FileTreePanel", async () => {
  const { createElement } = await import("react");
  return {
    FileTreePanel: (props: unknown) => {
      panels.fileTree = props;
      return createElement("div", { "data-panel": "filetree" });
    },
  };
});

interface RecordedProps {
  onArchiveFinished?: (threadId: string) => void;
  onBack?: () => void;
  onChanged?: () => void;
  onInspectTool?: (toolId: string) => void;
  onSelectArtifact?: (artifactId: string) => void;
  onTerminateRun?: (threadId: string, run: StoredRun) => void;
  [key: string]: unknown;
}

/** Props recorded by the stub that renders `data-panel="<key>"`. */
const recorded = (name: keyof typeof panels) => panels[name] as RecordedProps;

function thread(id: string, overrides: Partial<StoredThread> = {}): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "chat",
    workspaceId: `w-${id}`,
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
    ...overrides,
  };
}

const chatThread = thread("t1", { workspaceId: "w-t1" });
const chatWorkspace = { id: "w-t1", kind: "user", name: "Chat", path: "/tmp/chat" } as unknown as StoredWorkspace;
const run = { id: "r1", status: "completed" } as unknown as StoredRun;
const tool = { id: "tool-1", runId: "r1" } as unknown as StoredToolCall;
const artifact = { id: "art-1", name: "out.md" } as unknown as StoredArtifact;
const overview = { files: [{ path: "src/a.ts" }] } as unknown as GitReview;
const capabilities = { changePreview: "unsupported", isGitWorkspace: true } as unknown as WorkspaceReviewCapabilities;

function mount(node: ReactElement) {
  const container = document.createElement("div");
  document.body.appendChild(container);
  const root = createRoot(container);
  act(() => root.render(node));
  return {
    container,
    rerender: (next: ReactElement) => act(() => root.render(next)),
    unmount: () => {
      act(() => root.unmount());
      container.remove();
    },
  };
}

function baseProps(overrides: Partial<Parameters<typeof ContextPanel>[0]> = {}): Parameters<typeof ContextPanel>[0] {
  return {
    activeTab: "files" as ContextTab,
    activeThread: chatThread,
    activeWorkspace: chatWorkspace,
    expanded: true,
    onResizeNudge: vi.fn(),
    onResizeStart: vi.fn(),
    onTabChange: vi.fn(),
    onToggleExpanded: vi.fn(),
    width: 360,
    ...overrides,
  };
}

type PanelProps = Parameters<typeof ContextPanel>[0];

beforeEach(() => {
  panels.context = null;
  panels.detail = null;
  panels.fileArtifacts = null;
  panels.fileTree = null;
  panels.inspect = null;
  panels.review = null;
  panels.runs = null;
  mocks.abortRun.mockClear();
  mocks.archiveFinishedRuns.mockClear();
  mocks.data = {};
  mocks.refreshContext.mockClear();
});

describe("context panel shell", () => {
  it("collapses to a floating expand button", () => {
    const p = baseProps({ expanded: false });
    const view = mount(<ContextPanel {...p} />);
    expect(view.container.querySelector("aside")).toBeNull();
    const expand = view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Expand context panel\"]")!;
    act(() => expand.click());
    expect(p.onToggleExpanded).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("renders the aside at the given width and lets the header collapse it", () => {
    const p = baseProps({ width: 412 });
    const view = mount(<ContextPanel {...p} />);
    expect(view.container.querySelector<HTMLElement>("aside")!.style.width).toBe("412px");
    act(() => view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Collapse context panel\"]")!.click());
    expect(p.onToggleExpanded).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("offers only the non-git tabs for a chat thread and routes the picker", () => {
    const p = baseProps();
    const view = mount(<ContextPanel {...p} />);
    const select = view.container.querySelector<HTMLSelectElement>("select#context-panel-view")!;
    expect([...select.options].map(option => option.value)).toEqual(["files", "runs"]);
    act(() => {
      select.value = "runs";
      select.dispatchEvent(new Event("change", { bubbles: true }));
    });
    expect(p.onTabChange).toHaveBeenCalledWith("runs");
    view.unmount();
  });

  it("adds the Review tab once a workspace thread's capabilities land", () => {
    mocks.data = { reviewCapabilities: capabilities };
    const p = baseProps({ activeThread: thread("t1", { mode: "workspace", workspaceId: "w-t1" }) });
    const view = mount(<ContextPanel {...p} />);
    expect([...view.container.querySelectorAll<HTMLOptionElement>("option")].map(option => option.value))
      .toEqual(["files", "runs", "review"]);
    view.unmount();
  });

  it("keeps the pre-capability tab set while a workspace thread's kind is still resolving", () => {
    const p = baseProps({ activeTab: "review", activeThread: thread("t1", { mode: "workspace", workspaceId: "w-t1" }) });
    const view = mount(<ContextPanel {...p} />);
    // The Review tab is not offered yet, so the panel falls back to the first tab
    // instead of rendering a panel no tab can reach.
    expect([...view.container.querySelectorAll<HTMLOptionElement>("option")].map(option => option.value))
      .toEqual(["files", "runs"]);
    expect(p.onTabChange).toHaveBeenCalledWith("files");
    view.unmount();
  });

  it("falls back to the first tab whenever the current tab is not offered", () => {
    const p = baseProps({ activeTab: "review" });
    mount(<ContextPanel {...p} />);
    expect(p.onTabChange).toHaveBeenCalledWith("files");
  });
});

describe("context panel states", () => {
  it("shows the loading notice only until some context data exists", () => {
    mocks.data = { loading: true };
    const empty = mount(<ContextPanel {...baseProps()} />);
    expect(empty.container.textContent).toContain("Loading context...");
    empty.unmount();

    for (const data of [
      { loading: true, runs: [run] },
      { loading: true, artifacts: [artifact] },
      { loading: true, gitReviews: { branch: overview, uncommitted: null } },
      { loading: true, gitReviews: { branch: null, uncommitted: overview } },
    ]) {
      mocks.data = data;
      const withData = mount(<ContextPanel {...baseProps()} />);
      expect(withData.container.textContent).not.toContain("Loading context...");
      withData.unmount();
    }
  });

  it("asks for a thread when none is selected", () => {
    const view = mount(<ContextPanel {...baseProps({ activeThread: null, activeWorkspace: null })} />);
    expect(view.container.textContent).toContain("No thread selected");
    expect(view.container.querySelector("[data-panel=\"filetree\"]")).toBeNull();
    view.unmount();
  });

  it("passes the file tree its root and workspace flag", () => {
    const view = mount(<ContextPanel {...baseProps()} />);
    expect(recorded("fileTree")).toMatchObject({ isWorkspace: false, rootPath: "/tmp/chat" });
    view.unmount();

    mocks.data = { reviewCapabilities: capabilities };
    const workspace = mount(<ContextPanel {...baseProps({ activeThread: thread("t1", { mode: "workspace", workspaceId: "w-t1" }) })} />);
    expect(recorded("fileTree")).toMatchObject({ isWorkspace: true, rootPath: "/tmp/chat" });
    workspace.unmount();
  });

  it("hands the file tree a null root when there is no active workspace", () => {
    // A chat thread with no workspace resolved (or a workspace that was just
    // deleted) has no path to hand down — the tree must get `null`, not the
    // previous workspace's path kept in a stale closure.
    mocks.data = { reviewCapabilities: capabilities };
    const view = mount(<ContextPanel {...baseProps({ activeWorkspace: null })} />);
    expect(recorded("fileTree")).toMatchObject({ isWorkspace: false, rootPath: null });

    // The artifacts tab passes the same workspace path down, so it needs the
    // same null-safe treatment ("reveal in folder" has nothing to reveal).
    view.rerender(<ContextPanel {...baseProps({ activeTab: "artifacts", activeWorkspace: null })} />);
    expect(recorded("fileArtifacts")).toMatchObject({ workspacePath: null });
    view.unmount();
  });

  it("withholds a stale workspace path while the thread's workspace is still resolving", () => {
    mount(
      <ContextPanel {...baseProps({
        activeWorkspace: { ...chatWorkspace, id: "w-other", path: "/tmp/other" } as StoredWorkspace,
      })}
      />,
    );
    expect(recorded("context")).toMatchObject({ activeWorkspacePath: null, workspaceScopeReady: false });
  });

  it("passes the active thread's scope identity through to the data hook", () => {
    mount(<ContextPanel {...baseProps()} />);
    expect(recorded("context")).toMatchObject({
      activeThreadId: "t1",
      activeThreadMode: "chat",
      activeWorkspaceId: "w-t1",
      activeWorkspacePath: "/tmp/chat",
      workspaceScopeReady: true,
    });
  });
});

describe("context panel runs tab", () => {
  const runsData = {
    runs: [run],
    runsScope: { threadId: "t1", workspaceId: "w-t1", workspacePath: "/tmp/chat" },
    toolsByRun: { r1: [tool] },
  };

  it("hands the runs snapshot to the panel and archives/terminates through the store", async () => {
    mocks.data = runsData;
    const view = mount(<ContextPanel {...baseProps({ activeTab: "runs" })} />);
    expect(recorded("runs")).toMatchObject({ runs: [run], scope: runsData.runsScope, toolsByRun: { r1: [tool] } });

    await act(async () => {
      recorded("runs").onTerminateRun!("t1", run);
    });
    expect(mocks.abortRun).toHaveBeenCalledWith({ runId: "r1", threadId: "t1" });
    expect(mocks.refreshContext).toHaveBeenCalledTimes(1);

    await act(async () => {
      recorded("runs").onArchiveFinished!("t1");
    });
    expect(mocks.archiveFinishedRuns).toHaveBeenCalledWith("t1");
    expect(mocks.refreshContext).toHaveBeenCalledTimes(2);
    view.unmount();
  });

  it("drills into a tool call and comes back", () => {
    mocks.data = runsData;
    const onTabChange = vi.fn();
    const p = baseProps({ activeTab: "runs", onTabChange });
    const view = mount(<ContextPanel {...p} />);
    // The open-time seed may have already asked for Files; only the selection's
    // own tab request matters from here.
    onTabChange.mockClear();

    act(() => recorded("runs").onInspectTool!("tool-1"));
    expect(view.container.querySelector("[data-panel=\"inspect\"]")).not.toBeNull();
    expect(recorded("inspect")).toMatchObject({ compact: true, run, tools: [tool] });
    // Already on the runs tab, so the selection must not re-issue a tab change.
    expect(onTabChange).not.toHaveBeenCalled();

    act(() => recorded("inspect").onBack!());
    expect(view.container.querySelector("[data-panel=\"runs\"]")).not.toBeNull();
    view.unmount();
  });

  it("shows no inspect panel when the tools' run is gone from the snapshot", () => {
    // The selected tool resolves through its run, and the run list can move on
    // (archived / filtered) while a tool selection survives. The lookup must
    // then yield nothing rather than half an inspect panel.
    mocks.data = {
      runs: [],
      runsScope: { threadId: "t1", workspaceId: "w-t1", workspacePath: "/tmp/chat" },
      toolsByRun: { "ghost-run": [{ id: "tool-1", runId: "ghost-run" } as unknown as StoredToolCall] },
    };
    const view = mount(<ContextPanel {...baseProps({ activeTab: "runs" })} />);

    act(() => recorded("runs").onInspectTool!("tool-1"));

    expect(view.container.querySelector("[data-panel=\"inspect\"]")).toBeNull();
    expect(recorded("inspect")).toBeNull();
    view.unmount();
  });

  it("switches to the runs tab when a run is inspected from another tab", () => {
    mocks.data = runsData;
    const p = baseProps({ activeTab: "files" });
    mount(<ContextPanel {...p} />);
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-run", { detail: { runId: "r1" } }));
    });
    expect(p.onTabChange).toHaveBeenCalledWith("runs");
  });

  it("does not re-issue the tab change for an inspect-run event while already on runs", () => {
    // The other side of the guard in `handleSelectRun`: opening a run from a
    // notification while the user is *already* looking at the runs tab must not
    // push a redundant tab change (which would fight a later manual tab click).
    mocks.data = runsData;
    const onTabChange = vi.fn();
    const p = baseProps({ activeTab: "runs", onTabChange });
    const view = mount(<ContextPanel {...p} />);
    onTabChange.mockClear();

    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-run", { detail: { runId: "r1" } }));
    });

    expect(p.onTabChange).not.toHaveBeenCalled();
    // The run is still selected — only the redundant tab change is suppressed.
    expect(view.container.querySelector("[data-panel=\"runs\"]")).not.toBeNull();
    view.unmount();
  });

  it("surfaces the tool id of an inspect-tool event even before the run data lands", () => {
    mocks.data = runsData;
    const p = baseProps({ activeTab: "runs" });
    const view = mount(<ContextPanel {...p} />);
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-tool", { detail: { runId: "r1", toolId: "tool-1" } }));
    });
    expect(recorded("inspect")).toMatchObject({ run, tools: [tool] });
    view.unmount();
  });

  it("resolves the tool from the first run that owns it when several do", () => {
    // Two runs both containing the same tool id: the reduce must stop scanning
    // once it has a match instead of overwriting it with a later run.
    const secondRun = { id: "r2", status: "completed" } as unknown as StoredRun;
    mocks.data = { runs: [run, secondRun], toolsByRun: { r1: [tool], r2: [tool] } };
    const view = mount(<ContextPanel {...baseProps({ activeTab: "runs" })} />);
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-tool", { detail: { runId: "r1", toolId: "tool-1" } }));
    });
    expect(recorded("inspect")).toMatchObject({ run, tools: [tool] });
    view.unmount();
  });

  it("shows the loading notice while an inspected tool has not resolved yet", () => {
    mocks.data = runsData;
    const view = mount(<ContextPanel {...baseProps({ activeTab: "runs" })} />);
    // A tool id that no run owns: the panel must not render an empty inspector.
    act(() => recorded("runs").onInspectTool!("missing-tool"));
    expect(view.container.querySelector("[data-panel=\"inspect\"]")).toBeNull();
    expect(view.container.textContent).toContain("Loading context...");
    view.unmount();
  });

  it("clears the inspected tool when the active thread changes", () => {
    mocks.data = runsData;
    const p = baseProps({ activeTab: "runs" });
    const view = mount(<ContextPanel {...p} />);
    act(() => recorded("runs").onInspectTool!("tool-1"));
    expect(view.container.querySelector("[data-panel=\"inspect\"]")).not.toBeNull();

    view.rerender(<ContextPanel {...p} activeThread={thread("t2", { workspaceId: "w-t2" })} />);
    expect(view.container.querySelector("[data-panel=\"runs\"]")).not.toBeNull();
    view.unmount();
  });
});

describe("context panel review tab", () => {
  const workspaceThread = thread("t1", { mode: "workspace", workspaceId: "w-t1" });

  it("passes both reviews and the capability verdict to the review panel", () => {
    mocks.data = {
      gitReviews: { branch: overview, uncommitted: overview },
      reviewCapabilities: capabilities,
    };
    mount(<ContextPanel {...baseProps({ activeTab: "review", activeThread: workspaceThread })} />);
    expect(recorded("review")).toMatchObject({
      branchReview: overview,
      changePreview: "unsupported",
      isGitWorkspace: true,
      threadId: "t1",
      uncommittedReview: overview,
    });
  });

  it("defaults the preview to ready while capabilities are unknown", () => {
    mount(<ContextPanel {...baseProps({ activeTab: "review", activeThread: workspaceThread })} />);
    expect(recorded("review")).toMatchObject({ changePreview: "ready", isGitWorkspace: null });
  });

  it("renders the review panel with both reviews absent", () => {
    mocks.data = { reviewCapabilities: capabilities };
    mount(<ContextPanel {...baseProps({ activeTab: "review", activeThread: workspaceThread })} />);
    expect(recorded("review")).toMatchObject({ branchReview: null, uncommittedReview: null });
  });
});

describe("context panel artifacts tab", () => {
  it("lists artifacts and opens one on selection", async () => {
    mocks.data = { artifacts: [artifact] };
    const onTabChange = vi.fn();
    const p = baseProps({ activeTab: "artifacts", onTabChange });
    const view = mount(<ContextPanel {...p} />);
    expect(recorded("fileArtifacts")).toMatchObject({ artifacts: [artifact], threadId: "t1", workspacePath: "/tmp/chat" });
    onTabChange.mockClear();

    act(() => recorded("fileArtifacts").onSelectArtifact!("art-1"));
    expect(view.container.querySelector("[data-panel=\"artifact-detail\"]")).not.toBeNull();
    expect(recorded("detail")).toMatchObject({ artifact });
    // Already on the artifacts tab, so no tab change is issued.
    expect(p.onTabChange).not.toHaveBeenCalled();

    // `onChanged` is the async refresh: await it so the act scope closes before
    // the next interaction (an un-awaited async act poisons the next one).
    await act(async () => {
      await recorded("detail").onChanged!();
    });
    expect(mocks.refreshContext).toHaveBeenCalled();

    act(() => recorded("detail").onBack!());
    expect(view.container.querySelector("[data-panel=\"artifacts\"]")).not.toBeNull();
    view.unmount();
  });

  it("opens an artifact pushed by the inspect-artifact event", () => {
    mocks.data = { artifacts: [artifact] };
    const p = baseProps({ activeTab: "files" });
    const view = mount(<ContextPanel {...p} />);
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-artifact", { detail: { artifactId: "art-1" } }));
    });
    expect(p.onTabChange).toHaveBeenCalledWith("artifacts");

    // The caller owns the tab state, so committing its request is what actually
    // reveals the artifact the event selected.
    view.rerender(<ContextPanel {...p} activeTab="artifacts" />);
    expect(view.container.querySelector("[data-panel=\"artifact-detail\"]")).not.toBeNull();
    expect(recorded("detail")).toMatchObject({ artifact });
    view.unmount();
  });

  it("ignores a selected artifact that is no longer in the list", () => {
    mocks.data = { artifacts: [] };
    const view = mount(<ContextPanel {...baseProps({ activeTab: "artifacts" })} />);
    act(() => recorded("fileArtifacts").onSelectArtifact!("gone"));
    expect(view.container.querySelector("[data-panel=\"artifact-detail\"]")).toBeNull();
    expect(view.container.querySelector("[data-panel=\"artifacts\"]")).not.toBeNull();
    view.unmount();
  });

  it("opens the panel from an inspect-artifact event while collapsed", () => {
    mocks.data = { artifacts: [artifact] };
    const p = baseProps({ expanded: false });
    mount(<ContextPanel {...p} />);
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-artifact", { detail: { artifactId: "art-1" } }));
    });
    expect(p.onToggleExpanded).toHaveBeenCalledTimes(1);
    expect(p.onTabChange).toHaveBeenCalledWith("artifacts");
  });
});

describe("context panel resize divider", () => {
  it("nudges in and out with the arrow keys and ignores everything else", () => {
    const p = baseProps();
    const view = mount(<ContextPanel {...p} />);
    const divider = view.container.querySelector<HTMLElement>("[role=\"separator\"]")!;
    expect(divider.getAttribute("aria-orientation")).toBe("vertical");
    expect(divider.getAttribute("aria-label")).toBe("Drag to resize panel");

    act(() => {
      divider.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowLeft" }));
    });
    expect(p.onResizeNudge).toHaveBeenCalledWith(16);
    act(() => {
      divider.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "ArrowRight" }));
    });
    expect(p.onResizeNudge).toHaveBeenLastCalledWith(-16);
    act(() => {
      divider.dispatchEvent(new KeyboardEvent("keydown", { bubbles: true, key: "Enter" }));
    });
    expect(p.onResizeNudge).toHaveBeenCalledTimes(2);

    act(() => {
      divider.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(p.onResizeStart).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("keeps the divider reachable by keyboard, not just by pointer", () => {
    // `tabIndex={0}` is the whole reason the arrow-key path above is reachable
    // without a mouse: a separator that is not focusable is a drag-only handle.
    // The a11y dimension of this task asks for exactly this kind of assertion,
    // and it was previously only implied by the keyboard test.
    const view = mount(<ContextPanel {...baseProps()} />);
    const divider = view.container.querySelector<HTMLElement>("[role=\"separator\"]")!;

    expect(divider.tabIndex).toBe(0);
    divider.focus();
    // Focus actually lands on it, so a keyboard user can reach and operate it.
    expect(document.activeElement).toBe(divider);
    expect(divider.getAttribute("aria-orientation")).toBe("vertical");
    view.unmount();
  });
});

describe("context panel tab seeding", () => {
  it("seeds Files once per open and does not re-seed on a mid-open thread switch", () => {
    const p = baseProps({ activeTab: "runs" });
    const view = mount(<ContextPanel {...p} />);
    expect(p.onTabChange).toHaveBeenCalledTimes(1);
    expect(p.onTabChange).toHaveBeenCalledWith("files");

    view.rerender(<ContextPanel {...p} activeThread={thread("t2", { workspaceId: "w-t2" })} />);
    expect(p.onTabChange).toHaveBeenCalledTimes(1);
    view.unmount();
  });

  it("does not seed while the panel is closed, then seeds on the next open", () => {
    const p = baseProps({ activeTab: "files" });
    const view = mount(<ContextPanel {...p} />);
    expect(p.onTabChange).not.toHaveBeenCalled();

    view.rerender(<ContextPanel {...p} expanded={false} />);
    view.rerender(<ContextPanel {...p} activeTab="runs" expanded />);
    expect(p.onTabChange).toHaveBeenCalledWith("files");
    view.unmount();
  });

  it("leaves the tab alone when the open was caused by a tool inspection", () => {
    mocks.data = { runs: [run], toolsByRun: { r1: [tool] } };
    const onTabChange = vi.fn();
    const p = baseProps({ activeTab: "files", onTabChange });
    const view = mount(<ContextPanel {...p} expanded={false} />);

    // A tool inspection opens the panel: it picks the Runs tab itself, and the
    // seeding effect must not override that choice once the reopen commits.
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-tool", { detail: { runId: "r1", toolId: "tool-1" } }));
    });
    expect(onTabChange).toHaveBeenCalledWith("runs");
    expect(p.onToggleExpanded).toHaveBeenCalledTimes(1);

    onTabChange.mockClear();
    view.rerender(<ContextPanel {...p} activeTab="runs" expanded />);
    expect(p.onTabChange).not.toHaveBeenCalled();
    view.unmount();
  });
});

describe("context panel inspect events", () => {
  it("opens the panel and selects a run on inspect-run", () => {
    mocks.data = { runs: [run], toolsByRun: { r1: [tool] } };
    const p = baseProps({ activeTab: "files" });
    mount(<ContextPanel {...p} expanded={false} />);
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-run", { detail: { runId: "r1" } }));
    });
    expect(p.onTabChange).toHaveBeenCalledWith("runs");
    expect(p.onToggleExpanded).toHaveBeenCalledTimes(1);
  });

  it("does not re-expand an already open panel", () => {
    const p = baseProps();
    mount(<ContextPanel {...p} />);
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-run", { detail: { runId: "r1" } }));
    });
    expect(p.onToggleExpanded).not.toHaveBeenCalled();
  });

  it("switches to Review and clears any selection on open-review", () => {
    mocks.data = { runs: [run], toolsByRun: { r1: [tool] }, reviewCapabilities: capabilities };
    const p = baseProps({ activeTab: "files", activeThread: thread("t1", { mode: "workspace", workspaceId: "w-t1" }) });
    const view = mount(<ContextPanel {...p} />);

    // Select a tool to inspect, then commit the tab that request asked for.
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-tool", { detail: { runId: "r1", toolId: "tool-1" } }));
    });
    view.rerender(<ContextPanel {...p} activeTab="runs" />);
    expect(view.container.querySelector("[data-panel=\"inspect\"]")).not.toBeNull();

    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:open-review", { detail: undefined }));
    });
    expect(p.onTabChange).toHaveBeenLastCalledWith("review");

    view.rerender(<ContextPanel {...p} activeTab="review" />);
    expect(view.container.querySelector("[data-panel=\"inspect\"]")).toBeNull();
    expect(view.container.querySelector("[data-panel=\"review\"]")).not.toBeNull();
    view.unmount();
  });

  it("opens the panel from open-review while collapsed", () => {
    const p = baseProps({ expanded: false });
    mount(<ContextPanel {...p} />);
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:open-review", { detail: undefined }));
    });
    expect(p.onToggleExpanded).toHaveBeenCalledTimes(1);
    expect(p.onTabChange).toHaveBeenCalledWith("review");
  });

  it("stops listening for inspect events once unmounted", () => {
    const p = baseProps();
    const view = mount(<ContextPanel {...p} />);
    view.unmount();
    act(() => {
      window.dispatchEvent(new CustomEvent("futureos:inspect-run", { detail: { runId: "r1" } }));
    });
    expect(p.onToggleExpanded).not.toHaveBeenCalled();
  });
});

describe("context panel tab bodies", () => {
  it("renders the runs panel with an empty snapshot rather than a stale one", () => {
    mount(<ContextPanel {...baseProps({ activeTab: "runs" })} />);
    expect(recorded("runs")).toMatchObject({ runs: [], toolsByRun: {} });
  });

  it("keeps the aside and header available for every tab", () => {
    const view = mount(<ContextPanel {...baseProps({ activeTab: "artifacts" })} />);
    expect(view.container.querySelector("aside")).not.toBeNull();
    expect(view.container.querySelector<HTMLButtonElement>("button[aria-label=\"Collapse context panel\"]")).not.toBeNull();
    view.unmount();
  });
});

// The `PanelProps` alias documents the prop shape the assertions above mirror.
const _propShape: PanelProps | undefined = undefined;
void _propShape;
