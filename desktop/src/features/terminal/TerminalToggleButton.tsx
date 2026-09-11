/**
 * The header affordance that opens/closes the terminal panel.
 *
 * Lives in the conversation header, so it is reachable whether or not the right
 * context panel is expanded.
 */
import { SquareTerminal } from "lucide-react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";

export interface TerminalToggleButtonProps {
  open: boolean;
  shortcut: string;
  onToggle: () => void;
}

export function TerminalToggleButton({ open, shortcut, onToggle }: TerminalToggleButtonProps) {
  const { t } = useTranslation("terminal");
  const label = open ? t("collapse", { shortcut }) : t("expand", { shortcut });
  return (
    <IconButton
      active={open}
      aria-pressed={open}
      className="size-8"
      data-component="terminal-toggle"
      icon={<SquareTerminal className="size-4" />}
      label={label}
      onClick={onToggle}
    />
  );
}
