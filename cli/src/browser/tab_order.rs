//! Cross-protocol tab order reconciliation — port of
//! `cli/src/browser/tab-order.ts`.

use crate::browser::types::PageId;

/// `reconcilePageOrder(storedOrder, currentPageIds)`.
pub fn reconcile_page_order(
    stored_order: Option<&[PageId]>,
    current_page_ids: &[PageId],
) -> Vec<PageId> {
    let stored = match stored_order {
        Some(s) if !s.is_empty() => s,
        _ => {
            // First migration: use whatever order the current protocol gives us.
            return current_page_ids.to_vec();
        }
    };

    let current_set: std::collections::HashSet<&String> = current_page_ids.iter().collect();

    // Keep surviving pages in their stored order
    let existing: Vec<PageId> = stored
        .iter()
        .filter(|id| current_set.contains(id))
        .cloned()
        .collect();

    // Append newly discovered pages at the end
    let discovered: Vec<PageId> = current_page_ids
        .iter()
        .filter(|id| !existing.contains(id))
        .cloned()
        .collect();

    let mut out = existing;
    out.extend(discovered);
    out
}

/// `resolveActivePage(orderedPages, activePageId?)` — defaults to the last page.
pub fn resolve_active_page(
    ordered_pages: &[PageId],
    active_page_id: Option<&str>,
) -> Option<PageId> {
    if let Some(active) = active_page_id {
        if ordered_pages.iter().any(|p| p == active) {
            return Some(active.to_string());
        }
    }
    ordered_pages.last().cloned()
}

/// `insertNewPage(order, newPageId)`.
pub fn insert_new_page(order: &[PageId], new_page_id: &str) -> Vec<PageId> {
    if order.iter().any(|p| p == new_page_id) {
        return order.to_vec();
    }
    let mut out = order.to_vec();
    out.push(new_page_id.to_string());
    out
}

/// `removePage(order, closedPageId)`.
pub fn remove_page(order: &[PageId], closed_page_id: &str) -> Vec<PageId> {
    order
        .iter()
        .filter(|id| id.as_str() != closed_page_id)
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_stored_order_returns_current_order() {
        assert_eq!(
            reconcile_page_order(None, &["a".into(), "b".into(), "c".into()]),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn empty_stored_order_returns_current_order() {
        assert_eq!(
            reconcile_page_order(Some(&[]), &["a".into(), "b".into()]),
            vec!["a", "b"]
        );
    }

    #[test]
    fn surviving_pages_keep_stored_order() {
        // Stored: [a, b, c], Current: [b, a, d] → [a, b, d]
        let stored = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let current = vec!["b".to_string(), "a".to_string(), "d".to_string()];
        assert_eq!(
            reconcile_page_order(Some(&stored), &current),
            vec!["a", "b", "d"]
        );
    }

    #[test]
    fn closed_pages_removed_new_pages_appended() {
        let stored = vec!["a".to_string(), "b".to_string()];
        let current = vec!["b".to_string(), "c".to_string()];
        assert_eq!(
            reconcile_page_order(Some(&stored), &current),
            vec!["b", "c"]
        );
    }

    #[test]
    fn reorder_preserved_for_surviving_pages() {
        // Stored [c, b, a], a and b closed, c remains first
        let stored = vec!["c".to_string(), "b".to_string(), "a".to_string()];
        assert_eq!(
            reconcile_page_order(Some(&stored), &["c".to_string()]),
            vec!["c"]
        );
    }

    #[test]
    fn insert_appends_new_page() {
        assert_eq!(insert_new_page(&["a".to_string()], "b"), vec!["a", "b"]);
    }

    #[test]
    fn insert_noop_if_already_present() {
        assert_eq!(
            insert_new_page(&["a".to_string(), "b".to_string()], "a"),
            vec!["a", "b"]
        );
    }

    #[test]
    fn remove_existing_page() {
        assert_eq!(
            remove_page(&["a".to_string(), "b".to_string(), "c".to_string()], "b"),
            vec!["a", "c"]
        );
    }

    #[test]
    fn remove_noop_if_not_present() {
        assert_eq!(remove_page(&["a".to_string()], "b"), vec!["a"]);
    }

    #[test]
    fn resolve_active_returns_active_page_id_if_present() {
        assert_eq!(
            resolve_active_page(&["a".into(), "b".into(), "c".into()], Some("b")),
            Some("b".to_string())
        );
    }

    #[test]
    fn resolve_active_defaults_to_last_if_not_found() {
        assert_eq!(
            resolve_active_page(&["a".into(), "b".into(), "c".into()], Some("d")),
            Some("c".to_string())
        );
    }

    #[test]
    fn resolve_active_defaults_to_last_if_none() {
        assert_eq!(
            resolve_active_page(&["a".into(), "b".into()], None),
            Some("b".to_string())
        );
    }

    #[test]
    fn resolve_active_undefined_for_empty() {
        assert_eq!(resolve_active_page(&[], None), None);
    }
}
