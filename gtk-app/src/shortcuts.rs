//! Keyboard-shortcut search helpers shared by the GTK shortcuts dialog.

/// True when `query` matches a shortcut row's title, keys, or extra haystack.
pub fn shortcut_matches(query: &str, title: &str, keys: &str, extra: &str) -> bool {
    let query = query.trim();
    if query.is_empty() {
        return true;
    }
    let hay = format!("{title} {keys} {extra}").to_lowercase();
    hay.contains(&query.to_lowercase())
}

/// A category header stays visible when any of its item rows match.
pub fn shortcut_category_visible(query: &str, items: &[(&str, &str, &str)]) -> bool {
    items
        .iter()
        .any(|(keys, title, extra)| shortcut_matches(query, title, keys, extra))
}

/// True when a non-empty search hid every item row.
pub fn shortcut_search_empty(query: &str, visible_items: usize) -> bool {
    !query.trim().is_empty() && visible_items == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_query_matches_everything() {
        assert!(shortcut_matches("", "Quit Application", "Ctrl+Q", "quit"));
        assert!(shortcut_matches("   ", "Preferences", "Ctrl+,", ""));
        assert!(shortcut_category_visible("", &[("Ctrl+Q", "Quit", "quit")]));
        assert!(!shortcut_search_empty("", 0));
        assert!(!shortcut_search_empty("   ", 0));
    }

    #[test]
    fn matches_title_keys_and_extra_case_insensitively() {
        assert!(shortcut_matches("quit", "Quit Application", "Ctrl+Q", ""));
        assert!(shortcut_matches("CTRL+Q", "Quit Application", "Ctrl+Q", ""));
        assert!(shortcut_matches(
            "force check",
            "Force Check Mounts",
            "Ctrl+Shift+M",
            ""
        ));
        assert!(shortcut_matches(
            "paste",
            "Paste",
            "Ctrl+V",
            "nautilus.contextMenu.paste"
        ));
        assert!(!shortcut_matches(
            "xyzzy",
            "Quit Application",
            "Ctrl+Q",
            "quit"
        ));
    }

    #[test]
    fn category_and_empty_state() {
        let items = [
            ("Ctrl+Q", "Quit Application", "quit"),
            ("Ctrl+,", "Open Preferences", "prefs"),
        ];
        assert!(shortcut_category_visible("quit", &items));
        assert!(!shortcut_category_visible("xyzzy", &items));
        assert!(shortcut_search_empty("xyzzy", 0));
        assert!(!shortcut_search_empty("quit", 1));
    }
}
