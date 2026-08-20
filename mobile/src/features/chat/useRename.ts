import { useState } from "react";
import { Alert, Platform } from "react-native";
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
    if (Platform.OS === "ios") {
      Alert.prompt(
        t("chat.renameTitle"),
        undefined,
        [
          { text: t("chat.cancel"), style: "cancel" },
          {
            text: t("chat.save"),
            onPress: (value?: string) => {
              if (value?.trim()) void renameConversation(value);
            },
          },
        ],
        "plain-text",
        currentTitle,
      );
      return;
    }
    // Android has no native React Native text-input alert.
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
