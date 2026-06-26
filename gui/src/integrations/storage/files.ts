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

export async function inspectAttachment(path: string) {
  return invokeCommand<{ isDir: boolean; size: number; isBinary: boolean }>("inspect_attachment", {
    path,
  });
}

export async function readFileBase64(input: { path: string; maxBytes?: number | null }) {
  return invokeCommand<string>("read_file_base64", {
    maxBytes: input.maxBytes ?? null,
    path: input.path,
  });
}

export async function writeThumbnail(input: { base64Jpeg: string; key: string }) {
  return invokeCommand<string>("write_thumbnail", {
    base64Jpeg: input.base64Jpeg,
    key: input.key,
  });
}

export async function deleteTempAttachment(path: string) {
  return invokeCommand<void>("delete_temp_attachment", { path });
}
