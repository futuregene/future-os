import { createElement, useContext, useEffect, useState } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { PausedTimeline, TimelineVisibleContext } from "../PausedTimeline";

let tree: ReactTestRenderer;
afterEach(() => { act(() => tree?.unmount()); jest.useRealTimers(); });

test("500 covered updates retain children/state; uncovering installs the newest content", () => {
  let renders = 0;
  let unmounts = 0;
  function Content({ version }: { version: number }) {
    const [expanded, setExpanded] = useState(false);
    renders++;
    useEffect(() => () => { unmounts++; }, []);
    return createElement("button", { onClick: () => setExpanded(true), version, expanded });
  }
  const frame = (paused: boolean, version: number) =>
    createElement(PausedTimeline, { paused }, createElement(Content, { version }));
  act(() => { tree = create(frame(false, 0)); });
  act(() => tree.root.findByType("button").props.onClick());
  const before = renders;
  for (let i = 1; i <= 500; i++) act(() => tree.update(frame(true, i)));
  expect(renders).toBe(before);
  expect(unmounts).toBe(0);
  expect(tree.root.findByType("button").props).toMatchObject({ version: 0, expanded: true });
  act(() => tree.update(frame(false, 501)));
  expect(renders).toBe(before + 1);
  expect(tree.root.findByType("button").props).toMatchObject({ version: 501, expanded: true });
});

test("visibility crosses the memo boundary so display timers pause without unmounting", () => {
  jest.useFakeTimers();
  const tick = jest.fn();
  const dispose = jest.fn();
  function Timer() {
    const visible = useContext(TimelineVisibleContext);
    useEffect(() => {
      if (!visible) return;
      const timer = setInterval(tick, 1000);
      return () => clearInterval(timer);
    }, [visible]);
    useEffect(() => dispose, []);
    return null;
  }
  const frame = (paused: boolean) => createElement(PausedTimeline, { paused }, createElement(Timer));
  act(() => { tree = create(frame(false)); });
  act(() => jest.advanceTimersByTime(1000));
  expect(tick).toHaveBeenCalledTimes(1);
  act(() => tree.update(frame(true)));
  act(() => jest.advanceTimersByTime(10000));
  expect(tick).toHaveBeenCalledTimes(1);
  expect(dispose).not.toHaveBeenCalled();
  act(() => tree.update(frame(false)));
  act(() => jest.advanceTimersByTime(1000));
  expect(tick).toHaveBeenCalledTimes(2);
});
