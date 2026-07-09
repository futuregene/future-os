import { invokeCommand } from "../tauri/invoke";

/** The FutureOS builtin provider id — its key backs the signed-in account. */
export const FUTURE_PROVIDER_ID = "future";

export interface BuiltinProvider {
  id: string;
  name: string;
  baseUrl: string;
  hasApiKey: boolean;
  modelCount: number;
  /**
   * The catalog base URL is a placeholder (e.g. Azure's YOUR_RESOURCE); the user
   * must supply their own, so the Providers page shows a Base URL field.
   */
  requiresBaseUrl: boolean;
}

export interface CustomProviderModel {
  id: string;
  name: string;
  /** Whether the model accepts image input. Text input is always implied. */
  supportsImages: boolean;
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

export interface FutureEnvironment {
  /** `production` | `test` | `custom`. */
  environment: string;
  /** Resolved platform root currently in effect (no `/api`). */
  platformUrl: string;
}

/** The platform environment currently in effect (owned by the Tauri backend). */
export function getFutureEnvironment() {
  return invokeCommand<FutureEnvironment>("get_future_environment");
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
  const view = await invokeCommand<ProvidersView>("update_builtin_provider_key", { input });
  // Setting the FutureOS key by hand (Providers page) changes the account too.
  if (input.id === FUTURE_PROVIDER_ID)
    clearFutureProfileCache();
  return view;
}

export async function setBuiltinProviderBaseUrl(input: {
  id: string;
  /** Empty string clears the override, reverting to the catalog placeholder. */
  baseUrl: string;
}) {
  return invokeCommand<ProvidersView>("set_builtin_provider_base_url", { input });
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
  const result = await invokeCommand<FutureLoginPoll>("poll_future_login", { deviceCode });
  // A completed device login writes a new key — invalidate here so it doesn't
  // matter which page (Account or Providers) ran the login dialog.
  if (result.status === "authorized")
    clearFutureProfileCache();
  return result;
}

export async function logoutFutureProvider() {
  const view = await invokeCommand<ProvidersView>("logout_future_provider");
  clearFutureProfileCache();
  return view;
}

/** The signed-in FutureOS account, from `{platform}/client/v1/account/profile`. */
export interface FutureProfile {
  email: string;
  userId: string;
  emailVerified: boolean;
  createdAt: string | null;
}

// The profile rarely changes (only on logout/login), so cache it in-memory for
// the app session: reopening the account page reuses this instead of refetching
// (which flashed the label). Cleared on logout; `force` refetches after login.
let profileCache: FutureProfile | null = null;

/**
 * Fetch the signed-in account profile, served from the session cache after the
 * first call. Pass `force` to bypass the cache (e.g. right after a login).
 * Rejects when signed out or on error.
 */
export async function getFutureProfile(force = false): Promise<FutureProfile> {
  if (force || !profileCache)
    profileCache = await invokeCommand<FutureProfile>("get_future_profile");
  return profileCache;
}

/** The cached profile if one was fetched this session, else null (sync read). */
export function peekFutureProfile(): FutureProfile | null {
  return profileCache;
}

/** Drop the cached profile so the next login refetches (call on logout). */
export function clearFutureProfileCache(): void {
  profileCache = null;
}
