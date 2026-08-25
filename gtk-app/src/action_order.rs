//! Action-order / visibility lists — same contract as Angular
//! `ItemOrderVisibilityModal` + `buildActionOrderItems`.

use crate::operations::OperationType;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderItem {
    pub id: String,
    pub visible: bool,
}

pub fn catalog_ids() -> Vec<&'static str> {
    OperationType::ALL.iter().map(|op| op.as_str()).collect()
}

/// Build an editor list: current visible ids first (preserving order), then
/// the remaining catalog entries hidden. An empty `current` means “show all
/// in catalog order”, matching `RemoteMeta::visible_operations`.
pub fn build_items(current: &[String], catalog: &[&str]) -> Vec<OrderItem> {
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    if current.is_empty() {
        for id in catalog {
            items.push(OrderItem {
                id: (*id).to_string(),
                visible: true,
            });
            seen.insert((*id).to_string());
        }
        return items;
    }
    for id in current {
        let key = id.trim();
        if key.is_empty() || !catalog.iter().any(|c| *c == key) || !seen.insert(key.to_string()) {
            continue;
        }
        items.push(OrderItem {
            id: key.to_string(),
            visible: true,
        });
    }
    for id in catalog {
        if seen.insert((*id).to_string()) {
            items.push(OrderItem {
                id: (*id).to_string(),
                visible: false,
            });
        }
    }
    items
}

pub fn move_item(items: &mut [OrderItem], index: usize, delta: isize) -> usize {
    let next = index as isize + delta;
    if index >= items.len() || next < 0 || next >= items.len() as isize {
        return index;
    }
    items.swap(index, next as usize);
    next as usize
}

pub fn visible_ids(items: &[OrderItem]) -> Vec<String> {
    items
        .iter()
        .filter(|item| item.visible)
        .map(|item| item.id.clone())
        .collect()
}

pub fn apply_visibility(items: &mut [OrderItem], id: &str, visible: bool) {
    if let Some(item) = items.iter_mut().find(|item| item.id == id) {
        item.visible = visible;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_current_shows_full_catalog() {
        let items = build_items(&[], &["sync", "copy", "mount"]);
        assert_eq!(items.len(), 3);
        assert!(items.iter().all(|i| i.visible));
        assert_eq!(visible_ids(&items), vec!["sync", "copy", "mount"]);
    }

    #[test]
    fn preserves_visible_order_and_appends_hidden() {
        let items = build_items(
            &["copy".into(), "sync".into()],
            &["mount", "sync", "copy", "serve"],
        );
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            vec!["copy", "sync", "mount", "serve"]
        );
        assert_eq!(visible_ids(&items), vec!["copy", "sync"]);
    }

    #[test]
    fn ignores_unknown_and_duplicate_ids() {
        let items = build_items(
            &["nope".into(), "copy".into(), "copy".into(), "  ".into()],
            &["copy", "sync"],
        );
        assert_eq!(visible_ids(&items), vec!["copy"]);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn moves_and_toggles() {
        let mut items = build_items(&["a".into(), "b".into()], &["a", "b", "c"]);
        assert_eq!(move_item(&mut items, 0, -1), 0);
        assert_eq!(move_item(&mut items, 0, 1), 1);
        assert_eq!(items[0].id, "b");
        assert_eq!(items[1].id, "a");
        apply_visibility(&mut items, "a", false);
        assert_eq!(visible_ids(&items), vec!["b"]);
        assert_eq!(move_item(&mut items, 2, 1), 2);
    }

    #[test]
    fn catalog_covers_all_operations() {
        assert_eq!(catalog_ids().len(), OperationType::ALL.len());
        assert!(catalog_ids().contains(&"serve"));
    }
}
