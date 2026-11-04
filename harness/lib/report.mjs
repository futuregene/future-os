/**
 * Human-readable report. Built from `analysis.json` when it exists, and from
 * the raw case records otherwise, so a run can be reported even if analysis was
 * skipped or failed.
 */
import { existsSync, readFileSync, writeFileSync, readdirSync } from "node:fs";
import { join, relative, resolve } from "node:path";

function loadAnalysis(runDir) {
  const path = join(runDir, "analysis.json");
  return existsSync(path) ? JSON.parse(readFileSync(path, "utf8")) : null;
}

function loadCases(runDir) {
  return readdirSync(runDir)
    .filter(name => existsSync(join(runDir, name, "result.json")))
    .map(name => JSON.parse(readFileSync(join(runDir, name, "result.json"), "utf8")));
}

function statusIcon(status) {
  return { pass: "✅", fail: "❌", error: "💥" }[status] ?? "•";
}

function groupFindings(findings) {
  const rank = { critical: 0, high: 1, medium: 2, low: 3, info: 4 };
  const groups = new Map();
  for (const finding of findings) {
    const key = `${finding.category ?? "behaviour"}|${String(finding.title).replace(/\s+/g, " ").trim()}`;
    const group = groups.get(key) ?? { ...finding, cases: new Set(), details: new Set() };
    if ((rank[finding.severity] ?? 5) < (rank[group.severity] ?? 5))
      group.severity = finding.severity;
    if (finding.evidence?.caseId)
      group.cases.add(finding.evidence.caseId);
    if (finding.detail)
      group.details.add(finding.detail);
    groups.set(key, group);
  }
  return [...groups.values()].sort((a, b) => (rank[a.severity] ?? 5) - (rank[b.severity] ?? 5));
}

export function writeReport(runDir, { repoRoot = resolve(import.meta.dirname, "..", "..") } = {}) {
  runDir = resolve(runDir);
  const analysis = loadAnalysis(runDir);
  const cases = loadCases(runDir);
  const manifest = existsSync(join(runDir, "run.json"))
    ? JSON.parse(readFileSync(join(runDir, "run.json"), "utf8"))
    : { id: runDir.split("/").pop() };
  const totals = analysis?.totals ?? {
    cases: cases.length,
    passed: cases.filter(item => item.status === "pass").length,
    failed: cases.filter(item => item.status !== "pass").length,
    findings: 0,
  };

  const lines = [];
  lines.push(`# FutureOS UI 端到端测试报告`, "");
  lines.push(`- 运行：\`${manifest.id}\``);
  lines.push(`- 时间：${new Date().toISOString()}`);
  lines.push(`- 用例：${totals.cases} 个，通过 ${totals.passed}，失败 ${totals.failed}；发现问题 ${totals.findings ?? 0} 项`);
  lines.push(`- 数据：真实 \`app.db\` 快照（每个用例独立副本）`);
  lines.push(`- 界面：真实桌面前端（Vite + React），Tauri IPC 由夹具服务替代`);
  lines.push("");

  if (analysis?.cases?.length) {
    const summaries = analysis.cases.filter(item => item.summary);
    if (summaries.length) {
      lines.push("## 结论摘要", "");
      for (const item of summaries)
        lines.push(`- **${item.id}**：${item.summary}`);
      lines.push("");
    }
  }

  lines.push("## 用例结果", "");
  lines.push("| 结果 | 界面 | 用例 | 检查 | 耗时 | 录像 |");
  lines.push("|---|---|---|---|---|---|");
  for (const item of cases) {
    const checks = item.checks ?? [];
    const passed = checks.filter(check => check.ok).length;
    const dir = relative(runDir, item.artifacts?.dir ?? "");
    const surface = item.surface ?? "desktop";
    const video = existsSync(join(item.artifacts?.dir ?? "", "video.mp4"))
      ? `[mp4](${relative(runDir, join(item.artifacts.dir, "video.mp4"))})`
      : "—";
    lines.push(`| ${statusIcon(item.status)} ${item.status} | ${surface} | ${item.title}<br><code>${item.id}</code> | ${checks.length ? `${passed}/${checks.length}` : "—"} | ${(item.durationMs / 1000).toFixed(1)}s | ${video} |`);
  }
  lines.push("");

  if (analysis?.findings?.length) {
    const grouped = groupFindings(analysis.findings);
    lines.push("## 发现的问题", "");
    lines.push("| 级别 | 受影响用例 | 问题 |");
    lines.push("|---|---|---|");
    for (const finding of grouped) {
      const cases = [...finding.cases];
      lines.push(`| ${finding.severity} | ${cases.length ? cases.map(id => `\`${id}\``).join("<br>") : "—"} | ${finding.title.replace(/\|/g, "\\|")} |`);
    }
    lines.push("");

    grouped.forEach((finding, index) => {
      lines.push(`### F${index + 1} · [${finding.severity}] ${finding.title}`, "");
      for (const detail of finding.details)
        lines.push(detail, "");
      if (finding.cases.size) {
        lines.push(`- 用例：${[...finding.cases].map(id => `\`${id}\``).join("、")}`);
        const first = analysis.findings.find(item => item.title === finding.title && item.evidence?.dir);
        if (first?.evidence?.dir)
          lines.push(`- 证据目录：\`${relative(runDir, first.evidence.dir)}\``);
      }
      if (finding.source === "model")
        lines.push("- 来源：模型综合判断（结合截图与运行信号）");
      lines.push("");
    });
  } else {
    lines.push("## 发现的问题", "", analysis ? "没有发现需要报告的问题。" : "本次运行未执行分析；请运行 `node harness/cli.mjs analyze --run <dir>`。", "");
  }

  lines.push("## 证据与观测通道", "");
  lines.push("每个用例目录里包含：", "");
  lines.push("- `shots/*.png`：每一步操作后的截图，以及失败现场；`shots/shot-*.png` 为显式关键帧");
  lines.push("- `video.mp4`：真实时间的操作录像（CDP screencast，帧间隔按真实停顿编码）");
  lines.push("  - 播放器里的暂停/快进按钮是播放器叠加的，不在文件里；分析读的是帧图与 ffmpeg 抽取的拼图，不会把播放控件当成界面元素。");
  lines.push("- `timeline.jsonl`：动作、目标、耗时、指标、DB 查询结果");
  lines.push("- `server.jsonl`：UI 发出的每个 IPC 命令、参数摘要、耗时、是否由夹具处理");
  lines.push("- `console-errors.log` + `result.json`：控制台、异常、网络失败、性能序列");
  lines.push("- `app.db`：本用例的数据库副本（可直接 SQL 复核）");
  lines.push("");
  lines.push("这些通道覆盖了人眼之外的证据：网络失败、CPU/内存/主线程忙碌、IPC 调用与返回、数据库写入结果。", "");

  lines.push("## 如何复现", "");
  lines.push("```bash");
  lines.push("node harness/cli.mjs prepare");
  lines.push("node harness/cli.mjs run --case <case-id>");
  lines.push("node harness/cli.mjs analyze --run harness/runs/<run-id>");
  lines.push("```", "");
  lines.push("用例定义在 `harness/cases/**/*.json`，步骤是数据：可以在界面上照着点一遍复现。", "");

  lines.push("## 边界（这套 harness 没证明什么）", "");
  lines.push("- 后端是 JS 实现的夹具（读写真实 `app.db` 副本），不执行 `host/src/store/*.rs` 的真实查询，因此后端 SQL/Rust 逻辑缺陷不在范围内。");
  lines.push("- 对话正文为按标题合成的占位转录；真实转录在 1 GB 的 agent.db 投影层，尚未接入。");
  lines.push("- `agent_prompt` 等流式命令仍走截图 mock，不调用真实模型（不花钱、可重复），因此“模型回答质量”不在范围内。");
  lines.push("- 移动端（React Native）不在本次运行内；同一套 driver 可以驱动 `serve-mobile`。", "");

  const path = join(runDir, "report.md");
  writeFileSync(path, lines.join("\n"));
  return path;
}
