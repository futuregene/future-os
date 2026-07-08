import type { StoredFile } from "../../../integrations/storage/types";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { openPath } from "../../../integrations/storage/files";
import { copyText } from "../../../lib/clipboard";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { FilePreviewOverlay } from "../../filepreview/FilePreviewOverlay";
import { previewKindForPath } from "../../filepreview/previewKind";
import { LinkContextMenu } from "./LinkContextMenu";
import { useLinkContextMenu } from "./useLinkContextMenu";

/**
 * A local-file link (from a plain markdown path link) rendered inline as an
 * anchor. For previewable types (image / PDF / markdown) left click opens a
 * fullscreen in-app preview; every other type opens with the OS default handler
 * (directories land in the file manager). Right click opens a context menu with
 * a preview action (when applicable) plus copy / open actions. The `file://`
 * href is only a hover/drag affordance — navigation is prevented so the click
 * always routes through our handlers. In-workspace files show their
 * workspace-relative path, files written elsewhere (e.g. `~/Desktop`) show the
 * full path.
 */
export function FileLink({ file }: { file: StoredFile }) {
  const { t } = useTranslation("markdown");
  const menu = useLinkContextMenu();
  const [previewing, setPreviewing] = useState(false);

  const display = file.insideWorkspace && file.relativePath ? file.relativePath : file.path;
  const previewKind = previewKindForPath(file.path);

  async function open() {
    try {
      await openPath(file.path);
    }
    catch {
      emitFutureEvent("toast", { message: t("fileLink.notFound", { name: file.name }), tone: "error" });
    }
  }

  function activate() {
    if (previewKind)
      setPreviewing(true);
    else
      void open();
  }

  const items = [
    ...(previewKind
      ? [{ label: t("fileLink.preview"), onSelect: () => setPreviewing(true) }]
      : []),
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
          activate();
        }}
        onContextMenu={menu.open}
        title={file.path}
      >
        {display}
      </a>
      <LinkContextMenu controller={menu} items={items} />
      {previewKind
        ? (
            <FilePreviewOverlay
              kind={previewKind}
              name={file.name}
              onClose={() => setPreviewing(false)}
              onOpenExternal={() => void open()}
              open={previewing}
              path={file.path}
            />
          )
        : null}
    </>
  );
}
