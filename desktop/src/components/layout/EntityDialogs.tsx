import type { ReactNode } from "react";
import { useId } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";

interface RenameDialogProps {
  open: boolean;
  title: string;
  description: string;
  label: string;
  value: string;
  submitting: boolean;
  error: string | null;
  /** Receives the new value; the caller is responsible for clearing any error. */
  onChange: (value: string) => void;
  onConfirm: () => void;
  onClose: () => void;
}

/**
 * Rename dialog shared by the thread and workspace flows: a single labelled text
 * input that submits on Enter, with a cancel / save footer and inline error. The
 * caller owns the dialog state and wires `onChange` (clearing its own error).
 */
export function RenameDialog({
  open,
  title,
  description,
  label,
  value,
  submitting,
  error,
  onChange,
  onConfirm,
  onClose,
}: RenameDialogProps) {
  const { t } = useTranslation("layout");
  const inputId = useId();
  return (
    <Dialog
      description={description}
      footer={(
        <>
          <Button disabled={submitting} onClick={onClose} type="button" variant="ghost">
            {t("common:cancel")}
          </Button>
          <Button disabled={submitting} onClick={onConfirm} type="button" variant="primary">
            {submitting ? t("appShellDialogs.saving") : t("common:save")}
          </Button>
        </>
      )}
      onClose={onClose}
      open={open}
      title={title}
    >
      <label className="block text-sm font-medium text-ink-soft" htmlFor={inputId}>
        {label}
      </label>
      <input
        autoFocus
        className="mt-2 h-10 w-full rounded-md border border-line bg-surface px-3 text-sm text-ink outline-none transition focus:border-focus focus:ring-2 focus:ring-focus"
        disabled={submitting}
        id={inputId}
        onChange={event => onChange(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            onConfirm();
          }
        }}
        value={value}
      />
      {error ? <div className="mt-2 text-xs leading-5 text-danger">{error}</div> : null}
    </Dialog>
  );
}

interface ConfirmDeleteDialogProps {
  open: boolean;
  title: string;
  description?: ReactNode;
  submitting: boolean;
  error: string | null;
  onConfirm: () => void;
  onClose: () => void;
  /** The dialog body (the entity being deleted, plus any extra detail). */
  children: ReactNode;
}

/**
 * Delete-confirmation dialog shared by the thread and workspace flows: a danger
 * confirm / cancel footer and inline error around a caller-supplied body.
 */
export function ConfirmDeleteDialog({
  open,
  title,
  description,
  submitting,
  error,
  onConfirm,
  onClose,
  children,
}: ConfirmDeleteDialogProps) {
  const { t } = useTranslation("layout");
  return (
    <Dialog
      description={description}
      footer={(
        <>
          <Button disabled={submitting} onClick={onClose} type="button" variant="ghost">
            {t("common:cancel")}
          </Button>
          <Button disabled={submitting} onClick={onConfirm} type="button" variant="danger">
            {submitting ? t("appShellDialogs.deleting") : t("common:delete")}
          </Button>
        </>
      )}
      onClose={onClose}
      open={open}
      title={title}
    >
      {children}
      {error ? <div className="mt-2 text-xs leading-5 text-danger">{error}</div> : null}
    </Dialog>
  );
}
