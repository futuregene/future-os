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
