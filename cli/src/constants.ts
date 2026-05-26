import { homedir } from "node:os";
import { join } from "node:path";

export const DEFAULT_API_URL = "http://127.0.0.1:7003";
export const AUTH_FILE = join(homedir(), ".future", "agent", "auth.json");
export const MODELS_FILE = join(homedir(), ".future", "agent", "models.json");
export const FUTURE_AUTH_PROVIDER = "future";

export const DEFAULT_LAUNCHD_LABEL = "com.future.agent";
export const DEFAULT_SYSTEMD_UNIT = "future-agent.service";
export const DEFAULT_WINDOWS_SERVICE = "FutureAgent";
export const DEFAULT_AGENT_GRPC_ADDR = "127.0.0.1:50051";
