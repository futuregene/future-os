import { useState } from "react";

/**
 * Per-file collapse state for a review view, keyed by `keyOf(file)`. Files
 * default collapsed; `toggleAll` expands all when any is collapsed, else
 * collapses everything. Shared by the working-tree (keyed by path) and last-run
 * (keyed by id) views.
 */
export function useExpandableFiles<T>(files: T[], keyOf: (file: T) => string) {
  const [openFiles, setOpenFiles] = useState<Record<string, boolean>>({});
  const allOpen = files.length > 0 && files.every(file => openFiles[keyOf(file)]);

  const isOpen = (file: T) => openFiles[keyOf(file)] ?? false;

  const toggle = (file: T) => setOpenFiles((current) => {
    const key = keyOf(file);
    return { ...current, [key]: !(current[key] ?? false) };
  });

  const toggleAll = () => setOpenFiles(
    allOpen ? {} : Object.fromEntries(files.map(file => [keyOf(file), true])),
  );

  return { allOpen, isOpen, toggle, toggleAll };
}
