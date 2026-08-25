//! Preferences search — Angular `preferences-modal` title + help matching.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrefSearchItem {
    pub page: &'static str,
    pub title_key: &'static str,
    pub title_fallback: &'static str,
    pub help_key: &'static str,
    pub help_fallback: &'static str,
    pub extra: &'static [&'static str],
}

pub const SUGGESTIONS: &[(&str, &str, &str)] = &[
    (
        "apiPort",
        "modals.preferences.searchSuggestions.apiPort",
        "API port",
    ),
    (
        "startup",
        "modals.preferences.searchSuggestions.startup",
        "startup",
    ),
    (
        "debug",
        "modals.preferences.searchSuggestions.debug",
        "debug",
    ),
    (
        "bandwidth",
        "modals.preferences.searchSuggestions.bandwidth",
        "bandwidth",
    ),
];

pub const ITEMS: &[PrefSearchItem] = &[
    PrefSearchItem {
        page: "general",
        title_key: "settings.general.language.label",
        title_fallback: "Language",
        help_key: "settings.general.language.description",
        help_fallback: "Language to use for the user interface.",
        extra: &["language", "locale"],
    },
    PrefSearchItem {
        page: "general",
        title_key: "settings.general.default_view.label",
        title_fallback: "Default Startup View",
        help_key: "settings.general.default_view.description",
        help_fallback: "Select the main interface view to launch when starting the app.",
        extra: &["startup", "view"],
    },
    PrefSearchItem {
        page: "general",
        title_key: "settings.general.tray_enabled.label",
        title_fallback: "Enable Tray Icon",
        help_key: "settings.general.tray_enabled.description",
        help_fallback: "Show an icon in the system tray.",
        extra: &["tray"],
    },
    PrefSearchItem {
        page: "general",
        title_key: "settings.general.start_on_startup.label",
        title_fallback: "Start on Startup",
        help_key: "settings.general.start_on_startup.description",
        help_fallback: "Automatically start the app when the system starts.",
        extra: &["startup"],
    },
    PrefSearchItem {
        page: "general",
        title_key: "settings.general.notifications.label",
        title_fallback: "Enable Notifications",
        help_key: "settings.general.notifications.description",
        help_fallback: "Show desktop notifications for application events.",
        extra: &["notify"],
    },
    PrefSearchItem {
        page: "core",
        title_key: "settings.core.rclone_binary.label",
        title_fallback: "Rclone binary",
        help_key: "settings.core.rclone_binary.description",
        help_fallback: "Path to the rclone executable.",
        extra: &["binary", "rclone"],
    },
    PrefSearchItem {
        page: "core",
        title_key: "settings.core.bandwidth_limit.label",
        title_fallback: "Bandwidth limit",
        help_key: "settings.core.bandwidth_limit.description",
        help_fallback: "Limit rclone transfer speed.",
        extra: &["bandwidth", "speed"],
    },
    PrefSearchItem {
        page: "core",
        title_key: "settings.core.metered_bandwidth_limit.label",
        title_fallback: "Bandwidth limit on metered networks",
        help_key: "settings.core.metered_bandwidth_limit.description",
        help_fallback: "Limit rclone transfer speed on metered connections.",
        extra: &["bandwidth", "metered"],
    },
    PrefSearchItem {
        page: "core",
        title_key: "settings.core.connection_check_urls.label",
        title_fallback: "Connectivity check URLs",
        help_key: "settings.core.connection_check_urls.description",
        help_fallback: "URLs used to test network connectivity.",
        extra: &["apiPort", "url", "port", "connection"],
    },
    PrefSearchItem {
        page: "core",
        title_key: "settings.core.rclone_flags.label",
        title_fallback: "Additional rclone flags",
        help_key: "settings.core.rclone_flags.description",
        help_fallback: "Extra flags passed to the rclone engine.",
        extra: &["flags", "apiPort", "port"],
    },
    PrefSearchItem {
        page: "developer",
        title_key: "settings.developer.log_level.label",
        title_fallback: "Log level",
        help_key: "settings.developer.log_level.description",
        help_fallback: "Verbosity of application logs.",
        extra: &["debug", "trace", "log"],
    },
    PrefSearchItem {
        page: "security",
        title_key: "modals.backend.security.encrypted",
        title_fallback: "Security",
        help_key: "settings.core.config_password.description",
        help_fallback: "Protect the rclone config with a password.",
        extra: &["password", "keyring"],
    },
];

pub fn help_key_from_label(label_key: &str) -> Option<String> {
    let stem = label_key.strip_suffix(".label").unwrap_or(label_key);
    if stem.is_empty() {
        return None;
    }
    Some(format!("{stem}.description"))
}

pub fn item_haystack(item: &PrefSearchItem, i18n: &crate::i18n::I18n) -> String {
    let title = i18n.t_or(item.title_key, item.title_fallback);
    let help = i18n.t_or(item.help_key, item.help_fallback);
    let extra = item.extra.join(" ");
    format!("{title} {help} {extra}").to_lowercase()
}

/// True when `query` is empty or any field contains it (case-insensitive).
pub fn any_field_matches(fields: &[&str], query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    fields
        .iter()
        .any(|field| field.to_ascii_lowercase().contains(&query))
}

pub fn item_matches(item: &PrefSearchItem, query: &str, i18n: &crate::i18n::I18n) -> bool {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return false;
    }
    item_haystack(item, i18n).contains(&query)
}

pub fn matching_items<'a>(query: &str, i18n: &crate::i18n::I18n) -> Vec<&'a PrefSearchItem> {
    ITEMS
        .iter()
        .filter(|item| item_matches(item, query, i18n))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_title_help_and_keywords() {
        let i18n = crate::i18n::I18n::default();
        assert!(matching_items("startup", &i18n)
            .iter()
            .any(|item| item.page == "general"));
        assert!(matching_items("bandwidth", &i18n)
            .iter()
            .any(|item| item.title_key.contains("bandwidth")));
        assert!(matching_items("debug", &i18n)
            .iter()
            .any(|item| item.page == "developer"));
        assert!(matching_items("system tray", &i18n)
            .iter()
            .any(|item| item.title_key.contains("tray_enabled")));
        assert!(matching_items("   ", &i18n).is_empty());
        assert!(matching_items("no-such-setting-xyz", &i18n).is_empty());
        assert_eq!(
            help_key_from_label("settings.core.bandwidth_limit.label").as_deref(),
            Some("settings.core.bandwidth_limit.description")
        );
        assert_eq!(SUGGESTIONS.len(), 4);
        assert!(any_field_matches(&["Mount", "mount"], ""));
        assert!(any_field_matches(&["Mount", "mount"], "mou"));
        assert!(any_field_matches(&["VFS", "vfs"], "vfs"));
        assert!(!any_field_matches(&["Remote", "remote"], "sync"));
    }
}
