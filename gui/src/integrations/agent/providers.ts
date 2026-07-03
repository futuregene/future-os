import { invokeCommand } from "../tauri/invoke";

export interface BuiltinProvider {
  id: string;
  name: string;
  baseUrl: string;
  hasApiKey: boolean;
  modelCount: number;
}

export interface CustomProviderModel {
  id: string;
  name: string;
}

export interface CustomProvider {
  id: string;
  name: string;
  api: string;
  baseUrl: string;
  hasApiKey: boolean;
  models: CustomProviderModel[];
}

export interface ProvidersView {
  builtin: BuiltinProvider[];
  custom: CustomProvider[];
}

export async function listAgentProviders() {
  return invokeCommand<ProvidersView>("list_agent_providers");
}

export async function upsertCustomProvider(input: {
  id: string;
  name: string;
  api: string;
  baseUrl: string;
  apiKey?: string | null;
  models: CustomProviderModel[];
  /** True when adding a new provider; the backend then rejects a colliding id. */
  create: boolean;
}) {
  return invokeCommand<ProvidersView>("upsert_custom_provider", { input });
}

export async function updateBuiltinProviderKey(input: {
  id: string;
  apiKey?: string | null;
}) {
  return invokeCommand<ProvidersView>("update_builtin_provider_key", { input });
}

export async function deleteCustomProvider(id: string) {
  return invokeCommand<ProvidersView>("delete_custom_provider", { id });
}

export interface FutureLoginStart {
  userCode: string;
  verificationUri: string;
  verificationUriComplete: string;
  /** Server-suggested poll interval, in seconds. */
  interval: number;
  /** Device-code lifetime, in seconds. */
  expiresIn: number;
  deviceCode: string;
}

export type FutureLoginStatus
  = | "pending"
    | "slow_down"
    | "authorized"
    | "denied"
    | "expired"
    | "error";

export interface FutureLoginPoll {
  status: FutureLoginStatus;
  message?: string | null;
}

export async function startFutureLogin() {
  return invokeCommand<FutureLoginStart>("start_future_login");
}

export async function pollFutureLogin(deviceCode: string) {
  return invokeCommand<FutureLoginPoll>("poll_future_login", { deviceCode });
}

export async function logoutFutureProvider() {
  return invokeCommand<ProvidersView>("logout_future_provider");
}
