import type { StoredFile } from "../../../integrations/storage/types";
import { useTranslation } from "react-i18next";
import { openPath } from "../../../integrations/storage/files";
import { copyText } from "../../../lib/clipboard";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { LinkContextMenu } from "./LinkContextMenu";
import { useLinkContextMenu } from "./useLinkContextMenu";

/**
 * A local-file link (from a plain markdown path link) rendered inline as an
 * anchor. Left click opens the file with the OS default handler (directories
 * land in the file manager); right click opens a context menu with copy / open
 * actions. The `file://` href is only a hover/drag affordance — navigation is
 * prevented so the click always routes through `openPath`. In-workspace files
 * show their workspace-relative path, files written elsewhere (e.g. `~/Desktop`)
 * show the full path.
 */
export function FileLink({ file }: { file: StoredFile }) {
  const { t } = useTranslation("markdown");
  const menu = useLinkContextMenu();

  const display = file.insideWorkspace && file.relativePath ? file.relativePath : file.path;

  async function open() {
    try {
      await openPath(file.path);
    }
    catch {
      emitFutureEvent("toast", { message: t("fileLink.notFound", { name: file.name }), tone: "error" });
    }
  }

  const items = [
    { label: t("fileLink.copyPath"), onSelect: () => void copyText(file.path) },
    ...(file.insideWorkspace && file.relativePath
      ? [{ label: t("fileLink.copyRelativePath"), onSelect: () => void copyText(file.relativePath ?? "") }]
      : []),
    { label: t("fileLink.copyFilename"), onSelect: () => void copyText(file.name) },
    { divider: true, label: t("fileLink.open"), onSelect: () => void open() },
  ];

  return (
    <>
      <a
        className="max-w-full break-all align-baseline font-medium text-accent underline-offset-2 hover:underline"
        href={`file://${file.path}`}
        onClick={(event) => {
          event.preventDefault();
          void open();
        }}
        onContextMenu={menu.open}
        title={file.path}
      >
        {display}
      </a>
      <LinkContextMenu controller={menu} items={items} />
    </>
  );
}
