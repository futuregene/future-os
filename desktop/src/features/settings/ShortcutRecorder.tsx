/**
 * The "press the new combination" control.
 *
 * It is a button until the user clicks it, then it listens: the whole point of a
 * recorder is that the keys the user presses are the answer, so while it is armed
 * it takes the keyboard away from everything else (the global dispatcher asks
 * `keyboardOwners` first) and reads the raw event. A refusal — a bare key, an OS
 * combination, one another command already has — keeps it armed: the user pressed
 * a key they did not mean, and the next press is the one they did.
 *
 * The rules themselves live in `shortcutRecorderRules` (pure, DOM-free).
 */
import type { CommandId } from "../keyboard/catalog";
import type { RecorderRefusal } from "./shortcutRecorderRules";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { formatBinding, isModifierKey } from "../../lib/keybinding";
import { useKeyboardOwner } from "../../lib/keyboardOwners";
import { isMacOS } from "../../lib/platform";
import { resolvedBinding, setCommandBinding, useShortcutOverrides } from "../keyboard/shortcuts";
import { heldModifiers, interpretPress, refusalText } from "./shortcutRecorderRules";

export function ShortcutRecorder({
  armed,
  command,
  onArmedChange,
}: {
  /**
   * Whether this row is the one listening. Kept by the page, because two armed
   * recorders would both read the same press and write two bindings.
   */
  armed: boolean;
  command: CommandId;
  onArmedChange: (armed: boolean) => void;
}) {
  const { t } = useTranslation("shortcuts");
  // Subscribed: after a successful recording the button must show the new
  // binding, and the store is the only thing that knows it changed.
  useShortcutOverrides();
  const [refusal, setRefusal] = useState<RecorderRefusal | null>(null);
  // A held modifier is shown while waiting, so the control answers the press
  // instead of sitting there unchanged.
  const [held, setHeld] = useState("");
  const isMac = isMacOS;

  // While armed the recorder owns the keyboard: the global dispatcher must not
  // run the command the user is trying to replace.
  useKeyboardOwner(armed);

  useEffect(() => {
    if (!armed)
      return;
    const onKeyDown = (event: KeyboardEvent) => {
      event.preventDefault();
      event.stopPropagation();
      // Escape leaves the binding alone; Backspace removes it. Both are keys the
      // recorder could otherwise store, so they are its own reserved pair.
      if (event.key === "Escape") {
        onArmedChange(false);
        setRefusal(null);
        return;
      }
      if (event.key === "Backspace" || event.key === "Delete") {
        setCommandBinding(command, null);
        onArmedChange(false);
        setRefusal(null);
        return;
      }
      const result = interpretPress(event, command, isMac);
      if (result === null) {
        setHeld(heldModifiers(event, isMac));
        return;
      }
      if ("problem" in result) {
        setRefusal(result);
        return;
      }
      setCommandBinding(command, result.binding);
      onArmedChange(false);
      setRefusal(null);
    };
    const onKeyUp = (event: KeyboardEvent) => {
      if (isModifierKey(event))
        setHeld(heldModifiers(event, isMac));
    };
    window.addEventListener("keydown", onKeyDown, true);
    window.addEventListener("keyup", onKeyUp, true);
    return () => {
      window.removeEventListener("keydown", onKeyDown, true);
      window.removeEventListener("keyup", onKeyUp, true);
    };
  }, [armed, command, isMac, onArmedChange]);

  const current = resolvedBinding(command);
  const label = armed
    ? (held || t("recorder.prompt"))
    : (current ? formatBinding(current, isMac) : t("page.unbound"));

  return (
    <div className="flex flex-col items-end gap-1">
      <Button
        aria-label={armed ? t("recorder.prompt") : label}
        aria-pressed={armed}
        className={armed ? "min-w-28 border-accent text-accent" : "min-w-28"}
        data-recording={armed ? "true" : "false"}
        onClick={() => {
          if (armed) {
            setRefusal(null);
            onArmedChange(false);
            return;
          }
          setRefusal(null);
          setHeld("");
          onArmedChange(true);
        }}
        size="xs"
        variant="secondary"
      >
        {label}
      </Button>
      {armed
        ? <span className="text-2xs text-ink-muted">{t("recorder.hint")}</span>
        : null}
      {refusal
        ? (
            <span className="text-2xs text-danger" role="alert">
              {refusalText(t, refusal, isMac)}
            </span>
          )
        : null}
    </div>
  );
}
