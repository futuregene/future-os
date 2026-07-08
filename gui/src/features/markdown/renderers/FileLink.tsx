import type { StoredFile } from "../../../integrations/storage/types";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { openPath } from "../../../integrations/storage/files";
import { copyText } from "../../../lib/clipboard";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { useDismissableLayer } from "../../../lib/useDismissableLayer";

/**
 * A local-file link (from a plain markdown path link) rendered inline. Left
 * click opens the file with the OS default handler (directories land in the
 * file manager); right click opens a context menu with copy / open actions.
 * In-workspace files show their workspace-relative path, files written elsewhere
 * (e.g. `~/Desktop`) show the full path.
 */
export function FileLink({ file }: { file: StoredFile }) {
  const { t } = useTranslation("markdown");
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);
  const layerRef = useDismissableLayer<HTMLDivElement>({
    enabled: menu !== null,
    onDismiss: () => setMenu(null),
  });

  const display = file.insideWorkspace && file.relativePath ? file.relativePath : file.path;

  async function open() {
    setMenu(null);
    try {
      await openPath(file.path);
    }
    catch {
      emitFutureEvent("toast", { message: t("fileLink.notFound", { name: file.name }), tone: "error" });
    }
  }

  function copy(value: string) {
    setMenu(null);
    void copyText(value);
  }

  return (
    <>
      <button
        className="max-w-full break-all text-left align-baseline font-medium text-accent"
        onClick={() => void open()}
        onContextMenu={(event) => {
          event.preventDefault();
          setMenu({ x: event.clientX, y: event.clientY });
        }}
        type="button"
      >
        {display}
      </button>
      {menu
        ? (
            <div
              className="fixed z-50 min-w-44 overflow-hidden rounded-lg border border-line-soft bg-surface py-1 shadow-panel"
              ref={layerRef}
              style={{ left: menu.x, top: menu.y }}
            >
              <MenuItem label={t("fileLink.copyPath")} onSelect={() => copy(file.path)} />
              {file.insideWorkspace && file.relativePath
                ? <MenuItem label={t("fileLink.copyRelativePath")} onSelect={() => copy(file.relativePath ?? "")} />
                : null}
              <MenuItem label={t("fileLink.copyFilename")} onSelect={() => copy(file.name)} />
              <div className="my-1 border-t border-line-soft" />
              <MenuItem label={t("fileLink.open")} onSelect={() => void open()} />
            </div>
          )
        : null}
    </>
  );
}

function MenuItem({ label, onSelect }: { label: string; onSelect: () => void }) {
  return (
    <button
      className="block w-full px-3 py-1.5 text-left text-sm text-ink hover:bg-surface-subtle"
      onClick={onSelect}
      type="button"
    >
      {label}
    </button>
  );
}
