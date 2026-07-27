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

export interface RemoteSessionState {
  model?: string;
  thinkingLevel?: ThinkingLevel;
}

export type ThinkingLevel = "off" | "low" | "medium" | "high";

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
  level?: string;
  name?: string;
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
