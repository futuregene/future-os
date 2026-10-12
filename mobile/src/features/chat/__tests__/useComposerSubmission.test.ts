import { createElement, useLayoutEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useComposerSubmission } from "../composer/useComposerSubmission";
import { showToast } from "../utils";

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
    restoring: false,
    setMessage: jest.fn(),
    send: jest.fn(async () => { }),
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
  let pending!: Promise<void>;
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
  let first!: Promise<void>;
  let second!: Promise<void>;
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
  let pending!: Promise<void>;
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

test("unrelated remote updates preserve composer callbacks", () => {
  const options = optionsFor();
  render(options);
  const before = api;
  render({ ...options, remote: { ...options.remote, skillsRevision: 2 } });
  expect(api).toBe(before);
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
  let pending!: Promise<void>;
  act(() => { pending = api.installSuggestedSkill(); });
  expect(api.installingSkill).toBe(true);
  current = false;
  await act(async () => { installation.resolve("old draft /skill"); await pending; });
  expect(api.installingSkill).toBe(false);
  expect(options.send).not.toHaveBeenCalled();
  expect(options.setMessage).not.toHaveBeenCalled();
});
