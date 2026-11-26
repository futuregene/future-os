import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";
import { Dialog } from "../../components/ui/Dialog";

/**
 * Leaving a settings page that holds unsaved edits.
 *
 * A page that keeps a draft — rather than writing every control as it is
 * touched — can be left in a state the user believes is already applied: another
 * tab, the backdrop, Escape, "back". Each of those used to drop the draft
 * silently. The surface that owns the navigation hosts this guard, and a page
 * with a draft registers what it holds so leaving can ask first.
 */
export interface UnsavedEdits {
  /** What the page edits, named in the prompt ("Fusion model configuration"). */
  subject: string;
  /** Read at confirm time, not at registration: the draft changes under it. */
  isDirty: () => boolean;
  /** Save the draft. Resolves `false` when it could not be saved. */
  save: () => Promise<boolean>;
}

export interface UnsavedGuardApi {
  /** Run `action` now, or first ask about the edits a registered page holds. */
  confirm: (action: () => void) => void;
  register: (id: symbol, read: () => UnsavedEdits) => () => void;
}

/**
 * Hosts the guard for one settings surface. Returns the API its navigation calls
 * through — hand it to the pages that keep a draft — plus the prompt to render
 * (or `null` while nothing is asking).
 */
export function useUnsavedGuardHost() {
  const { t } = useTranslation("settings");
  const readersRef = useRef(new Map<symbol, () => UnsavedEdits>());
  const [pending, setPending] = useState<{ action: () => void; edits: UnsavedEdits[] } | null>(null);
  const [saving, setSaving] = useState(false);

  const api = useMemo<UnsavedGuardApi>(() => ({
    confirm: (action) => {
      // Read every guard now: a page that saved behind our back is not dirty any
      // more, and asking then would train the user to dismiss the prompt.
      const dirty = [...readersRef.current.values()]
        .map(read => read())
        .filter(edits => edits.isDirty());
      if (dirty.length === 0) {
        action();
        return;
      }
      setSaving(false);
      setPending({ action, edits: dirty });
    },
    register: (id, read) => {
      readersRef.current.set(id, read);
      return () => {
        readersRef.current.delete(id);
      };
    },
  }), []);

  const dismiss = () => setPending(null);

  async function saveAndLeave() {
    if (!pending)
      return;
    const { action, edits } = pending;
    setSaving(true);
    const results = await Promise.all(edits.map(entry => entry.save()));
    setSaving(false);
    // A refused save (an invalid draft, or the host went away) keeps the user
    // here with the edits intact; the page itself says what went wrong.
    const saved = results.every(Boolean);
    setPending(null);
    if (saved)
      action();
  }

  const dialog = pending
    ? (
        <Dialog
          bodyClassName="text-sm leading-5 text-ink-soft"
          footer={(
            <>
              <Button onClick={dismiss} variant="ghost">{t("unsavedGuard.cancel")}</Button>
              <Button
                onClick={() => {
                  const action = pending.action;
                  setPending(null);
                  action();
                }}
                variant="secondary"
              >
                {t("unsavedGuard.discard")}
              </Button>
              <Button disabled={saving} onClick={() => void saveAndLeave()} variant="primary">
                {saving ? t("unsavedGuard.saving") : t("unsavedGuard.save")}
              </Button>
            </>
          )}
          onClose={dismiss}
          open
          title={t("unsavedGuard.title")}
        >
          {t("unsavedGuard.body", { subject: pending.edits.map(edits => edits.subject).join(" / ") })}
        </Dialog>
      )
    : null;

  return { api, dialog };
}

/** Registers one page's draft with the surface that hosts the guard. */
export function useUnsavedGuard(guard: UnsavedGuardApi | null | undefined, edits: UnsavedEdits) {
  const latestRef = useRef<UnsavedEdits>(edits);
  // The registration is stable and the guard is read through the ref, so a page
  // does not re-register on every render.
  useEffect(() => {
    latestRef.current = edits;
  }, [edits]);
  const idRef = useRef(Symbol("unsaved-guard"));
  useEffect(() => guard ? guard.register(idRef.current, () => latestRef.current) : undefined, [guard]);
}
