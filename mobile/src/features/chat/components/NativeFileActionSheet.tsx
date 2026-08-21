import { useEffect, useRef } from "react";
import { Alert, Platform } from "react-native";
import { deferPresentation, type FileAction } from "../utils";

export function NativeFileActionSheet({
  action,
  cancelLabel,
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
    if (!action || shownActionRef.current === action) return;
    shownActionRef.current = action;
    const close = () => {
      shownActionRef.current = null;
      onClose();
    };
    const select = (save: boolean) => {
      close();
      deferPresentation(() => onSelect(action, save));
    };
    if (Platform.OS === "ios") {
      // iOS uses the same system share sheet for both "open" and "save".
      // Present it directly instead of asking the user to choose between two
      // actions that lead to the same native surface.
      select(false);
      return;
    }
    Alert.alert(
      action.info.name,
      undefined,
      [
        { text: openLabel, onPress: () => select(false) },
        { text: cancelLabel, style: "cancel", onPress: close },
        { text: saveLabel, onPress: () => select(true) },
      ],
      { cancelable: true, onDismiss: close },
    );
  }, [action, cancelLabel, onClose, onSelect, openLabel, saveLabel]);

  return null;
}
