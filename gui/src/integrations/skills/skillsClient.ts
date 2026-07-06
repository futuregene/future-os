import { invokeCommand } from "../tauri/invoke";

/** A skill the agent currently loads (source of the "已安装" tab). */
export interface InstalledSkill {
  id: string;
  name: string;
  description: string;
  version: string | null;
}

/** A skill from the platform catalogue (source of the "全部" tab). */
export interface AvailableSkill {
  id: string;
  name: string;
  description: string;
  category: string;
  latestVersion: string | null;
}

/** Installed skills, as seen by the agent (`get_commands`). */
export function listInstalledSkills(): Promise<InstalledSkill[]> {
  return invokeCommand<InstalledSkill[]>("list_installed_skills");
}

/** The platform skill catalogue. Requires the platform to be reachable. */
export function listAvailableSkills(): Promise<AvailableSkill[]> {
  return invokeCommand<AvailableSkill[]>("list_available_skills");
}

/** Download + unpack a skill version into the app scope. */
export function installSkill(id: string, version: string): Promise<void> {
  return invokeCommand<void>("install_skill", { id, version });
}

/** Remove a skill from every scope it's installed in. */
export function uninstallSkill(id: string): Promise<boolean> {
  return invokeCommand<boolean>("uninstall_skill", { id });
}
