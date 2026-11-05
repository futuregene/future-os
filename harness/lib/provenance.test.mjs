/**
 * Proof that the fixture-provenance guard does its one job and no more.
 *
 * Two directions matter equally: a finding that quotes a value only a harness
 * build can produce must not become an issue, and a real defect must not be
 * demoted because the screenshot happened to be rendered from fixture data. The
 * second direction is the one a corpus-wide check failed — see the comment at the
 * top of provenance.mjs. Run with `node --test harness/lib/provenance.test.mjs`.
 */
import test from "node:test";
import assert from "node:assert/strict";
import { fixtureProvenance, harnessOnlyValue, markFixtureFindings } from "./provenance.mjs";

test("the desktop block names the fixture channels and what the demo mock answered", () => {
  const block = fixtureProvenance({
    surface: "desktop",
    server: { fallbacks: [{ command: "session_queue", handled: false }, { command: "list_runs", handled: false }, { command: "session_queue", handled: false }] },
  });
  assert.match(block.text, /0\.0\.0-harness/);
  assert.match(block.text, /真实 `app\.db` 的副本/);
  // Deduplicated, and only the commands this run actually fell through on.
  assert.match(block.text, /list_runs、session_queue/);
  assert.deepEqual(block.fallbacks, ["list_runs", "session_queue"]);
});

test("the mobile block says the whole dataset is a mock", () => {
  // #1548: five machines rendered 「已连接」 because `mobile/shot/mock/fleet.ts`
  // declares them so. The product's picker was correct; the data was not real.
  const block = fixtureProvenance({ surface: "mobile", server: { fallbacks: [] } });
  assert.match(block.text, /全部数据/);
  assert.match(block.text, /手机/);
});

test("a surface with no screen gets no block", () => {
  assert.equal(fixtureProvenance({ surface: "tui" }).text, "");
  assert.equal(fixtureProvenance({ surface: "cli" }).text, "");
});

test("a value no real build can emit is recognised", () => {
  // #1531: the update page printed the fixture's version and was reported as
  // "showing a placeholder instead of a release version".
  assert.match(harnessOnlyValue("检查更新页把当前版本写成 v0.0.0-harness") ?? "", /0\.0\.0-harness/);
  assert.equal(harnessOnlyValue("检查更新页写着「已是最新版本」，没有检查时间"), null);
});

test("a real defect that merely quotes fixture data is left alone", () => {
  // #1518 was real: the composer appended the skill token after the user's last
  // word. Its evidence quotes the draft the case itself typed, which the mock
  // corpus also happens to contain — so a corpus check demoted a true defect.
  const findings = [
    { source: "judge", severity: "high", title: "安装技能后的消息和发送前草稿对不上", detail: "输入框是「帮我把今天的文献整理成综述初稿并核对引用」，发出去变成「… /future-slides」", evidence: { quote: "第 12 步截图" } },
  ];
  assert.equal(markFixtureFindings(findings), 0);
  assert.equal(findings[0].severity, "high");
  assert.equal(findings[0].fixture, undefined);
});

test("a finding quoting the placeholder version is demoted, and stays visible in the report", () => {
  const findings = [
    { source: "judge", severity: "medium", title: "版本信息显示占位串 v0.0.0-harness 并立刻声称已是最新版本", detail: "", evidence: {} },
    { source: "judge", severity: "medium", title: "删除后侧边栏分组变稀疏", detail: "", evidence: {} },
  ];
  assert.equal(markFixtureFindings(findings), 1);
  assert.equal(findings[0].severity, "low");
  assert.match(findings[0].fixture, /0\.0\.0-harness/);
  assert.match(findings[0].demoted, /测试夹具/);
  assert.equal(findings[1].severity, "medium");
});

test("a failed assertion is never demoted by what the screen showed", () => {
  const findings = [{ source: "check", severity: "high", title: "版本输出的断言失败：v0.0.0-harness", detail: "", evidence: {} }];
  assert.equal(markFixtureFindings(findings), 0);
  assert.equal(findings[0].severity, "high");
});
