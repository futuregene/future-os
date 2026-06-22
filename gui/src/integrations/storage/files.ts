import { invokeCommand } from "../tauri/invoke";

export async function openPath(path: string) {
  return invokeCommand<void>("open_path", { path });
}

export async function readTextFilePreview(input: {
  path: string;
  maxBytes?: number | null;
}) {
  return invokeCommand<{ content: string; size: number; truncated: boolean }>("read_text_file_preview", {
    maxBytes: input.maxBytes ?? null,
    path: input.path,
  });
}

export async function exportArtifactFile(input: {
  destinationPath: string;
  sourcePath?: string | null;
  content?: string | null;
}) {
  return invokeCommand<void>("export_artifact_file", {
    content: input.content ?? null,
    destinationPath: input.destinationPath,
    sourcePath: input.sourcePath ?? null,
  });
}

export async function savePastedImage(input: { bytes: number[]; extension: string }) {
  return invokeCommand<{ name: string; path: string }>("save_pasted_image", {
    bytes: input.bytes,
    extension: input.extension,
  });
}
