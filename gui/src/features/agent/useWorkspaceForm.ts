import type { FormEvent } from "react";
import type { StoredWorkspace } from "../../integrations/storage/threadStore";
import type { ConversationMode } from "./NewConversation";
import { open } from "@tauri-apps/plugin-dialog";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { errorMessage } from "../../lib/errors";
import { pathBasename } from "../../lib/workspacePath";

export type WorkspaceFormMode = "open" | null;

export interface WorkspaceCreateRequest {
  name?: string | null;
  path: string;
  createDirectory: boolean;
}

interface UseWorkspaceFormInput {
  initialWorkspaceForm?: WorkspaceFormMode;
  workspaces: StoredWorkspace[];
  onAddWorkspace: (input: WorkspaceCreateRequest) => Promise<StoredWorkspace | null>;
  onSelectWorkspace: (id: string) => void;
  onModeChange: (mode: ConversationMode) => void;
  onCloseMenu: () => void;
}

export interface WorkspaceForm {
  mode: WorkspaceFormMode;
  displayName: string;
  path: string;
  error: string | null;
  notice: string | null;
  creating: boolean;
  setDisplayName: (value: string) => void;
  begin: () => void;
  cancel: () => void;
  pickFolder: () => Promise<void>;
  submit: (event: FormEvent) => Promise<void>;
}

/**
 * The create/open-workspace form state machine, lifted out of NewConversation so
 * the composer view stays focused on message composition. Owns the form fields
 * and drives the parent's workspace selection/mode through callbacks.
 */
export function useWorkspaceForm({
  initialWorkspaceForm,
  workspaces,
  onAddWorkspace,
  onSelectWorkspace,
  onModeChange,
  onCloseMenu,
}: UseWorkspaceFormInput): WorkspaceForm {
  const { t } = useTranslation("agent");
  const [workspaceFormMode, setWorkspaceFormMode] = useState<WorkspaceFormMode>(initialWorkspaceForm ?? null);
  const [workspaceDisplayName, setWorkspaceDisplayName] = useState("");
  const [workspacePath, setWorkspacePath] = useState("");
  const [workspaceError, setWorkspaceError] = useState<string | null>(null);
  const [workspaceNotice, setWorkspaceNotice] = useState<string | null>(null);
  const [creatingWorkspace, setCreatingWorkspace] = useState(false);

  async function submit(event: FormEvent) {
    event.preventDefault();
    const path = workspacePath.trim();
    const displayName = workspaceDisplayName.trim();
    if (!path) {
      setWorkspaceError(t("newConversation.errorChooseDir"));
      return;
    }

    setCreatingWorkspace(true);
    setWorkspaceError(null);
    try {
      const workspace = await onAddWorkspace({
        name: displayName || null,
        path,
        createDirectory: false,
      });
      if (workspace) {
        onSelectWorkspace(workspace.id);
        onModeChange("workspace");
      }
      setWorkspaceDisplayName("");
      setWorkspacePath("");
      setWorkspaceFormMode(null);
      onCloseMenu();
    }
    catch (error) {
      setWorkspaceError(errorMessage(error));
    }
    finally {
      setCreatingWorkspace(false);
    }
  }

  function begin() {
    setWorkspaceFormMode("open");
    onCloseMenu();
    setWorkspaceError(null);
    setWorkspaceNotice(null);
    setWorkspaceDisplayName("");
    setWorkspacePath("");
  }

  // Cancelling the create-workspace dialog: fall back to the most-recently-used
  // workspace when one exists (list is ordered last-opened first), otherwise
  // there's nothing to land on, so switch to plain chat.
  function cancel() {
    setWorkspaceFormMode(null);
    setWorkspaceError(null);
    setWorkspaceNotice(null);
    setWorkspaceDisplayName("");
    setWorkspacePath("");
    const mostRecent = workspaces[0];
    if (mostRecent) {
      onSelectWorkspace(mostRecent.id);
      onModeChange("workspace");
    }
    else {
      onModeChange("chat");
    }
  }

  async function pickFolder() {
    const selected = await open({
      directory: true,
      multiple: false,
      title: t("newConversation.openWorkspaceDialog"),
    });

    if (typeof selected === "string") {
      setWorkspacePath(selected);
      // Opening a folder that already has a workspace record just reopens the
      // existing one (the backend dedupes by path) — flag it so the user knows.
      const existing = workspaces.find(workspace => samePath(workspace.path, selected));
      setWorkspaceNotice(existing ? t("newConversation.workspaceExists", { name: existing.name }) : null);
      if (!workspaceDisplayName.trim()) {
        setWorkspaceDisplayName(lastPathSegment(selected));
      }
    }
  }

  return {
    mode: workspaceFormMode,
    displayName: workspaceDisplayName,
    path: workspacePath,
    error: workspaceError,
    notice: workspaceNotice,
    creating: creatingWorkspace,
    setDisplayName: setWorkspaceDisplayName,
    begin,
    cancel,
    pickFolder,
    submit,
  };
}

function lastPathSegment(path: string) {
  return pathBasename(path) || "Workspace";
}

/**
 * Compare filesystem paths ignoring a trailing separator and case (macOS /
 * Windows are case-insensitive; the backend also dedupes by exact string).
 */
function samePath(a: string, b: string) {
  const normalize = (value: string) => value.replace(/[/\\]+$/, "").toLowerCase();
  return normalize(a) === normalize(b);
}
