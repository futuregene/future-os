import { invokeCommand } from "../tauri/invoke";

/** One entry in a directory listing from {@link listDirectory}. */
export interface DirEntry {
  /** Last path component (display name). */
  name: string;
  /** Absolute path to this entry. */
  path: string;
  isDir: boolean;
  /** Byte size for files; 0 for directories. */
  size: number;
  /** Last-modified time as Unix epoch millis, or null if unavailable. */
  modified: number | null;
}

export async function openPath(path: string) {
  return invokeCommand<void>("open_path", { path });
}

/**
 * List a single directory level (no recursion), sorted directories-first then
 * by name. Backs the file-tree panel's lazy per-level loading.
 */
export async function listDirectory(path: string) {
  return invokeCommand<DirEntry[]>("list_directory", { path });
}

/** Open an http(s) / mailto URL in the system default handler (backend restricts scheme). */
export async function openExternalUrl(url: string) {
  return invokeCommand<void>("open_external_url", { url });
}

/**
 * Resolve a markdown link target found while previewing a local file into an
 * absolute path. Relative targets resolve against `baseFile`'s directory; used
 * by the file-preview renderer, which has no workspace root to anchor against.
 */
export async function resolvePreviewLinkPath(baseFile: string, target: string) {
  return invokeCommand<{ path: string; name: string }>("resolve_preview_link_path", { baseFile, target });
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

/** Fully decode a candidate image so unreadable/corrupt files are rejected before send. */
export async function validateImageAttachment(path: string) {
  return invokeCommand<void>("validate_image_attachment", { path });
}

export async function readFileBase64(input: { path: string; maxBytes?: number | null }) {
  return invokeCommand<string>("read_file_base64", {
    maxBytes: input.maxBytes ?? null,
    path: input.path,
  });
}

/**
 * Decode + downscale an image into a thumbnail entirely in Rust and return its
 * persistent path. Avoids shipping the full-size image over the IPC bridge to a
 * webview canvas.
 */
export async function generateImageThumbnail(input: { threadId: string; sourcePath: string }) {
  return invokeCommand<string>("generate_image_thumbnail", {
    sourcePath: input.sourcePath,
    threadId: input.threadId,
  });
}

/**
 * Copy an ephemeral pasted-image original into the thread's persistent image dir
 * (`~/.future/app/images/<threadId>/origin`) and return the durable path.
 */
export async function importEphemeralImage(input: { threadId: string; path: string; name: string }) {
  return invokeCommand<string>("import_ephemeral_image", {
    name: input.name,
    sourcePath: input.path,
    threadId: input.threadId,
  });
}

export async function deleteTempAttachment(path: string) {
  return invokeCommand<void>("delete_temp_attachment", { path });
}
