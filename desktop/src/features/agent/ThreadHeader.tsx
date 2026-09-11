import type { ReactNode } from "react";
import type { StoredThread } from "../../integrations/storage/threadStore";
import { useTranslation } from "react-i18next";
import { LeftPanelTitlebarToggle } from "../../components/layout/LeftPanelTitlebarToggle";
import { startWindowDrag } from "../../lib/windowDrag";

interface ThreadHeaderProps {
  thread: StoredThread | null;
  leftPanelExpanded: boolean;
  onToggleLeftPanel: () => void;
  /**
   * Optional trailing affordance owned by the shell (the terminal toggle).
   * A node rather than a callback keeps this header unaware of the feature.
   */
  action?: ReactNode;
}

export function ThreadHeader({
  thread,
  leftPanelExpanded,
  onToggleLeftPanel,
  action,
}: ThreadHeaderProps) {
  const { t } = useTranslation("agent");
  return (
    <header
      className="flex h-12 shrink-0 select-none items-center border-b border-line-soft/40 pl-4 pr-14"
      onMouseDown={startWindowDrag}
    >
      <div className="mr-3 flex min-w-0 flex-1 items-center" data-tauri-drag-region>
        <LeftPanelTitlebarToggle
          expanded={leftPanelExpanded}
          onToggle={onToggleLeftPanel}
        />
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm font-semibold text-ink">{thread?.title ?? t("thread.defaultTitle")}</div>
        </div>
      </div>
      {action
        ? (
            <div className="flex shrink-0 items-center gap-1" data-tauri-drag-region="false">
              {action}
            </div>
          )
        : null}
    </header>
  );
}
