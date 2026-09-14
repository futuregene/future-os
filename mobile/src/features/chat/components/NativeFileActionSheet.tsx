import { ActionMenu } from "../../../components/ActionMenu";
import type { FileAction, FileOperation } from "../utils";

export function NativeFileActionSheet({
  action,
  openLabel,
  saveLabel,
  shareLabel,
  onClose,
  onSelect,
}: {
  action: FileAction | null;
  openLabel: string;
  saveLabel: string;
  shareLabel: string;
  onClose: () => void;
  onSelect: (action: FileAction, operation: FileOperation) => void;
}) {
  // ActionMenu waits for dismissal before handing presentation to the OS.
  // Keep all operations available even if no external reader is installed.
  return (
    <ActionMenu
      title={action?.info.name ?? ""}
      visible={action !== null}
      onClose={onClose}
      actions={action ? [
        { label: openLabel, onPress: () => onSelect(action, "open") },
        { label: saveLabel, onPress: () => onSelect(action, "save") },
        { label: shareLabel, onPress: () => onSelect(action, "share") },
      ] : []}
    />
  );
}
