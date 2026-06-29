import { homedir } from "node:os";
import { join } from "node:path";

export const DEFAULT_API_URL = "https://api.future-os.cn";
export const DEFAULT_PLATFORM_URL = "https://api.future-os.cn";
export const AUTH_FILE = join(homedir(), ".future", "agent", "auth.json");
export const FUTURE_AUTH_PROVIDER = "future";

export const DEFAULT_LAUNCHD_LABEL = "com.future.agent";
export const DEFAULT_SYSTEMD_UNIT = "future-agent.service";
export const DEFAULT_WINDOWS_SERVICE = "FutureAgent";
export const DEFAULT_AGENT_GRPC_ADDR = "127.0.0.1:50051";

export const DEFAULT_CHANNEL_LAUNCHD_LABEL = "com.future.channel";
export const DEFAULT_CHANNEL_SYSTEMD_UNIT = "future-channel.service";
export const DEFAULT_CHANNEL_WINDOWS_SERVICE = "FutureChannel";
