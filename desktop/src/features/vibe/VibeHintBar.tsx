import type { CommandId } from "../keyboard/catalog";
import { useTranslation } from "react-i18next";
import { keyLabel } from "../../lib/keybinding";
import { isMacOS } from "../../lib/platform";
import { useActiveHints } from "./hints";

/**
 * A key the panel shows but does not own in the command table.
 *
 * Only sending is like this: `⏎` is the composer's own fixed key
 * (`MentionEditor.onSubmit`, see `FIXED_KEYS`), not a command, so listing it needs
 * the literal rather than a binding. Everything else on the line is read from the
 * table.
 */
export interface VibeFixedHint {
  /** The key, as `KeyboardEvent.code` (`Enter`, `Escape`). */
  code: string;
  /** A `shortcuts:fixed.*` key — the same string the settings page shows. */
  labelKey: string;
}

/**
 * The panel's hint line: the keys that do something in this layer, and what they
 * do.
 *
 * Generated rather than written by hand, because a hand-written line is a second
 * description of the keyboard and drifts from the first one. `activeHints` drops a
 * command nobody registered (so a layer that is not open cannot promise its keys)
 * and one the reader unbound.
 */
export function VibeHintBar({ fixed = [], ids }: { fixed?: readonly VibeFixedHint[]; ids: readonly CommandId[] }) {
  const { t } = useTranslation("shortcuts");
  const hints = useActiveHints(ids);
  const isMac = isMacOS;
  const items: Array<{ id: string; keys: string; label: string }> = [
    ...fixed.map(item => ({ id: item.labelKey, keys: keyLabel(item.code, isMac), label: t(item.labelKey) })),
    ...hints.map(hint => ({ id: hint.id, keys: hint.keys, label: t(`commands.${hint.id}`) })),
  ];
  if (items.length === 0)
    return null;
  return (
    <div className="flex shrink-0 flex-wrap items-center justify-center gap-x-3 gap-y-1 px-4 py-1.5 text-[11px] leading-4 text-ink-soft">
      {items.map(item => (
        <span className="flex items-center gap-1" key={item.id}>
          <kbd className="rounded border border-line-soft bg-surface-subtle px-1 font-mono text-[10px] text-ink">
            {item.keys}
          </kbd>
          <span>{item.label}</span>
        </span>
      ))}
    </div>
  );
}
