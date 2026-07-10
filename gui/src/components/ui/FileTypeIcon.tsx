import type { LucideIcon } from "lucide-react";
import type { FileKind } from "../../lib/fileType";
import { BookText, FileArchive, FileBraces, FileCode, FileDown, FileImage, FileTerminal, FileText, Folder } from "lucide-react";

/** kind → lucide glyph. The one place a {@link FileKind} becomes an icon. */
const ICONS: Record<FileKind, LucideIcon> = {
  folder: Folder,
  image: FileImage,
  pdf: BookText,
  markdown: FileDown,
  html: FileCode,
  archive: FileArchive,
  shell: FileTerminal,
  code: FileBraces,
  text: FileText,
};

export function FileTypeIcon({ kind, className }: { kind: FileKind; className?: string }) {
  const Icon = ICONS[kind];
  return <Icon className={className} />;
}
