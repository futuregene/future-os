import { invoke } from "@tauri-apps/api/core";

export async function openPath(path: string) {
  return invoke<void>("open_path", { path });
}

export async function readTextFilePreview(input: {
  path: string;
  maxBytes?: number | null;
}) {
  return invoke<{ content: string; size: number; truncated: boolean }>("read_text_file_preview", {
    maxBytes: input.maxBytes ?? null,
    path: input.path,
  });
}

export async function exportArtifactFile(input: {
  destinationPath: string;
  sourcePath?: string | null;
  content?: string | null;
}) {
  return invoke<void>("export_artifact_file", {
    content: input.content ?? null,
    destinationPath: input.destinationPath,
    sourcePath: input.sourcePath ?? null,
  });
}

export async function savePastedImage(input: { bytes: number[]; extension: string }) {
  return invoke<{ name: string; path: string }>("save_pasted_image", input);
}
