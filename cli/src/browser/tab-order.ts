/**
 * Cross-protocol tab order reconciliation.
 *
 * Both Chromium (Target.getTargets) and Safari (Get Window Handles)
 * return pages in arbitrary order. This module provides a stable
 * ordering that persists across CLI commands.
 */
import type { PageId } from "./types.js";

/**
 * Reconcile a stored page order with the current set of page IDs.
 *
 * Rules:
 * - Stored pages that still exist are kept in their stored order.
 * - Closed pages are removed.
 * - New pages are appended to the end.
 * - No stored order: use the current protocol order (not stable, but best effort).
 */
export function reconcilePageOrder(
  storedOrder: PageId[] | undefined,
  currentPageIds: PageId[],
): PageId[] {
  if (!storedOrder || storedOrder.length === 0) {
    // First migration: use whatever order the current protocol gives us.
    // This is NOT guaranteed stable across browser restarts.
    return [...currentPageIds];
  }

  const currentSet = new Set(currentPageIds);

  // Keep surviving pages in their stored order
  const existing = storedOrder.filter(id => currentSet.has(id));

  // Append newly discovered pages at the end
  const discovered = currentPageIds.filter(id => !existing.includes(id));

  return [...existing, ...discovered];
}

/**
 * Find the active page from the ordered list.
 * Returns the last page if no activePageId is specified (matching current behavior).
 */
export function resolveActivePage(
  orderedPages: PageId[],
  activePageId?: PageId,
): PageId | undefined {
  if (activePageId && orderedPages.includes(activePageId)) {
    return activePageId;
  }
  // Default: last page (matches current Playwright behavior)
  return orderedPages[orderedPages.length - 1];
}

/**
 * Update tab order after creating a new page.
 */
export function insertNewPage(
  order: PageId[],
  newPageId: PageId,
): PageId[] {
  if (order.includes(newPageId)) return order;
  return [...order, newPageId];
}

/**
 * Update tab order after closing a page.
 */
export function removePage(
  order: PageId[],
  closedPageId: PageId,
): PageId[] {
  return order.filter(id => id !== closedPageId);
}
