import type { StoredThread } from "../../../integrations/storage/threadStore";
import { useEffect, useRef, useState } from "react";
import { searchThreads } from "../../../integrations/storage/threadStore";

/**
 * One keystroke's worth of wait before asking the store.
 *
 * The rail filters what it already holds as the reader types (that is the
 * client-side pass), so the round trip only has to answer for rows the loaded
 * pages do not hold. Debouncing keeps a typed title from costing a query per
 * character.
 */
const DEBOUNCE_MS = 200;

/**
 * The store's own cap is 200 a half; asking for it is what makes the answer
 * "the whole store" rather than "the first page of it".
 */
const SEARCH_LIMIT = 200;

export interface ThreadSearchState {
  /** The trimmed query these rows answer. */
  query: string;
  /** Conversations the store matched and still counts as active. */
  active: StoredThread[];
  /** Conversations the store matched and counts as shelved. */
  archived: StoredThread[];
}

/**
 * The rail's search, answered by the store instead of by the loaded pages.
 *
 * The lists are paged: the tree holds the pages its groups and the recent view
 * asked for, and the archived view holds its own two streams. Filtering those
 * arrays meant a reader could only find a conversation whose page happened to
 * be loaded 鈥?a folded workspace, an unexpanded shelf or anything older than
 * the first page was invisible to the search box, which reads as "this
 * conversation does not exist".
 *
 * This asks `search_threads` (title or workspace name, over the whole store)
 * and hands the answer back split the way the rail draws it 鈥?active lists and
 * shelf 鈥?so each view can draw real hits next to the ones it already has.
 *
 * `null` means "no server answer for what is on screen": no query, or a query
 * whose round trip has not landed yet. The caller falls back to its own filter
 * meanwhile, so typing narrows the list immediately and widens again when the
 * full answer arrives.
 */
export function useThreadSearch(query: string): ThreadSearchState | null {
  const trimmed = query.trim();
  const [state, setState] = useState<ThreadSearchState | null>(null);
  // Every request carries the number it was issued with; an answer that is not
  // the latest is dropped, so a slow first query cannot overwrite a fast second.
  const sequenceRef = useRef(0);

  useEffect(() => {
    if (trimmed.length === 0) {
      sequenceRef.current += 1;
      setState(null);
      return;
    }
    // A previous answer belongs to the previous query; keeping it on screen
    // would draw rows the reader did not ask for.
    setState(current => (current && current.query === trimmed ? current : null));
    const issued = ++sequenceRef.current;
    const timer = window.setTimeout(() => {
      void searchThreads(trimmed, SEARCH_LIMIT)
        .then((results) => {
          if (sequenceRef.current !== issued)
            return;
          setState({ active: results.active, archived: results.archived, query: trimmed });
        })
        .catch(() => {
          // A search that cannot be answered leaves the client-side pass in
          // charge rather than emptying the list: the reader still sees the
          // matches among what is loaded.
          if (sequenceRef.current === issued)
            setState(null);
        });
    }, DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [trimmed]);

  // Read through the current query: a state kept from the previous one is not
  // this query's answer even before the effect below clears it.
  return state && state.query === trimmed ? state : null;
}
