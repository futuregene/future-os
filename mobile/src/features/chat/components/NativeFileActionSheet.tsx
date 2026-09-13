import { useEffect, useRef } from "react";
import { Platform } from "react-native";
import { ActionMenu } from "../../../components/ActionMenu";
import { deferPresentation, type FileAction } from "../utils";

export function NativeFileActionSheet({
  action,
  openLabel,
  saveLabel,
  onClose,
  onSelect,
}: {
  action: FileAction | null;
  cancelLabel: string;
  openLabel: string;
  saveLabel: string;
  onClose: () => void;
  onSelect: (action: FileAction, save: boolean) => void;
}) {
  const shownActionRef = useRef<FileAction | null>(null);
  useEffect(() => {
    if (!action) { shownActionRef.current = null; return; }
    if (Platform.OS !== "ios" || shownActionRef.current === action) return;
    shownActionRef.current = action;
    // iOS uses one real system share sheet for both open and save.
    onClose();
    deferPresentation(() => onSelect(action, false));
  }, [action, onClose, onSelect]);
  if (Platform.OS === "ios") return null;
  return (
    <ActionMenu
      title={action?.info.name ?? ""}
      visible={action !== null}
      onClose={onClose}
      actions={action ? [
        { label: openLabel, onPress: () => onSelect(action, false) },
        { label: saveLabel, onPress: () => onSelect(action, true) },
      ] : []}
    />
  );
}
