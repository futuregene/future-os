import type { ReactNode } from "react";
import { ArrowLeft } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "../../components/ui/Button";

/** Shared compact header for ordinary tool details and specialized inspectors. */
export function RunInspectorToolbar({ status, actions, onBack }: {
  status?: ReactNode;
  actions?: ReactNode;
  onBack: () => void;
}) {
  const { t } = useTranslation("runs");
  return (
    <div className="flex h-8 shrink-0 items-center justify-between gap-2">
      {status ?? <span />}
      <div className="flex shrink-0 items-center gap-1">
        {actions}
        <Button className="h-6 shrink-0 self-start px-2" leftIcon={<ArrowLeft className="size-3.5" />} onClick={onBack} size="xs" variant="toolbar">
          {t("runInspect.back")}
        </Button>
      </div>
    </div>
  );
}
