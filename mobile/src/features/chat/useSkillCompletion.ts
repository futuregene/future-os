import { useEffect, useState, type Dispatch, type RefObject, type SetStateAction } from "react";
import { BackHandler, Keyboard, type TextInput } from "react-native";
import { completeSkill, insertSkillSlash, skillQuery, type TextSelection } from "./skillCompletion";

export function useSkillCompletion(message: string, setMessage: Dispatch<SetStateAction<string>>, enabled: boolean, inputRef: RefObject<TextInput | null>) {
  const [selection, setSelection] = useState<TextSelection>({ start: message.length, end: message.length });
  // Observe normal native caret movement without controlling it on every
  // keystroke (important for IME composition and asynchronously restored drafts).
  const [inputSelection, setInputSelection] = useState<TextSelection>();
  const [focused, setFocused] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const query = enabled && focused && !dismissed ? skillQuery(message, selection) : null;
  const open = query !== null;
  useEffect(() => {
    if (!open) return;
    const back = BackHandler.addEventListener("hardwareBackPress", () => {
      setDismissed(true);
      return true;
    });
    // Android's IME can consume Back before RN's BackHandler sees it.
    const keyboard = Keyboard.addListener("keyboardDidHide", () => setDismissed(true));
    return () => { back.remove(); keyboard.remove(); };
  }, [open]);

  return {
    selection, inputSelection, query,
    onSelectionChange: (next: TextSelection) => {
      setSelection(next);
      setInputSelection(undefined);
    },
    onChangeText: (text: string) => {
      if (text !== message) {
        // Native text and selection events need not arrive together. Derive
        // the edit's caret now so slash filtering never waits on a stale caret.
        // Prefer the known selection to disambiguate repeated characters.
        const prefix = message.slice(0, selection.start);
        const suffix = message.slice(selection.end);
        let cursor: number;
        if (selection.end <= message.length && text.length >= prefix.length + suffix.length
          && text.startsWith(prefix) && text.endsWith(suffix)) {
          cursor = text.length - suffix.length;
        } else {
          let start = 0;
          while (start < message.length && start < text.length && message[start] === text[start]) start++;
          let tail = 0;
          while (tail < message.length - start && tail < text.length - start
            && message[message.length - tail - 1] === text[text.length - tail - 1]) tail++;
          cursor = text.length - tail;
        }
        setSelection({ start: cursor, end: cursor });
      }
      setMessage(text);
      // Prediction is only for completion; do not control the native IME caret.
      setInputSelection(undefined);
      setDismissed(false);
    },
    onFocus: () => setFocused(true),
    onBlur: () => { setFocused(false); },
    close: () => setDismissed(true),
    insertSlash: () => {
      if (!enabled) return;
      const next = insertSkillSlash(message, selection);
      setMessage(next.text);
      setSelection(next.selection);
      setInputSelection(next.selection);
      setDismissed(false);
      setFocused(true);
      inputRef.current?.focus();
    },
    select: (name: string) => {
      if (!query || !enabled) return;
      const next = completeSkill(message, query, name);
      setMessage(next.text);
      setSelection(next.selection);
      setInputSelection(next.selection);
      setDismissed(true);
      inputRef.current?.focus();
    },
  };
}
