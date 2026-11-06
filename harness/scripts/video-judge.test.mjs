import { test } from "node:test";
import assert from "node:assert/strict";
import { applyQuoteGate, judge } from "./video-judge.mjs";

// A corpus standing in for the app's own copy: only these words exist on screen.
const CORPUS = [
  "删除工作区", "暂无工作区对话", "暂无对话", "这些工作区对话将从侧边栏移除。",
  "设置", "取消", "正在启动 Future Agent",
].join("\n");

const issue = (o) => ({ at: "2s", quote: "", what: "", why: "", ...o });

test("a transport failure is retried once and the result records it", async () => {
  let calls = 0;
  const fetchImpl = async () => {
    calls += 1;
    if (calls === 1) throw new Error('HTTP/2: "stream timeout after 300000"');
    return { ok: true, json: async () => ({
      choices: [{ message: { content: '{"verdict":"pass","summary":"ok","issues":[]}' } }],
      usage: { prompt_tokens: 10, completion_tokens: 2 },
    }) };
  };
  const result = await judge({ video: process.execPath, expect: "x", fetchImpl });
  assert.equal(calls, 2, "one retry after the timeout");
  assert.equal(result.retried, true);
  assert.equal(result.attempts, 2);
  assert.equal(result.verdict, "pass");
});

test("a judgement that fails twice is an error, never a verdict", async () => {
  const fetchImpl = async () => { throw new Error("network down"); };
  await assert.rejects(
    () => judge({ video: process.execPath, expect: "x", fetchImpl }),
    /judge failed after 2 attempt/,
    "两个 300 秒超时不能变成一个 pass，也不能变成一个 fail",
  );
});

test("an issue with no verbatim quote cannot hold up a fail", () => {
  const gated = applyQuoteGate({ verdict: "fail", issues: [issue({ what: "按钮看起来不对" })] }, CORPUS);
  assert.equal(gated.verdict, "pass");
  assert.equal(gated.downgraded, true);
  assert.match(gated.rejected[0].why, /没有原样引用/);
});

test("a quote that merges two locale strings is rejected", () => {
  // The real failure: the model reported 「暂无会话 yet 工作区/对话」. The app has
  // 暂无工作区对话 and 暂无对话, and no "yet" anywhere.
  const gated = applyQuoteGate({ verdict: "fail",
    issues: [issue({ quote: "暂无会话 yet 工作区/对话", what: "侧栏空态矛盾" })] }, CORPUS);
  assert.equal(gated.verdict, "pass");
  assert.match(gated.rejected[0].why, /不存在/);
});

test("reading a red button as an enabled one is rejected", () => {
  const gated = applyQuoteGate({ verdict: "fail",
    issues: [issue({ quote: "删除", why: "红色高亮说明删除按钮可用" })] }, CORPUS);
  assert.equal(gated.verdict, "pass");
  assert.match(gated.rejected[0].why, /状态证据/);
});

test("the same claim survives when the quote carries real state evidence", () => {
  const gated = applyQuoteGate({ verdict: "fail",
    issues: [issue({ quote: "删除 disabled", why: "禁用态的按钮仍可点击" })] }, CORPUS);
  assert.equal(gated.verdict, "fail", "有状态证据的观察要留下");
  assert.equal(gated.downgraded, false);
});

test("a genuine quote from the app's copy still fails the case", () => {
  const gated = applyQuoteGate({ verdict: "fail",
    issues: [issue({ quote: "这些工作区对话将从侧边栏移除。", what: "弹窗文案与列表不符" })] }, CORPUS);
  assert.equal(gated.verdict, "fail");
  assert.equal(gated.issues.length, 1);
});
