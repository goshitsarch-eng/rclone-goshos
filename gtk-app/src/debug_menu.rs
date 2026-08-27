//! Global developer context menu — Angular `DebugService` parity.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugMenuItem {
    RefreshUi,
    ClearCache,
    OpenDevTools,
}

impl DebugMenuItem {
    pub fn i18n_key(self) -> &'static str {
        match self {
            Self::RefreshUi => "developerTools.refreshUi",
            Self::ClearCache => "developerTools.clearCache",
            Self::OpenDevTools => "developerTools.openDevTools",
        }
    }

    pub fn fallback(self) -> &'static str {
        match self {
            Self::RefreshUi => "Refresh UI",
            Self::ClearCache => "Clear Cache",
            Self::OpenDevTools => "Open Developer Tools",
        }
    }

    pub fn action_name(self) -> &'static str {
        match self {
            Self::RefreshUi => "win.refresh-ui",
            Self::ClearCache => "win.fscache",
            Self::OpenDevTools => "win.inspector",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DebugMenuTarget {
    /// GTK already owns cut/copy/paste on editable widgets.
    Editable,
    /// A widget that already shows its own context menu (Files listing, etc.).
    CustomMenu,
    /// Empty chrome — show Refresh UI / Clear Cache / DevTools.
    Chrome,
}

pub fn classify_widget_type(type_name: &str) -> DebugMenuTarget {
    let name = type_name.to_ascii_lowercase();
    if is_editable_type(&name) {
        return DebugMenuTarget::Editable;
    }
    if is_custom_menu_type(&name) {
        return DebugMenuTarget::CustomMenu;
    }
    DebugMenuTarget::Chrome
}

pub fn classify_ancestry(type_names: &[&str]) -> DebugMenuTarget {
    for name in type_names {
        match classify_widget_type(name) {
            DebugMenuTarget::Editable => return DebugMenuTarget::Editable,
            DebugMenuTarget::CustomMenu => return DebugMenuTarget::CustomMenu,
            DebugMenuTarget::Chrome => {}
        }
    }
    DebugMenuTarget::Chrome
}

pub fn items_for_target(target: DebugMenuTarget) -> Vec<DebugMenuItem> {
    match target {
        DebugMenuTarget::Editable | DebugMenuTarget::CustomMenu => Vec::new(),
        DebugMenuTarget::Chrome => vec![
            DebugMenuItem::RefreshUi,
            DebugMenuItem::ClearCache,
            DebugMenuItem::OpenDevTools,
        ],
    }
}

pub fn items_for_ancestry(type_names: &[&str]) -> Vec<DebugMenuItem> {
    items_for_target(classify_ancestry(type_names))
}

fn is_editable_type(name: &str) -> bool {
    name.contains("entry")
        || name.contains("textview")
        || name.contains("password")
        || name.contains("spinbutton")
        || name == "gtktext"
        || name.contains("editablelabel")
}

fn is_custom_menu_type(name: &str) -> bool {
    name.contains("popover") || name.contains("menubutton")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_offers_refresh_cache_and_inspector() {
        assert_eq!(
            items_for_target(DebugMenuTarget::Chrome),
            vec![
                DebugMenuItem::RefreshUi,
                DebugMenuItem::ClearCache,
                DebugMenuItem::OpenDevTools,
            ]
        );
    }

    #[test]
    fn editable_and_custom_menus_stay_empty() {
        assert!(items_for_target(DebugMenuTarget::Editable).is_empty());
        assert!(items_for_target(DebugMenuTarget::CustomMenu).is_empty());
    }

    #[test]
    fn classifies_entry_and_password_rows() {
        assert_eq!(
            classify_widget_type("AdwPasswordEntryRow"),
            DebugMenuTarget::Editable
        );
        assert_eq!(classify_widget_type("GtkEntry"), DebugMenuTarget::Editable);
        assert_eq!(classify_widget_type("GtkText"), DebugMenuTarget::Editable);
        assert_eq!(
            classify_widget_type("GtkTextView"),
            DebugMenuTarget::Editable
        );
    }

    #[test]
    fn ancestry_prefers_editable_over_chrome() {
        assert_eq!(
            classify_ancestry(&["GtkLabel", "AdwActionRow", "AdwPreferencesGroup"]),
            DebugMenuTarget::Chrome
        );
        assert_eq!(
            classify_ancestry(&["GtkText", "AdwPasswordEntryRow", "GtkBox"]),
            DebugMenuTarget::Editable
        );
        assert!(items_for_ancestry(&["GtkText", "GtkEntry"]).is_empty());
        assert_eq!(items_for_ancestry(&["AdwHeaderBar"]).len(), 3);
    }

    #[test]
    fn action_names_match_window_actions() {
        assert_eq!(DebugMenuItem::RefreshUi.action_name(), "win.refresh-ui");
        assert_eq!(DebugMenuItem::ClearCache.action_name(), "win.fscache");
        assert_eq!(DebugMenuItem::OpenDevTools.action_name(), "win.inspector");
    }
}
