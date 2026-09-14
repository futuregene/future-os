import { useState } from "react";
import { AppAlert as Alert } from "../../components/appAlerts";
import type { TFunction } from "i18next";
import { useRemote } from "../../remote/RemoteContext";

type Remote = ReturnType<typeof useRemote>;

export interface RenameApi {
  renameOpen: boolean;
  renameValue: string;
  setRenameValue: (value: string) => void;
  setRenameOpen: (value: boolean) => void;
  openRename: () => void;
  submitRename: () => Promise<void>;
}

export function useRename(remote: Remote, t: TFunction): RenameApi {
  const [renameOpen, setRenameOpen] = useState(false);
  const [renameValue, setRenameValue] = useState("");

  const renameConversation = async (rawName: string) => {
    const name = rawName.trim();
    if (!name) return;
    try {
      await remote.rename(remote.selectedSessionId, name);
    } catch {
      Alert.alert(t("common.error"));
    }
  };

  const openRename = () => {
    const currentTitle = remote.selectedTitle || "";
    setRenameValue(currentTitle);
    setRenameOpen(true);
  };

  const submitRename = async () => {
    const name = renameValue.trim();
    if (!name) return;
    setRenameOpen(false);
    await renameConversation(name);
  };

  return { renameOpen, renameValue, setRenameValue, setRenameOpen, openRename, submitRename };
}
