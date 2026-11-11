/**
 * The header affordance that opens/closes the terminal panel.
 *
 * Lives in the conversation header, so it is reachable whether or not the right
 * context panel is expanded.
 */
import { SquareTerminal } from "lucide-react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import { terminalPanelLabel } from "./panelLabel";

export interface TerminalToggleButtonProps {
  open: boolean;
  shortcut: string;
  onToggle: () => void;
  /**
   * Set when the machine on screen serves no terminals.
   *
   * The button then renders disabled, carrying the reason in its label. Shown
   * rather than hidden because "no control at all" and "this build cannot do
   * it" were indistinguishable: a paired desktop that does not advertise
   * `terminal_v1` simply had no button, which reads as a bug in the window.
   */
  unavailableReason?: string;
}

export function TerminalToggleButton({ open, shortcut, onToggle, unavailableReason }: TerminalToggleButtonProps) {
  const { t } = useTranslation("terminal");
  if (unavailableReason) {
    return (
      <IconButton
        aria-disabled
        className="size-8 opacity-50"
        data-component="terminal-toggle-unavailable"
        disabled
        icon={<SquareTerminal className="size-4" />}
        label={unavailableReason}
        onClick={() => {}}
      />
    );
  }
  const label = terminalPanelLabel(t, open, shortcut);
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
