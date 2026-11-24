import { createElement, useLayoutEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useComposerSubmission } from "../composer/useComposerSubmission";
import { showToast } from "../utils";
import { choiceAnswerFromCard, pickChoiceOption } from "../choiceAnswer";

const mockRemoteControls = {
  capabilities: new Set<string>(), desktopReadable: false, desktopSettingsRevision: 0,
  getFusionSettings: jest.fn(),
};
jest.mock("../../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemoteControls }));
jest.mock("../utils", () => ({ showToast: jest.fn() }));

type Options = Parameters<typeof useComposerSubmission>[0];
let tree: ReactTestRenderer;
let api: ReturnType<typeof useComposerSubmission>;
function Harness({ options }: { options: Options; }) {
  const value = useComposerSubmission(options);
  useLayoutEffect(() => { api = value; }, [value]);
  return null;
}
function render(options: Options) {
  act(() => {
    if (tree) tree.update(createElement(Harness, { options }));
    else tree = create(createElement(Harness, { options }));
  });
}
function optionsFor(): Options {
  return {
    remote: { selectedSessionId: "A", timeline: { streaming: false } } as Options["remote"],
    t: ((key: string) => key) as Options["t"],
    message: "please search the web for this thing",
    attachments: [],
    choice: null,
    clearChoice: jest.fn(),
    restoring: false,
    setMessage: jest.fn(),
    send: jest.fn(async () => true),
    scrollToLatest: jest.fn(),
    recommendation: {
      suggestion: null, evaluating: false, evaluate: jest.fn(async () => false),
      dismiss: jest.fn(), installAndUse: jest.fn(async () => "draft /skill")
    },
  };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(done => { resolve = done; });
  return { promise, resolve };
}
beforeEach(() => jest.clearAllMocks());
afterEach(() => {
  if (tree) act(() => tree.unmount());
  tree = undefined as never;
});

test.each(["evaluation", "installation"])("navigation during %s never sends to the next conversation", async phase => {
  const options = optionsFor();
  const evaluation = deferred<boolean>();
  const installation = deferred<string>();
  jest.mocked(options.recommendation.evaluate).mockReturnValue(evaluation.promise);
  jest.mocked(options.recommendation.installAndUse).mockReturnValue(installation.promise);
  render(options);
  let pending!: Promise<boolean>;
  act(() => { pending = phase === "evaluation" ? api.sendFromComposer() : api.installSuggestedSkill(); });
  render({ ...options, remote: { ...options.remote, selectedSessionId: "B" }, message: "B draft" });
  render({ ...options, message: "back to A" });
  expect(api.installingSkill).toBe(false);
  await act(async () => { evaluation.resolve(false); installation.resolve("old draft /skill"); await pending; });
  expect(options.send).not.toHaveBeenCalled();
  expect(options.setMessage).not.toHaveBeenCalled();
  expect(options.scrollToLatest).not.toHaveBeenCalled();
  expect(showToast).not.toHaveBeenCalled();
});

test("double tapping submit starts one evaluation and one delivery", async () => {
  const options = optionsFor();
  const evaluation = deferred<boolean>();
  jest.mocked(options.recommendation.evaluate).mockReturnValue(evaluation.promise);
  render(options);
  let first!: Promise<boolean>;
  let second!: Promise<boolean>;
  act(() => { first = api.sendFromComposer(); second = api.sendFromComposer(); });
  expect(options.recommendation.evaluate).toHaveBeenCalledTimes(1);
  await act(async () => { evaluation.resolve(false); await Promise.all([first, second]); });
  expect(options.send).toHaveBeenCalledTimes(1);
});

test("a changed draft is held after its earlier evaluation", async () => {
  const options = optionsFor();
  const evaluation = deferred<boolean>();
  jest.mocked(options.recommendation.evaluate).mockReturnValue(evaluation.promise);
  render(options);
  let pending!: Promise<boolean>;
  act(() => { pending = api.sendFromComposer(); });
  render({ ...options, message: "new edit" });
  await act(async () => { evaluation.resolve(false); await pending; });
  expect(options.send).not.toHaveBeenCalled();
});

test("submit cannot send an empty draft while restoration is pending", async () => {
  const options = optionsFor();
  render({ ...options, restoring: true });
  await act(async () => { await api.sendFromComposer(); });
  expect(options.recommendation.evaluate).not.toHaveBeenCalled();
  expect(options.send).not.toHaveBeenCalled();
});

test("a stale \"still streaming\" flag does not swallow the send", async () => {
  // The flag is this client's projection: it outlives the run it described (a
  // missed terminal event, a socket that died mid-stream) and nothing clears it
  // now that automatic resume is retired. Consulting it made sends vanish with
  // no feedback at all; the desktop answers for itself instead.
  const options = optionsFor();
  render({
    ...options,
    remote: {
      ...options.remote,
      timeline: { ...options.remote.timeline, streaming: true },
    },
  });
  await act(async () => { await api.sendFromComposer(); });
  expect(options.send).toHaveBeenCalledWith(undefined);
});

test("unrelated remote updates preserve composer callbacks", () => {
  const options = optionsFor();
  render(options);
  const before = api;
  render({ ...options, remote: { ...options.remote, skillsRevision: 2 } });
  expect(api).toBe(before);
});

test("a send that names the queueing policy hands it to the session, and answers whether it landed", async () => {
  // The send sheet's interrupt relies on both halves: the policy is what keeps
  // the message the session's while the run it waited behind ends, and the
  // answer is what decides whether ending the run is still the right thing.
  const options = optionsFor();
  jest.mocked(options.send).mockResolvedValue(true);
  render(options);
  let delivered!: boolean | undefined;
  await act(async () => { delivered = await api.sendFromComposer({ busyPolicy: "enqueue_coalescing" }); });
  expect(options.send).toHaveBeenCalledWith(
    undefined, { busyPolicy: "enqueue_coalescing" },
  );
  expect(delivered).toBe(true);
});

test("a refused send answers false, so a caller does not act on it", async () => {
  const options = optionsFor();
  jest.mocked(options.send).mockResolvedValue(false);
  render(options);
  let delivered!: boolean | undefined;
  await act(async () => { delivered = await api.sendFromComposer({ busyPolicy: "enqueue_coalescing" }); });
  expect(delivered).toBe(false);
});

test("a send tapped while the context compacts waits for it and then delivers", async () => {
  // Manual compaction owns the session for as long as the summary takes, and the
  // Agent refuses prompts meanwhile. The tap is taken and held instead of being
  // dropped: the draft stays in the field and the send goes when the fence opens.
  const options = optionsFor();
  options.remote = { ...options.remote, compacting: true } as Options["remote"];
  render(options);
  let pending!: Promise<boolean>;
  act(() => { pending = api.sendFromComposer(); });
  expect(api.holdingSend).toBe(true);
  expect(options.send).not.toHaveBeenCalled();
  // The hold is one send: a second tap does not queue another one.
  await act(async () => { await api.sendFromComposer(); });
  render({ ...options, remote: { ...options.remote, compacting: false } });
  await act(async () => { await pending; });
  expect(api.holdingSend).toBe(false);
  expect(options.send).toHaveBeenCalledTimes(1);
});

test("a draft edited while the compaction held the send is not delivered", async () => {
  const options = optionsFor();
  options.remote = { ...options.remote, compacting: true } as Options["remote"];
  render(options);
  let pending!: Promise<boolean>;
  act(() => { pending = api.sendFromComposer(); });
  render({ ...options, message: "new edit", remote: { ...options.remote, compacting: false } });
  await act(async () => { await pending; });
  expect(options.send).not.toHaveBeenCalled();
  // The words are still there for the reader to send themselves.
  expect(options.setMessage).not.toHaveBeenCalled();
});

test("accepting a recommendation passes the current edited draft to installation", async () => {
  const options = optionsFor();
  options.message = "the edited question";
  options.recommendation.suggestion = {
    draft: "original question", skill: { name: "future-web", description: "search" },
  };
  jest.mocked(options.recommendation.installAndUse).mockResolvedValue("the edited question /future-web");
  render(options);
  await act(async () => { await api.installSuggestedSkill(); });
  expect(options.recommendation.installAndUse).toHaveBeenCalledWith("the edited question");
  expect(options.setMessage).toHaveBeenCalledWith("the edited question /future-web");
  expect(options.send).toHaveBeenCalledWith("the edited question /future-web");
});

test("connection cancellation releases the install spinner and keeps the draft", async () => {
  const options = optionsFor();
  let current = true;
  options.remote = { ...options.remote, captureConversationOperation: () => () => current };
  const installation = deferred<string>();
  jest.mocked(options.recommendation.installAndUse).mockReturnValue(installation.promise);
  render(options);
  let pending!: Promise<boolean>;
  act(() => { pending = api.installSuggestedSkill(); });
  expect(api.installingSkill).toBe(true);
  current = false;
  await act(async () => { installation.resolve("old draft /skill"); await pending; });
  expect(api.installingSkill).toBe(false);
  expect(options.send).not.toHaveBeenCalled();
  expect(options.setMessage).not.toHaveBeenCalled();
});

/**
 * The answer a `futureos-choice` card produced is sent as text the model can read
 * (§7), and it counts as something to send on its own — the phone's field cannot
 * hold it, so neither the gate here nor the composer's `canSend` may ignore it.
 */
const CARD_ANSWER = pickChoiceOption(choiceAnswerFromCard({
  mode: "single",
  options: [{ id: "forest", label: "重画森林图" }, { id: "flow", label: "补一张实验流程图" }],
  title: "接下来做哪一件？",
  type: "choiceCard",
}), "forest");

test("an answer with nothing typed is a send, and the send takes the answer with it", async () => {
  const options = { ...optionsFor(), message: "", attachments: [], choice: CARD_ANSWER, clearChoice: jest.fn() };
  render(options);
  let delivered!: Promise<boolean>;
  act(() => { delivered = api.sendFromComposer(); });
  await act(async () => { expect(await delivered).toBe(true); });
  expect(options.send).toHaveBeenCalledWith("[接下来做哪一件？ → 重画森林图]");
  expect(options.clearChoice).toHaveBeenCalledTimes(1);
  // The draft's text is untouched: the reader typed nothing.
  expect(options.setMessage).not.toHaveBeenCalled();
});

test("an answer plus a note sends both, answer first", async () => {
  const options = {
    ...optionsFor(),
    message: "  顺便把图例挪到右下  ",
    choice: CARD_ANSWER,
    clearChoice: jest.fn(),
  };
  render(options);
  await act(async () => { await api.sendFromComposer(); });
  expect(options.send).toHaveBeenCalledWith("[接下来做哪一件？ → 重画森林图] 顺便把图例挪到右下");
  expect(options.clearChoice).toHaveBeenCalledTimes(1);
});

test("a refused send keeps the answer, so it can be sent again", async () => {
  const options = {
    ...optionsFor(),
    message: "",
    choice: CARD_ANSWER,
    clearChoice: jest.fn(),
    send: jest.fn(async () => false),
  };
  render(options);
  let delivered!: Promise<boolean>;
  act(() => { delivered = api.sendFromComposer(); });
  await act(async () => { expect(await delivered).toBe(false); });
  expect(options.send).toHaveBeenCalledTimes(1);
  expect(options.clearChoice).not.toHaveBeenCalled();
});

test("an empty draft with no answer sends nothing", async () => {
  const options = { ...optionsFor(), message: "", attachments: [], choice: null };
  render(options);
  let delivered!: Promise<boolean>;
  act(() => { delivered = api.sendFromComposer(); });
  await act(async () => { expect(await delivered).toBe(false); });
  expect(options.send).not.toHaveBeenCalled();
});

test("an answer with nothing ticked is no answer: no tag, and nothing to send alone", async () => {
  // Unticking a multi-choice answer's last row leaves a question with no pick.
  // §7's sentence is `[question → answer]`, so it would send `[接下来做哪一件？ → ]`
  // — a question the model reads as an answer. The draft already drops it
  // (`answerOrNull`); this is the same rule at the last gate every send passes.
  const unpicked = { ...CARD_ANSWER, mode: "multi" as const, selectedIds: [] as string[] };

  const alone = { ...optionsFor(), message: "", attachments: [], choice: unpicked };
  render(alone);
  let delivered!: Promise<boolean>;
  act(() => { delivered = api.sendFromComposer(); });
  await act(async () => { expect(await delivered).toBe(false); });
  // Nothing to send on its own, and above all no sentence carrying `→ ]`.
  expect(alone.send).not.toHaveBeenCalled();

  // With a note beside it the note goes out alone — the reader's own words are
  // theirs, and dropping the answer must not take them.
  const withNote = { ...optionsFor(), message: "顺便把图例挪到右下", choice: unpicked };
  render(withNote);
  await act(async () => { await api.sendFromComposer(); });
  expect(withNote.send).toHaveBeenLastCalledWith("顺便把图例挪到右下");
  for (const call of (withNote.send as jest.Mock).mock.calls)
    expect(String(call[0])).not.toContain("→");
});
