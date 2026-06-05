export type AgentMode = "plan" | "research" | "build" | "review";

export type MessageRole = "user" | "assistant" | "system";

export interface ToolCall {
  id: string;
  name: string;
  status: "queued" | "running" | "completed" | "failed";
  summary: string;
  input: string;
  output?: string;
}

export interface AgentPlanStep {
  id: string;
  title: string;
  detail: string;
  status: "completed" | "active" | "pending";
}

export type AgentActivityKind = "thinking" | "read" | "bash" | "edit" | "write";

export interface AgentActivityItem {
  id: string;
  kind: AgentActivityKind;
  status: "running" | "completed" | "failed";
  target?: string;
  detail?: string;
  count?: number;
  additions?: number;
  deletions?: number;
}

export interface MessageAttachment {
  artifactId?: string | null;
  name: string;
  path: string;
}

export interface AgentMessage {
  id: string;
  runId?: string | null;
  role: MessageRole;
  author: string;
  content: string;
  createdAt: string;
  activityItems?: AgentActivityItem[];
  attachments?: MessageAttachment[];
  plan?: AgentPlanStep[];
  toolCalls?: ToolCall[];
}
