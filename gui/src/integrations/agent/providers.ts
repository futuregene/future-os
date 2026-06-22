import { invoke } from "@tauri-apps/api/core";

export interface BuiltinProvider {
  id: string;
  name: string;
  baseUrl: string;
  hasApiKey: boolean;
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
  return invoke<ProvidersView>("list_agent_providers");
}

export async function upsertCustomProvider(input: {
  id: string;
  name: string;
  api: string;
  baseUrl: string;
  apiKey?: string | null;
  models: CustomProviderModel[];
}) {
  return invoke<ProvidersView>("upsert_custom_provider", { input });
}

export async function deleteCustomProvider(id: string) {
  return invoke<ProvidersView>("delete_custom_provider", { id });
}
