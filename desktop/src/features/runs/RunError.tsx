import type { StoredRun } from "../../integrations/storage/threadStore";
import { useTranslation } from "react-i18next";
import { cn } from "../../lib/cn";
import { errorTypeMeta } from "./runErrorMeta";

/**
 * Run error display, shared by the run list (compact `summary`) and the run
 * inspector (boxed `banner`). The error-type icon/label color is a deliberate
 * category color (see COLOR.md), not a semantic token.
 */
export function RunError({
  errorMessage,
  errorType,
  variant,
}: {
  errorMessage: string;
  errorType?: StoredRun["errorType"];
  variant: "summary" | "banner";
}) {
  const { t } = useTranslation("runs");
  const meta = errorTypeMeta(errorType);
  const banner = variant === "banner";
  return (
    <div className={banner ? "mt-3 rounded-md border border-danger-line bg-danger-soft p-2" : "mt-2"}>
      {meta
        ? (
            <div className={cn("flex items-center gap-1.5 text-xs font-medium", banner && "mb-1", meta.color)}>
              <meta.Icon className="size-3.5 shrink-0" />
              <span>{t(meta.labelKey)}</span>
            </div>
          )
        : null}
      <p className={cn("text-xs leading-5 text-danger", !banner && "line-clamp-2")}>{errorMessage}</p>
    </div>
  );
}
