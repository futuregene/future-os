import { listen } from "@tauri-apps/api/event";
import { invokeCommand } from "../tauri/invoke";

export type AgentStatusPhase
  = | "checking"
    | "starting"
    | "recovering"
    | "ready"
    | "spawn_failed"
    | "exited"
    | "startup_timeout"
    | "unavailable"
    | "incompatible"
    | "stopping";

export interface ShutdownProgress {
  operationId: string;
  reason: string;
  state: "running" | "stopped" | "failed";
  stage: "admission" | "callers" | "remote" | "external_runs" | "terminals" | "agent" | "background" | "verification" | "complete";
  elapsedMs: number;
  errors: string[];
}

export function subscribeShutdownProgress(callback: (progress: ShutdownProgress) => void): Promise<() => void> {
  return listen<ShutdownProgress>("desktop-shutdown-progress", event => callback(event.payload)).catch(() => () => {});
}

export interface AgentStatus {
  phase: AgentStatusPhase;
  desktopVersion: string;
  agentVersion: string | null;
  error?: string;
  shutdown?: ShutdownProgress;
}

export function getAgentStatus(): Promise<AgentStatus> {
  return invokeCommand<AgentStatus>("get_agent_status");
}
