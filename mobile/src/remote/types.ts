export type ConnectionPhase =
  "booting" | "unpaired" | "claiming" | "connecting" | "connected" | "reconnecting" | "error";

export interface PairingCode {
  v: 2;
  nonce: string;
  claim_url: string;
  exp: number;
}

export interface RemoteCredentials {
  pairId: string;
  deviceId: string;
  seed: string;
  userJwt: string;
  refreshToken: string;
  natsWsUrl: string;
  tokenUrl: string;
  expectedDesktopId: string;
  expectedDesktopPublicKey: string;
}

export interface RemoteSession {
  sessionId: string;
  threadId: string;
  title: string;
  mode?: "chat" | "workspace";
  workspaceId?: string;
}

export interface RemoteWorkspace {
  id: string;
  name: string;
  path: string;
  description?: string;
}

export interface PresenceSession {
  id: string;
  name: string;
  streaming: boolean;
}

export interface Presence {
  online: boolean;
  pairId: string;
  bridgeInstanceId: string;
  lastHeartbeatTs: number;
  sessions: PresenceSession[];
}

export interface RemoteModel {
  id: string;
  label?: string;
  provider?: string;
  isDefault?: boolean;
}

/** Stable agent model identifier; model ids are only unique within a provider. */
export function modelReference(model: Pick<RemoteModel, "id" | "provider">): string {
  if (!model.provider || model.id.startsWith(`${model.provider}/`)) return model.id;
  return `${model.provider}/${model.id}`;
}

export function modelProviderFromReference(modelReference: string): string | undefined {
  const separator = modelReference.indexOf("/");
  return separator > 0 ? modelReference.slice(0, separator) : undefined;
}

export interface RemoteSessionState {
  model?: string;
  thinkingLevel?: ThinkingLevel;
}

export type ThinkingLevel = "off" | "minimal" | "low" | "medium" | "high" | "xhigh";

export interface HistoryMessage {
  role: "user" | "assistant" | "tool" | string;
  content: string | { type?: string; text?: string }[];
  run_id?: string;
}

export interface StreamEvent {
  type: string;
  data: string;
  runId?: string;
  idx?: number;
}

export interface ApprovalPayload {
  approval_request_id: string;
  tool_name?: string;
  risk_level?: string;
  title?: string;
  summary?: string;
}

export type TimelineItem =
  | {
      id: string;
      kind: "message";
      role: "user" | "assistant";
      text: string;
      runId?: string;
      durationMs?: number;
    }
  | {
      id: string;
      kind: "thinking";
      text: string;
      complete: boolean;
      runId?: string;
    }
  | {
      id: string;
      kind: "run";
      startedAt: number;
      runId?: string;
    }
  | {
      id: string;
      kind: "tool";
      toolId: string;
      name: string;
      complete: boolean;
      runId?: string;
    }
  | {
      id: string;
      kind: "approval";
      payload: ApprovalPayload;
      decision?: "approved" | "rejected" | "cancelled";
      runId?: string;
    }
  | {
      id: string;
      kind: "notice";
      tone: "neutral" | "danger" | "warning";
      text: string;
      runId?: string;
    };

export interface RpcResponse<T = unknown> {
  success: boolean;
  data: T;
  error?: string;
}

export interface RemoteCommand {
  id?: string;
  type: string;
  sessionId?: string;
  message?: string;
  entryId?: string;
  mode?: string;
  runId?: string;
  sinceIdx?: number;
  offset?: number;
  limit?: number;
  modelId?: string;
  providerId?: string;
  level?: string;
  name?: string;
  workspaceId?: string;
  protocolVersion?: number;
  pairId?: string;
  deviceId?: string;
  clientPublicKey?: string;
  clientNonce?: string;
  desktopNonce?: string;
  expectedDesktopId?: string;
  expectedDesktopPublicKey?: string;
  clientSignature?: string;
}
