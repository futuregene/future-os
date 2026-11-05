/**
 * Fixture provenance — which parts of the evidence the harness wrote itself.
 *
 * The harness renders the real UI over harness data: a copy of `app.db`, the
 * Tauri fixture server, and — wherever that server has no answer — the in-page
 * demo mock (`server.fallbacks` in a case's result). A model reading the frames
 * cannot tell those apart from the product, so it reports the harness's own data
 * as defects. Measured on the 2026-11-05 run (`final-v12`, 19 cases): of the
 * twelve findings a human later refuted, five were fixture-authored values —
 * the placeholder version `0.0.0-harness`, a demo approval card the fixture had
 * left pending, an assistant reply the demo backend cut to sixteen characters, a
 * device fleet the mock declares all-connected, and a forked history the fixture
 * had re-dated.
 *
 * Two defences, because a sentence in a prompt is a suggestion:
 *
 *  - `fixtureProvenance()` tells the model what is fixture data *in this run*
 *    (the commands the demo mock actually answered are read from the result, not
 *    guessed) and what that rules out.
 *  - `markFixtureFindings()` is deterministic, and deliberately narrow: only a
 *    value that cannot exist outside a harness build demotes a finding.
 *
 * A corpus check was tried first — flag any quoted string that occurs in the
 * fixture sources but not in the product's i18n — and thrown away: on that same
 * run it flagged two findings, one of them #1518 (a real defect whose evidence
 * quotes the draft the *case itself typed*, which the mock corpus also happens to
 * contain) and one of them a layout finding that merely named a demo path. Two
 * hits out of 116 findings, one wrong in each direction of usefulness. The
 * prompt does the work; the deterministic list only covers values a real build
 * cannot produce.
 *
 * Neither turns the harness into a code reviewer. A finding about *how the
 * product draws a value* — truncation, overlap, a bare empty state, a button that
 * does nothing, a count that disagrees with its own rows — is still a finding,
 * because that part is product code. Only the value itself is out of scope.
 */

/** Values only a harness build can produce. Extending this list is a promise
 * that no real build can emit the value; keep it to that. */
const HARNESS_ONLY = [
  { pattern: /0\.0\.0-harness/, what: "夹具的版本占位串 `0.0.0-harness`" },
  { pattern: /harness attachment/, what: "夹具写进附件的固定内容 `harness attachment`" },
];

const SCREENLESS = new Set(["tui", "cli", "remote-live", "remote-cn", "remote-mobile-live"]);

/**
 * Values the harness authors, with what a reader should conclude.
 *
 * This is the part a run cannot derive: knowing that `app_build_info` answers
 * `0.0.0-harness` needs someone to have read the fixture. Everything that *can*
 * be derived — which commands fell through to the demo mock — comes from the
 * result instead.
 */
const FIXTURE_CHANNELS = [
  ["app_build_info · get_agent_status · check_app_update", "版本号与构建号", "夹具固定回答 `0.0.0-harness` / `harness`；真实构建在编译期注入版本号，占位串不会出现在发行版里"],
  ["get_future_balance · 账单与用量", "余额、明细与金额", "夹具里的固定数字，不是账户的真实数据"],
  ["agent_prompt 及其流式事件", "助手回复正文", "演示后端写死的文案，不来自模型"],
  ["session_queue · 排队消息", "排队行与发送队列", "由页面内夹具模拟，不是真实会话队列"],
  ["pendingApproval（演示数据）", "审批卡的内容与状态", "夹具预置的一张卡，用来给审批界面拍照；它背后没有真实运行"],
  ["远程设备清单（fleet）", "已配对设备及其「已连接」状态", "夹具声明的设备集合，连接状态由夹具给定"],
  ["终端面板", "终端输出", "`desktop/shot/terminal-server.mjs` 回放的一段脚本输出"],
];

/**
 * The provenance block handed to the per-case judge and the open-ended pass.
 *
 * `result` is a case record (`result.json`) or anything with the same `surface`
 * and `server` fields. A surface with no screen to look at gets no block: there
 * is nothing a fixture could have drawn.
 */
export function fixtureProvenance(result = {}) {
  const surface = result.surface ?? "desktop";
  const fallbacks = [...new Set((result.server?.fallbacks ?? []).map(entry => entry.command).filter(Boolean))].sort();
  if (SCREENLESS.has(surface))
    return { surface, fallbacks, text: "" };

  const lines = ["数据来源（同一份证据里，哪些是产品行为，哪些是测试夹具写的）："];
  if (surface === "mobile" || surface === "remote") {
    lines.push(
      "本次运行是手机 / 远程界面，它的**全部数据**来自截图夹具（`mobile/shot/mock`）：会话、设备、用量、回复正文都是夹具写的，产品只负责把它们画出来。",
    );
  }
  else {
    lines.push(
      "本次运行是桌面端：会话列表、标题、时间戳、数量与工作区来自一份真实 `app.db` 的副本（量级与真实一致，但由夹具快照决定），界面由产品代码在真实 DOM 上渲染。",
    );
  }
  lines.push("", "夹具写死的部分（产品代码无法产生这些内容）：");
  const channels = surface === "mobile" || surface === "remote"
    ? FIXTURE_CHANNELS.filter(([channel]) => !channel.startsWith("终端面板"))
    : FIXTURE_CHANNELS;
  for (const [channel, what, why] of channels)
    lines.push(`- ${channel} → ${what}：${why}`);
  if (fallbacks.length) {
    lines.push(
      "",
      `这次运行里，以下 IPC 命令没有夹具实现，由页面内的演示 mock 作答（这些界面的内容同样是夹具写的）：${fallbacks.slice(0, 24).join("、")}`,
    );
  }
  lines.push(
    "",
    "据此的判断规则：",
    "- 某个值「是占位串」「不存在」「数量或状态与实际不符」，而该值来自上表 → 不是产品缺陷；最多写 info，并在 detail 里写明「来自测试夹具」。",
    "- 产品如何展示一个值——截断、遮挡、错位、文案、空态、禁用不说原因、按钮无反应、计数与条目不对应——仍然照常报：那是产品代码画的。",
    "- 分不清某条内容是谁写的，就把不确定写进 detail，severity 用 info；不要为了凑问题把它升级。",
  );
  return { surface, fallbacks, text: lines.join("\n") };
}

/** The harness-only value a piece of text quotes, if any. */
export function harnessOnlyValue(text) {
  const value = String(text ?? "");
  for (const entry of HARNESS_ONLY)
    if (entry.pattern.test(value))
      return entry.what;
  return null;
}

/**
 * Demote findings that quote a value no real build can emit.
 *
 * Runs over the merged finding list — per-case and cross-case alike — so the
 * report still records what was seen while the issue flow (which asks for medium
 * and above) cannot file it. Returns how many were marked.
 */
export function markFixtureFindings(findings) {
  let marked = 0;
  for (const finding of findings) {
    // A failed assertion speaks about the product's own behaviour, whatever the
    // screen happened to show around it.
    if (finding.fixture || finding.source === "check")
      continue;
    const value = harnessOnlyValue([finding.title, finding.detail, finding.evidence?.quote].filter(Boolean).join(" "));
    if (!value)
      continue;
    finding.fixture = `${value}（测试夹具写死，产品代码不会产生）`;
    finding.severity = "low";
    finding.demoted ??= "证据引用了测试夹具写死的值，已降级为观察项，不计入缺陷";
    marked += 1;
  }
  return marked;
}
