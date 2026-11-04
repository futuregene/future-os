/**
 * GitHub issue drafts from a run's findings.
 *
 * Human approval is the default: without `--create --yes` this only writes
 * `issues/*.md` next to the run so a person can read, edit and file them. The
 * harness is allowed to find problems by itself; it is not allowed to publish
 * them by itself.
 */
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join, resolve } from "node:path";
import { execFile } from "node:child_process";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

function slug(text) {
  return String(text).toLowerCase().replace(/[^\w\u4e00-\u9fa5]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 50) || "issue";
}

function issueBody(finding, run, analysis) {
  const caseInfo = analysis?.cases?.find(item => item.id === finding.evidence?.caseId);
  return [
    "## 现象",
    "",
    finding.detail || finding.title,
    "",
    "## 复现步骤",
    "",
    `- 用例：\`${finding.evidence?.caseId ?? "unknown"}\``,
    caseInfo ? `- 用例标题：${caseInfo.title}` : null,
    finding.evidence?.step ? `- 失败步骤：第 ${finding.evidence.step} 步` : null,
    "- 运行：`node harness/cli.mjs run --case " + (finding.evidence?.caseId ?? "<case>") + "`",
    "",
    "## 期望",
    "",
    caseInfo?.expectations?.length
      ? caseInfo.expectations.map(item => `- ${item}`).join("\n")
      : (caseInfo?.summary || "行为应与用例中写明的预期一致。"),
    "",
    "## 证据",
    "",
    `- 运行目录：\`harness/runs/${run.id}\``,
    finding.evidence?.dir ? `- 用例证据：\`${finding.evidence.dir}\`` : null,
    "- 截图 / 录像 / 控制台 / 网络 / 性能 / IPC 日志都在用例目录内",
    "",
    "## 环境",
    "",
    "- FutureOS 桌面端（Vite 渲染真实前端），真实 `app.db` 快照副本",
    "- 自动化来源：FutureOS UI harness",
    "",
    "_由 UI harness 发现并起草，经人工确认后提交。_",
  ].filter(Boolean).join("\n");
}

export async function draftIssues(runDir, { create = false, repo = null } = {}) {
  runDir = resolve(runDir);
  const analysisPath = join(runDir, "analysis.json");
  if (!existsSync(analysisPath))
    throw new Error(`no analysis.json in ${runDir}; run analyze first`);
  const analysis = JSON.parse(readFileSync(analysisPath, "utf8"));
  // Older analysis files predate the `expectations` field; the case record still
  // has them, and the issue's "期望" section is exactly where they belong.
  for (const item of analysis.cases ?? []) {
    if (item.expectations?.length)
      continue;
    const resultPath = join(runDir, item.id, "result.json");
    if (existsSync(resultPath))
      item.expectations = JSON.parse(readFileSync(resultPath, "utf8")).expectations ?? [];
  }
  const run = existsSync(join(runDir, "run.json"))
    ? JSON.parse(readFileSync(join(runDir, "run.json"), "utf8"))
    : { id: runDir.split("/").pop() };

  const findings = (analysis.findings ?? []).filter(finding =>
    ["high", "medium"].includes(finding.severity) && finding.source !== "harness");
  if (!findings.length)
    return { message: "没有 high/medium 级别的问题，未生成 issue 草稿。" };

  // One issue per case+category, not per finding: a failing flow usually
  // produces several sentences about the same defect, and six issues for two
  // bugs is how a tracker becomes noise.
  const groups = new Map();
  for (const finding of findings) {
    const key = `${finding.evidence?.caseId ?? "unknown"}|${finding.category ?? "behaviour"}`;
    const group = groups.get(key) ?? { findings: [], severity: finding.severity, caseId: finding.evidence?.caseId };
    group.findings.push(finding);
    if (finding.severity === "high")
      group.severity = "high";
    groups.set(key, group);
  }

  const dir = join(runDir, "issues");
  mkdirSync(dir, { recursive: true });
  const drafts = [];
  [...groups.values()].forEach((group, index) => {
    const primary = group.findings.find(item => item.severity === group.severity) ?? group.findings[0];
    const merged = {
      ...primary,
      detail: group.findings.map((item) => {
        const detail = String(item.detail ?? "").replace(/期望：；实际：。/g, "").trim();
        return `- **${item.title}**：${detail || "见失败断言与失败现场截图。"}`;
      }).join("\n\n"),
    };
    const file = join(dir, `${String(index + 1).padStart(2, "0")}-${slug(primary.title)}.md`);
    writeFileSync(file, issueBody(merged, run, analysis));
    drafts.push({ file, title: `[UI harness] ${primary.title}` });
  });

  if (!create)
    return { message: `已生成 ${drafts.length} 份 issue 草稿（未提交）：\n${drafts.map(item => `- ${item.file}`).join("\n")}` };

  if (!repo)
    throw new Error("--create 需要 --repo <owner/name>");
  const results = [];
  for (const draft of drafts) {
    try {
      const { stdout } = await execFileAsync("gh", [
        "issue", "create",
        "--repo", repo,
        "--title", draft.title,
        "--body-file", draft.file,
      ]);
      results.push(stdout.trim());
    }
    catch (error) {
      results.push(`失败：${draft.title} — ${String(error?.stderr ?? error?.message ?? error).slice(0, 200)}`);
    }
  }
  return { message: `提交结果：\n${results.join("\n")}` };
}
