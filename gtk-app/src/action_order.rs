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

/// Sync/copy/move/bisync plus check/delete/copyurl/archive/cryptcheck.
pub fn sync_catalog_ids() -> Vec<&'static str> {
    OperationType::PRIMARY_SYNC
        .iter()
        .chain(OperationType::MORE_SYNC.iter())
        .map(|op| op.as_str())
        .collect()
}

pub fn can_show_more(items: &[OrderItem], max_visible: Option<usize>) -> bool {
    match max_visible {
        Some(max) => items.iter().filter(|item| item.visible).count() < max,
        None => true,
    }
}

/// Hide overflow items so at most `max_visible` stay starred, preserving order.
pub fn cap_visible(items: &mut [OrderItem], max_visible: Option<usize>) {
    let Some(max) = max_visible else {
        return;
    };
    let mut seen = 0usize;
    for item in items.iter_mut() {
        if !item.visible {
            continue;
        }
        seen += 1;
        if seen > max {
            item.visible = false;
        }
    }
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

/// Angular `moveItemInArray`: move `from` to `to`, shifting neighbors.
pub fn move_index<T>(items: &mut [T], from: usize, to: usize) -> bool {
    if from >= items.len() || to >= items.len() || from == to {
        return false;
    }
    if from < to {
        items[from..=to].rotate_left(1);
    } else {
        items[to..=from].rotate_right(1);
    }
    true
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
    fn move_index_matches_angular_drag() {
        let mut ids = vec!["a", "b", "c", "d"];
        assert!(move_index(&mut ids, 0, 2));
        assert_eq!(ids, ["b", "c", "a", "d"]);
        assert!(move_index(&mut ids, 3, 0));
        assert_eq!(ids, ["d", "b", "c", "a"]);
        assert!(!move_index(&mut ids, 1, 1));
        assert!(!move_index(&mut ids, 9, 0));
        let mut items = build_items(&["copy".into(), "sync".into()], &["copy", "sync", "move"]);
        assert!(move_index(&mut items, 2, 0));
        assert_eq!(items[0].id, "move");
        assert_eq!(items[1].id, "copy");
    }

    #[test]
    fn catalog_covers_all_operations() {
        assert_eq!(catalog_ids().len(), OperationType::ALL.len());
        assert!(catalog_ids().contains(&"serve"));
    }

    #[test]
    fn sync_catalog_omits_mount_and_serve() {
        let ids = sync_catalog_ids();
        assert!(!ids.contains(&"mount"));
        assert!(!ids.contains(&"serve"));
        assert!(ids.contains(&"sync"));
        assert!(ids.contains(&"cryptcheck"));
        assert_eq!(ids.len(), 9);
    }

    #[test]
    fn caps_visible_to_max_and_blocks_more() {
        let mut items = build_items(
            &["sync".into(), "copy".into(), "move".into(), "bisync".into()],
            &["sync", "copy", "move", "bisync"],
        );
        assert!(!can_show_more(&items, Some(3)));
        assert!(can_show_more(&items, None));
        cap_visible(&mut items, Some(3));
        assert_eq!(visible_ids(&items), vec!["sync", "copy", "move"]);
        assert!(can_show_more(&items, Some(4)));
        cap_visible(&mut items, None);
        assert_eq!(visible_ids(&items), vec!["sync", "copy", "move"]);
    }
}
