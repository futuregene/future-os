import type { ProbeKey } from "./keyProbe";
import { X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { IconButton } from "../../components/ui/IconButton";
import {
  describeKey,
  isGesturePress,
  keyProbeAvailable,
  nextTap,
} from "./keyProbe";

/** The last presses the readout keeps: enough to see a key repeat, not a transcript. */
const KEPT = 6;

/**
 * The panel's key self-check: a small readout of what the machine actually sends.
 *
 * Shown by three quick `⌥⇧K`s (a chord nothing binds), and only in a developer build or
 * after an explicit opt-in — see `keyProbe.ts` for why it is not a command and never
 * consumes a press.
 *
 * `enabled` is a prop so a test can drive it without a build flag; the panel passes
 * nothing and the flag decides.
 */
export function VibeKeyProbe({ enabled }: { enabled?: boolean }) {
  const { t } = useTranslation("vibe");
  const available = enabled ?? keyProbeAvailable(Boolean(import.meta.env.DEV));
  const [open, setOpen] = useState(false);
  const [recent, setRecent] = useState<ProbeKey[]>([]);
  const [copied, setCopied] = useState(false);
  const tapRef = useRef({ at: 0, taps: 0 });

  useEffect(() => {
    if (!available)
      return;
    function onKeyDown(event: KeyboardEvent) {
      const press: ProbeKey = {
        alt: event.altKey,
        code: event.code,
        ctrl: event.ctrlKey,
        key: event.key,
        meta: event.metaKey,
        repeat: event.repeat,
        shift: event.shiftKey,
      };
      // The gesture: counted here, and *also* recorded like any other press, so the
      // person toggling it can see the chord's own values.
      const step = nextTap(tapRef.current, { at: event.timeStamp, gesture: isGesturePress(press) });
      tapRef.current = { at: step.at, taps: step.taps };
      if (step.fired) {
        setOpen(current => !current);
        setRecent([]);
        return;
      }
      setRecent(current => [press, ...current].slice(0, KEPT));
    }
    // Bubble phase, no `capture`: this is an observer, and a press it shows must still
    // reach the app. Nothing here calls `preventDefault` or `stopPropagation`.
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [available]);

  if (!available || !open)
    return null;

  const lines = recent.map(describeKey);
  async function copy() {
    try {
      await navigator.clipboard.writeText(lines.join("\n"));
      setCopied(true);
      // Not a timer anyone relies on: the label simply returns to "copy" on its own.
      setTimeout(setCopied, 1500, false);
    }
    catch {
      // A webview that refuses the clipboard leaves the text selectable on screen, which
      // is the fallback that always works.
    }
  }

  return (
    <aside
      aria-label={t("probe.title")}
      className="pointer-events-auto fixed bottom-3 right-3 z-60 w-[min(34rem,calc(100vw-1.5rem))] overflow-hidden rounded-lg border border-line-soft bg-surface/95 shadow-panel backdrop-blur"
      data-vibe-key-probe=""
    >
      <header className="flex h-8 items-center gap-2 border-b border-line-soft/60 px-2">
        <span className="text-[11px] font-medium text-ink-soft">{t("probe.title")}</span>
        <span className="min-w-0 flex-1 truncate text-[10px] text-ink-faint">{t("probe.hint")}</span>
        <button
          className="rounded border border-line-soft px-1.5 py-0.5 text-[10px] text-ink-soft hover:bg-surface-subtle"
          onClick={() => void copy()}
          type="button"
        >
          {copied ? t("probe.copied") : t("probe.copy")}
        </button>
        <IconButton
          className="size-6"
          icon={<X className="size-3" />}
          label={t("probe.close")}
          onClick={() => setOpen(false)}
        />
      </header>
      <div className="max-h-48 overflow-y-auto px-2 py-1.5 font-mono text-[11px] leading-5 text-ink">
        {lines.length === 0
          ? <p className="text-ink-muted">{t("probe.empty")}</p>
          : lines.map((line, index) => (
              // Newest first, and the newest is the one being read: it gets the ink.
              // A press has no id of its own, and the same key pressed twice must stay
              // two rows: the list is a short ring buffer of press history, where the
              // position *is* the identity (newest first).
              // eslint-disable-next-line react/no-array-index-key
              <div className={index === 0 ? "text-ink" : "text-ink-soft"} key={`${line}-${index}`}>
                {line}
              </div>
            ))}
      </div>
    </aside>
  );
}
