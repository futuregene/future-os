import { invokeCommand } from "../tauri/invoke";

export type AgentStatusPhase
  = | "checking"
    | "starting"
    | "ready"
    | "spawn_failed"
    | "exited"
    | "startup_timeout"
    | "unavailable"
    | "incompatible";

export interface AgentStatus {
  phase: AgentStatusPhase;
  desktopVersion: string;
  agentVersion: string | null;
}

export function getAgentStatus(): Promise<AgentStatus> {
  return invokeCommand<AgentStatus>("get_agent_status");
}
