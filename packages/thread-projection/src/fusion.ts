/** Public, credential-free Fusion read projections shared by desktop/mobile. */
export interface FusionConclusion {
  fusion_run_id: string;
  conclusion: string;
  rationale: string[];
  conditions: string[];
  unresolved: string[];
  next_steps: string[];
  completeness: string;
}
export interface FusionStep {
  id: string;
  phase: string;
  model: string;
  participant: string;
  attempt: number;
  input: string;
  status: string;
  output: string;
  error?: string | null;
  session_id: string;
  agent_run_id: string;
  started_at: number;
  finished_at?: number | null;
  usage: unknown;
  transcript?: string;
  next_offset?: number;
  total_bytes?: number;
}
export interface FusionDiscussion {
  pending?: boolean;
  run?: {
    id: string;
    status: string;
    phase: string;
    completeness: string;
    input: { question: string; goal: string; background: string; constraints: string[] };
    result?: FusionConclusion | null;
    error?: string | null;
  };
  steps?: FusionStep[];
  events?: { seq: number; kind: string; data: unknown; timestamp: number }[];
}
export interface FusionReadRequest { runId: string; toolCallId: string; stepId?: string; offset?: number; fusion?: boolean }
export function fusionTask(text: string): string | null {
  const match = /^\s*\/fusion(?=\s|$)\s*/u.exec(text);
  return match ? text.slice(match[0].length) : null;
}
export function fusionTranscript(raw: string): string {
  try {
    const messages = JSON.parse(raw) as { role: string; content: unknown }[];
    return messages.map(message => `${message.role}\n${JSON.stringify(message.content, null, 2)}`).join("\n\n");
  } catch { return raw; }
}

export interface FusionIssue { id: string; topic: string; stances: { participant: string; position: string }[] }
export function fusionIssues(discussion: FusionDiscussion | null): FusionIssue[] {
  const payload = discussion?.events?.find(event => event.kind === "issues")?.data;
  if (!payload || typeof payload !== "object" || !("issues" in payload) || !Array.isArray(payload.issues)) return [];
  return payload.issues.filter((issue): issue is FusionIssue => !!issue && typeof issue === "object" && typeof issue.id === "string" && typeof issue.topic === "string" && Array.isArray(issue.stances));
}

/** Stage progress, not a prediction of elapsed time or a completion percentage. */
export const FUSION_PHASES = ["analyzing", "issue_mapping", "reviewing", "synthesizing"] as const;
export type FusionModelStatus = "waiting" | "thinking" | "completed" | "failed" | "timed_out" | "output_limited" | "cancelled" | "interrupted" | "partial";
const DEFAULT_FUSION_MODELS = ["future/deepseek-flash", "future/glm-5.3-flash", "future/qwen3.7-plus"];
export function fusionModelLabel(model: string): string {
  const labels: Record<string, string> = { "future/deepseek-flash": "DeepSeek Flash", "future/glm-5.3-flash": "GLM 5.3 Flash", "future/qwen3.7-plus": "Qwen 3.7 Plus" };
  return labels[model] ?? model.replace(/^future\//u, "");
}
function modelStatus(step: FusionStep): FusionModelStatus {
  if (step.status === "starting" || step.status === "running") return "thinking";
  if (step.status === "failed") {
    if (/timed?\s*out|timeout/iu.test(step.error ?? "")) return "timed_out";
    if (/OUTPUT_LIMIT|finish_length/iu.test(step.error ?? "")) return "output_limited";
    return "failed";
  }
  if (["completed", "cancelled", "interrupted"].includes(step.status)) return step.status as FusionModelStatus;
  return "waiting";
}
export function fusionProgress(discussion: FusionDiscussion | null, toolStatus?: string) {
  const run = discussion?.run;
  const status = toolStatus === "failed" && (!run || ["preparing", "queued", "running", "delivering"].includes(run.status)) ? "interrupted" : run?.status ?? "preparing";
  const terminal = ["completed", "failed", "cancelled", "interrupted"].includes(status);
  const phase = run?.phase ?? "preparing";
  const stageIndex = FUSION_PHASES.findIndex(value => value === phase);
  const completedStages = status === "completed" ? FUSION_PHASES.length : Math.max(0, stageIndex);
  // A retry replaces the previous attempt for presentation only. Original
  // attempts stay in storage and continue to be available for debugging.
  const latest = new Map<string, FusionStep>();
  for (const step of discussion?.steps ?? []) {
    const key = `${step.phase}:${step.participant}`;
    const previous = latest.get(key);
    if (!previous || step.attempt > previous.attempt || (step.attempt === previous.attempt && step.started_at >= previous.started_at)) latest.set(key, step);
  }
  const created = discussion?.events?.find(event => event.kind === "created")?.data;
  const configured = created && typeof created === "object" && "models" in created && Array.isArray(created.models) ? created.models.filter((value): value is string => typeof value === "string") : DEFAULT_FUSION_MODELS;
  const models = [...new Set([...configured, ...[...latest.values()].map(step => step.model)])].map(model => {
    const steps = [...latest.values()].filter(step => step.model === model).sort((a, b) => a.started_at - b.started_at);
    const currentSteps = steps.filter(step => step.phase === phase);
    const current = currentSteps[currentSteps.length - 1];
    let state: FusionModelStatus = "waiting";
    const last = steps[steps.length - 1];
    if (last) state = modelStatus(last);
    if (!terminal && (phase === "analyzing" || phase === "reviewing") && !current) state = "waiting";
    if (current) state = modelStatus(current);
    if (terminal && (state === "thinking" || state === "waiting")) state = status === "cancelled" ? "cancelled" : "interrupted";
    if (terminal && state === "completed" && steps.some(step => ["failed", "timed_out", "output_limited", "cancelled", "interrupted"].includes(modelStatus(step)))) state = "partial";
    return { model, label: fusionModelLabel(model), status: state };
  });
  return { phase, status, terminal, stageIndex, completedStages, partial: !!run && run.completeness !== "full", models };
}
