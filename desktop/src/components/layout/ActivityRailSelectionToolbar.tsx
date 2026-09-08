import { Trash2, X } from "lucide-react";
import { useEffect, useRef } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "../ui/Button";

export function ActivityRailSelectionToolbar({
  onCancel,
  onDelete,
  onToggleAll,
  selectedCount,
  totalCount,
}: {
  onCancel: () => void;
  onDelete: () => void;
  onToggleAll: () => void;
  selectedCount: number;
  totalCount: number;
}) {
  const { t } = useTranslation("layout");
  const checked = totalCount > 0 && selectedCount === totalCount;
  const checkboxRef = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (checkboxRef.current)
      checkboxRef.current.indeterminate = selectedCount > 0 && selectedCount < totalCount;
  }, [selectedCount, totalCount]);

  return (
    <div
      aria-label={t("activityRail.selectThreads")}
      className="activity-rail-selection-toolbar -mx-2 shrink-0 border-t border-line-soft bg-surface px-2 pt-2"
      data-activity-rail-selection-toolbar="true"
      role="toolbar"
    >
      <div className="flex items-center gap-2">
        <label className="flex min-w-0 flex-1 items-center gap-2 text-xs text-ink-soft">
          <input
            ref={checkboxRef}
            checked={checked}
            className="size-4 shrink-0 rounded border-line accent-accent"
            onChange={onToggleAll}
            type="checkbox"
          />
          <span className="truncate">
            {selectedCount > 0
              ? t("activityRail.threadsSelected", { count: selectedCount })
              : t("activityRail.selectAll")}
          </span>
        </label>
        <Button
          aria-label={t("activityRail.deleteSelected")}
          className="activity-rail-selection-action shrink-0"
          disabled={selectedCount === 0}
          leftIcon={<Trash2 className="size-3.5" />}
          onClick={onDelete}
          size="sm"
          title={t("activityRail.deleteSelected")}
          type="button"
          variant="danger"
        >
          <span className="activity-rail-selection-action-label whitespace-nowrap">
            {t("activityRail.deleteSelected")}
          </span>
        </Button>
        <Button
          aria-label={t("common:cancel")}
          className="activity-rail-selection-action shrink-0"
          leftIcon={<X className="size-3.5" />}
          onClick={onCancel}
          size="sm"
          title={t("common:cancel")}
          type="button"
          variant="secondary"
        >
          <span className="activity-rail-selection-action-label whitespace-nowrap">
            {t("common:cancel")}
          </span>
        </Button>
      </div>
    </div>
  );
}
